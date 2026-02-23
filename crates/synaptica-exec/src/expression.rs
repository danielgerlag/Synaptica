use crate::engine::ExecError;
use crate::result::Record;
use synaptica_core::types::Value;
use synaptica_gql::ast::{AggregateFunction, BinaryOp, Expression, Literal, UnaryOp};

/// Evaluate an expression against a record context.
pub fn evaluate(expr: &Expression, context: &Record) -> Result<Value, ExecError> {
    match expr {
        Expression::Literal(lit) => eval_literal(lit),
        Expression::Identifier(name) => context
            .get(name)
            .cloned()
            .ok_or_else(|| ExecError::ExpressionError(format!("unknown identifier: {}", name))),
        Expression::PropertyAccess { object, property } => {
            // For node-variable property access (e.g. n.name), try column
            // lookups first so that an unresolvable variable does not short-
            // circuit the fallback paths.
            if let Expression::Identifier(var) = object.as_ref() {
                let key = format!("{}.{}", var, property);
                if let Some(v) = context.get(&key) {
                    return Ok(v.clone());
                }
                if let Some(v) = context.get(property) {
                    return Ok(v.clone());
                }
            }
            let obj = evaluate(object, context)?;
            match obj {
                Value::Map(map) => Ok(map.get(property).cloned().unwrap_or(Value::Null)),
                _ => Ok(Value::Null),
            }
        }
        Expression::BinaryOp { left, op, right } => {
            let lv = evaluate(left, context)?;
            // Short-circuit for AND/OR
            match op {
                BinaryOp::And => {
                    if lv == Value::Bool(false) {
                        return Ok(Value::Bool(false));
                    }
                    let rv = evaluate(right, context)?;
                    return eval_binary_op(&lv, op, &rv);
                }
                BinaryOp::Or => {
                    if lv == Value::Bool(true) {
                        return Ok(Value::Bool(true));
                    }
                    let rv = evaluate(right, context)?;
                    return eval_binary_op(&lv, op, &rv);
                }
                _ => {}
            }
            let rv = evaluate(right, context)?;
            eval_binary_op(&lv, op, &rv)
        }
        Expression::UnaryOp { op, operand } => {
            let v = evaluate(operand, context)?;
            eval_unary_op(op, &v)
        }
        Expression::IsNull(inner) => {
            let v = evaluate(inner, context)?;
            Ok(Value::Bool(v.is_null()))
        }
        Expression::IsNotNull(inner) => {
            let v = evaluate(inner, context)?;
            Ok(Value::Bool(!v.is_null()))
        }
        Expression::In { operand, list } => {
            let val = evaluate(operand, context)?;
            let list_val = evaluate(list, context)?;
            match list_val {
                Value::List(items) => Ok(Value::Bool(items.contains(&val))),
                _ => Err(ExecError::TypeError("IN requires a list".into())),
            }
        }
        Expression::FunctionCall { name, args } => {
            let evaluated_args: Vec<Value> = args
                .iter()
                .map(|a| evaluate(a, context))
                .collect::<Result<_, _>>()?;
            eval_function(name, &evaluated_args)
        }
        Expression::Aggregate {
            function,
            distinct: _,
            arg,
        } => {
            // Single-record evaluation of aggregates: just return the value
            match arg {
                Some(inner) => evaluate(inner, context),
                None => match function {
                    AggregateFunction::Count => Ok(Value::Integer(1)),
                    _ => Ok(Value::Null),
                },
            }
        }
        Expression::List(exprs) => {
            let values: Vec<Value> = exprs
                .iter()
                .map(|e| evaluate(e, context))
                .collect::<Result<_, _>>()?;
            Ok(Value::List(values))
        }
        Expression::Map(entries) => {
            let mut map = std::collections::BTreeMap::new();
            for (key, expr) in entries {
                map.insert(key.clone(), evaluate(expr, context)?);
            }
            Ok(Value::Map(map))
        }
        Expression::Case {
            operand,
            when_clauses,
            else_clause,
        } => {
            if let Some(op) = operand {
                let op_val = evaluate(op, context)?;
                for (when_expr, then_expr) in when_clauses {
                    let w = evaluate(when_expr, context)?;
                    if op_val == w {
                        return evaluate(then_expr, context);
                    }
                }
            } else {
                for (when_expr, then_expr) in when_clauses {
                    let w = evaluate(when_expr, context)?;
                    if w == Value::Bool(true) {
                        return evaluate(then_expr, context);
                    }
                }
            }
            match else_clause {
                Some(e) => evaluate(e, context),
                None => Ok(Value::Null),
            }
        }
        _ => Err(ExecError::NotImplemented(format!(
            "expression: {:?}",
            std::mem::discriminant(expr)
        ))),
    }
}

fn eval_literal(lit: &Literal) -> Result<Value, ExecError> {
    Ok(match lit {
        Literal::Integer(i) => Value::Integer(*i),
        Literal::Float(f) => Value::Float(*f),
        Literal::String(s) => Value::String(s.clone()),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Null => Value::Null,
        Literal::List(items) => {
            let vs: Vec<Value> = items.iter().map(eval_literal).collect::<Result<_, _>>()?;
            Value::List(vs)
        }
        Literal::Map(entries) => {
            let mut map = std::collections::BTreeMap::new();
            for (k, v) in entries {
                map.insert(k.clone(), eval_literal(v)?);
            }
            Value::Map(map)
        }
        _ => {
            return Err(ExecError::NotImplemented(format!(
                "literal: {:?}",
                std::mem::discriminant(lit)
            )))
        }
    })
}

fn eval_binary_op(lv: &Value, op: &BinaryOp, rv: &Value) -> Result<Value, ExecError> {
    match op {
        BinaryOp::Eq => Ok(Value::Bool(lv == rv)),
        BinaryOp::Neq => Ok(Value::Bool(lv != rv)),

        BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
            eval_comparison(lv, op, rv)
        }

        BinaryOp::Add => eval_arithmetic(lv, rv, |a, b| a + b, |a, b| a + b),
        BinaryOp::Sub => eval_arithmetic(lv, rv, |a, b| a - b, |a, b| a - b),
        BinaryOp::Mul => eval_arithmetic(lv, rv, |a, b| a * b, |a, b| a * b),
        BinaryOp::Div => {
            // Check for division by zero
            match (lv, rv) {
                (Value::Integer(_), Value::Integer(0))
                | (Value::Float(_), Value::Integer(0)) => {
                    return Err(ExecError::ExpressionError("division by zero".into()));
                }
                _ => {}
            }
            eval_arithmetic(lv, rv, |a, b| a / b, |a, b| a / b)
        }
        BinaryOp::Mod => eval_arithmetic(lv, rv, |a, b| a % b, |a, b| a % b),

        BinaryOp::And => match (lv, rv) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a && *b)),
            _ => Ok(Value::Null),
        },
        BinaryOp::Or => match (lv, rv) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a || *b)),
            _ => Ok(Value::Null),
        },
        BinaryOp::Xor => match (lv, rv) {
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(*a ^ *b)),
            _ => Ok(Value::Null),
        },
        BinaryOp::Concat => match (lv, rv) {
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
            (Value::List(a), Value::List(b)) => {
                let mut out = a.clone();
                out.extend(b.iter().cloned());
                Ok(Value::List(out))
            }
            _ => Err(ExecError::TypeError(format!(
                "cannot concatenate {} and {}",
                lv.type_name(),
                rv.type_name()
            ))),
        },
    }
}

fn eval_comparison(lv: &Value, op: &BinaryOp, rv: &Value) -> Result<Value, ExecError> {
    let ord = match (lv, rv) {
        (Value::Integer(a), Value::Integer(b)) => a.partial_cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
        (Value::Integer(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
        (Value::Float(a), Value::Integer(b)) => a.partial_cmp(&(*b as f64)),
        (Value::String(a), Value::String(b)) => a.partial_cmp(b),
        (Value::Null, _) | (_, Value::Null) => return Ok(Value::Null),
        _ => {
            return Err(ExecError::TypeError(format!(
                "cannot compare {} and {}",
                lv.type_name(),
                rv.type_name()
            )))
        }
    };
    let ord = match ord {
        Some(o) => o,
        None => return Ok(Value::Null),
    };
    let result = match op {
        BinaryOp::Lt => ord == std::cmp::Ordering::Less,
        BinaryOp::Gt => ord == std::cmp::Ordering::Greater,
        BinaryOp::Le => ord != std::cmp::Ordering::Greater,
        BinaryOp::Ge => ord != std::cmp::Ordering::Less,
        _ => unreachable!(),
    };
    Ok(Value::Bool(result))
}

fn eval_arithmetic(
    lv: &Value,
    rv: &Value,
    int_op: fn(i64, i64) -> i64,
    float_op: fn(f64, f64) -> f64,
) -> Result<Value, ExecError> {
    match (lv, rv) {
        (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(int_op(*a, *b))),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_op(*a, *b))),
        (Value::Integer(a), Value::Float(b)) => Ok(Value::Float(float_op(*a as f64, *b))),
        (Value::Float(a), Value::Integer(b)) => Ok(Value::Float(float_op(*a, *b as f64))),
        _ => Err(ExecError::TypeError(format!(
            "cannot perform arithmetic on {} and {}",
            lv.type_name(),
            rv.type_name()
        ))),
    }
}

fn eval_unary_op(op: &UnaryOp, v: &Value) -> Result<Value, ExecError> {
    match op {
        UnaryOp::Not => match v {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            Value::Null => Ok(Value::Null),
            _ => Err(ExecError::TypeError(format!(
                "NOT requires boolean, got {}",
                v.type_name()
            ))),
        },
        UnaryOp::Neg => match v {
            Value::Integer(i) => Ok(Value::Integer(-i)),
            Value::Float(f) => Ok(Value::Float(-f)),
            Value::Null => Ok(Value::Null),
            _ => Err(ExecError::TypeError(format!(
                "cannot negate {}",
                v.type_name()
            ))),
        },
        UnaryOp::Pos => match v {
            Value::Integer(_) | Value::Float(_) => Ok(v.clone()),
            Value::Null => Ok(Value::Null),
            _ => Err(ExecError::TypeError(format!(
                "cannot apply unary + to {}",
                v.type_name()
            ))),
        },
    }
}

fn eval_function(name: &str, args: &[Value]) -> Result<Value, ExecError> {
    match name.to_lowercase().as_str() {
        "tostring" => {
            let v = args.first().unwrap_or(&Value::Null);
            Ok(Value::String(format!("{}", v)))
        }
        "tointeger" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::Integer(i) => Ok(Value::Integer(*i)),
                Value::Float(f) => Ok(Value::Integer(*f as i64)),
                Value::String(s) => s
                    .parse::<i64>()
                    .map(Value::Integer)
                    .map_err(|_| ExecError::TypeError(format!("cannot convert '{}' to integer", s))),
                Value::Bool(true) => Ok(Value::Integer(1)),
                Value::Bool(false) => Ok(Value::Integer(0)),
                Value::Null => Ok(Value::Null),
                _ => Err(ExecError::TypeError(format!(
                    "cannot convert {} to integer",
                    v.type_name()
                ))),
            }
        }
        "tofloat" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::Float(f) => Ok(Value::Float(*f)),
                Value::Integer(i) => Ok(Value::Float(*i as f64)),
                Value::String(s) => s
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| ExecError::TypeError(format!("cannot convert '{}' to float", s))),
                Value::Null => Ok(Value::Null),
                _ => Err(ExecError::TypeError(format!(
                    "cannot convert {} to float",
                    v.type_name()
                ))),
            }
        }
        "size" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::String(s) => Ok(Value::Integer(s.len() as i64)),
                Value::List(l) => Ok(Value::Integer(l.len() as i64)),
                Value::Map(m) => Ok(Value::Integer(m.len() as i64)),
                Value::Null => Ok(Value::Null),
                _ => Err(ExecError::TypeError(format!(
                    "size() not supported for {}",
                    v.type_name()
                ))),
            }
        }
        "keys" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::Map(m) => Ok(Value::List(
                    m.keys().map(|k| Value::String(k.clone())).collect(),
                )),
                Value::Null => Ok(Value::Null),
                _ => Err(ExecError::TypeError(format!(
                    "keys() not supported for {}",
                    v.type_name()
                ))),
            }
        }
        "labels" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::List(l) => Ok(Value::List(l.clone())),
                Value::Null => Ok(Value::Null),
                _ => Ok(Value::List(vec![])),
            }
        }
        "type" => {
            let v = args.first().unwrap_or(&Value::Null);
            Ok(Value::String(v.type_name().to_string()))
        }
        "id" => {
            let v = args.first().unwrap_or(&Value::Null);
            match v {
                Value::String(s) => Ok(Value::String(s.clone())),
                _ => Ok(v.clone()),
            }
        }
        _ => Err(ExecError::NotImplemented(format!("function: {}", name))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn int_lit(v: i64) -> Expression {
        Expression::Literal(Literal::Integer(v))
    }
    fn float_lit(v: f64) -> Expression {
        Expression::Literal(Literal::Float(v))
    }
    fn str_lit(s: &str) -> Expression {
        Expression::Literal(Literal::String(s.to_string()))
    }
    fn bool_lit(b: bool) -> Expression {
        Expression::Literal(Literal::Bool(b))
    }
    fn null_lit() -> Expression {
        Expression::Literal(Literal::Null)
    }
    fn binop(left: Expression, op: BinaryOp, right: Expression) -> Expression {
        Expression::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }
    fn unaryop(op: UnaryOp, operand: Expression) -> Expression {
        Expression::UnaryOp {
            op,
            operand: Box::new(operand),
        }
    }
    fn empty_record() -> Record {
        Record::new(vec![], vec![])
    }

    // -----------------------------------------------------------------------
    // Literal evaluation
    // -----------------------------------------------------------------------

    #[test]
    fn test_literal_integer() {
        let result = evaluate(&int_lit(42), &empty_record()).unwrap();
        assert_eq!(result, Value::Integer(42));
    }

    #[test]
    fn test_literal_float() {
        let result = evaluate(&float_lit(3.14), &empty_record()).unwrap();
        assert_eq!(result, Value::Float(3.14));
    }

    #[test]
    fn test_literal_string() {
        let result = evaluate(&str_lit("hello"), &empty_record()).unwrap();
        assert_eq!(result, Value::String("hello".to_string()));
    }

    #[test]
    fn test_literal_bool_true() {
        let result = evaluate(&bool_lit(true), &empty_record()).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_literal_bool_false() {
        let result = evaluate(&bool_lit(false), &empty_record()).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_literal_null() {
        let result = evaluate(&null_lit(), &empty_record()).unwrap();
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_literal_list() {
        let expr = Expression::Literal(Literal::List(vec![
            Literal::Integer(1),
            Literal::Integer(2),
        ]));
        let result = evaluate(&expr, &empty_record()).unwrap();
        assert_eq!(result, Value::List(vec![Value::Integer(1), Value::Integer(2)]));
    }

    #[test]
    fn test_literal_map() {
        let expr = Expression::Literal(Literal::Map(vec![
            ("a".to_string(), Literal::Integer(1)),
        ]));
        let result = evaluate(&expr, &empty_record()).unwrap();
        let mut expected = BTreeMap::new();
        expected.insert("a".to_string(), Value::Integer(1));
        assert_eq!(result, Value::Map(expected));
    }

    // -----------------------------------------------------------------------
    // Arithmetic operations
    // -----------------------------------------------------------------------

    #[test]
    fn test_add_integers() {
        let expr = binop(int_lit(3), BinaryOp::Add, int_lit(4));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(7));
    }

    #[test]
    fn test_add_floats() {
        let expr = binop(float_lit(1.5), BinaryOp::Add, float_lit(2.5));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Float(4.0));
    }

    #[test]
    fn test_add_mixed_int_float() {
        let expr = binop(int_lit(3), BinaryOp::Add, float_lit(1.5));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Float(4.5));
    }

    #[test]
    fn test_sub_integers() {
        let expr = binop(int_lit(10), BinaryOp::Sub, int_lit(3));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(7));
    }

    #[test]
    fn test_mul_integers() {
        let expr = binop(int_lit(6), BinaryOp::Mul, int_lit(7));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(42));
    }

    #[test]
    fn test_div_integers() {
        let expr = binop(int_lit(10), BinaryOp::Div, int_lit(3));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(3));
    }

    #[test]
    fn test_div_by_zero_error() {
        let expr = binop(int_lit(10), BinaryOp::Div, int_lit(0));
        assert!(evaluate(&expr, &empty_record()).is_err());
    }

    #[test]
    fn test_mod_integers() {
        let expr = binop(int_lit(10), BinaryOp::Mod, int_lit(3));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(1));
    }

    // -----------------------------------------------------------------------
    // Comparison operations
    // -----------------------------------------------------------------------

    #[test]
    fn test_eq_integers() {
        let expr = binop(int_lit(5), BinaryOp::Eq, int_lit(5));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_neq_integers() {
        let expr = binop(int_lit(5), BinaryOp::Neq, int_lit(3));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_lt_integers() {
        let expr = binop(int_lit(3), BinaryOp::Lt, int_lit(5));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_gt_integers() {
        let expr = binop(int_lit(5), BinaryOp::Gt, int_lit(3));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_le_integers() {
        let expr = binop(int_lit(5), BinaryOp::Le, int_lit(5));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
        let expr2 = binop(int_lit(4), BinaryOp::Le, int_lit(5));
        assert_eq!(evaluate(&expr2, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_ge_integers() {
        let expr = binop(int_lit(5), BinaryOp::Ge, int_lit(5));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
        let expr2 = binop(int_lit(6), BinaryOp::Ge, int_lit(5));
        assert_eq!(evaluate(&expr2, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_compare_strings() {
        let expr = binop(str_lit("abc"), BinaryOp::Lt, str_lit("def"));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_compare_null_returns_null() {
        let expr = binop(null_lit(), BinaryOp::Lt, int_lit(5));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Null);
    }

    // -----------------------------------------------------------------------
    // Boolean logic
    // -----------------------------------------------------------------------

    #[test]
    fn test_and_true_true() {
        let expr = binop(bool_lit(true), BinaryOp::And, bool_lit(true));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_and_true_false() {
        let expr = binop(bool_lit(true), BinaryOp::And, bool_lit(false));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_or_false_true() {
        let expr = binop(bool_lit(false), BinaryOp::Or, bool_lit(true));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_xor_true_false() {
        let expr = binop(bool_lit(true), BinaryOp::Xor, bool_lit(false));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_not_true() {
        let expr = unaryop(UnaryOp::Not, bool_lit(true));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_not_false() {
        let expr = unaryop(UnaryOp::Not, bool_lit(false));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_and_short_circuit() {
        // false AND (unknown_identifier) should short-circuit to false
        let error_expr = Expression::Identifier("nonexistent".to_string());
        let expr = binop(bool_lit(false), BinaryOp::And, error_expr);
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(false));
    }

    // -----------------------------------------------------------------------
    // String / list concatenation
    // -----------------------------------------------------------------------

    #[test]
    fn test_concat_strings() {
        let expr = binop(str_lit("hello"), BinaryOp::Concat, str_lit(" world"));
        assert_eq!(
            evaluate(&expr, &empty_record()).unwrap(),
            Value::String("hello world".to_string())
        );
    }

    #[test]
    fn test_concat_lists() {
        let left = Expression::List(vec![int_lit(1)]);
        let right = Expression::List(vec![int_lit(2), int_lit(3)]);
        let expr = binop(left, BinaryOp::Concat, right);
        assert_eq!(
            evaluate(&expr, &empty_record()).unwrap(),
            Value::List(vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)])
        );
    }

    // -----------------------------------------------------------------------
    // Null handling
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_null_on_null() {
        let expr = Expression::IsNull(Box::new(null_lit()));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_is_null_on_value() {
        let expr = Expression::IsNull(Box::new(int_lit(5)));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(false));
    }

    #[test]
    fn test_is_not_null_on_value() {
        let expr = Expression::IsNotNull(Box::new(int_lit(5)));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_is_not_null_on_null() {
        let expr = Expression::IsNotNull(Box::new(null_lit()));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(false));
    }

    // -----------------------------------------------------------------------
    // Unary operations
    // -----------------------------------------------------------------------

    #[test]
    fn test_neg_integer() {
        let expr = unaryop(UnaryOp::Neg, int_lit(5));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(-5));
    }

    #[test]
    fn test_neg_float() {
        let expr = unaryop(UnaryOp::Neg, float_lit(3.14));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Float(-3.14));
    }

    #[test]
    fn test_pos_integer() {
        let expr = unaryop(UnaryOp::Pos, int_lit(5));
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(5));
    }

    // -----------------------------------------------------------------------
    // IN expression
    // -----------------------------------------------------------------------

    #[test]
    fn test_in_found() {
        let expr = Expression::In {
            operand: Box::new(int_lit(3)),
            list: Box::new(Expression::List(vec![int_lit(1), int_lit(2), int_lit(3)])),
        };
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_in_not_found() {
        let expr = Expression::In {
            operand: Box::new(int_lit(4)),
            list: Box::new(Expression::List(vec![int_lit(1), int_lit(2), int_lit(3)])),
        };
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Bool(false));
    }

    // -----------------------------------------------------------------------
    // Function calls
    // -----------------------------------------------------------------------

    fn fn_call(name: &str, args: Vec<Expression>) -> Expression {
        Expression::FunctionCall {
            name: name.to_string(),
            args,
        }
    }

    #[test]
    fn test_fn_tostring_integer() {
        let expr = fn_call("toString", vec![int_lit(42)]);
        assert_eq!(
            evaluate(&expr, &empty_record()).unwrap(),
            Value::String("42".to_string())
        );
    }

    #[test]
    fn test_fn_tointeger_string() {
        let expr = fn_call("toInteger", vec![str_lit("42")]);
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(42));
    }

    #[test]
    fn test_fn_tofloat_integer() {
        let expr = fn_call("toFloat", vec![int_lit(42)]);
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Float(42.0));
    }

    #[test]
    fn test_fn_size_string() {
        let expr = fn_call("size", vec![str_lit("hello")]);
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(5));
    }

    #[test]
    fn test_fn_size_list() {
        let expr = fn_call(
            "size",
            vec![Expression::List(vec![int_lit(1), int_lit(2), int_lit(3)])],
        );
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Integer(3));
    }

    #[test]
    fn test_fn_keys_map() {
        let expr = fn_call(
            "keys",
            vec![Expression::Map(vec![
                ("a".to_string(), int_lit(1)),
                ("b".to_string(), int_lit(2)),
            ])],
        );
        let result = evaluate(&expr, &empty_record()).unwrap();
        // BTreeMap keys are sorted alphabetically
        assert_eq!(
            result,
            Value::List(vec![
                Value::String("a".to_string()),
                Value::String("b".to_string()),
            ])
        );
    }

    #[test]
    fn test_fn_type_integer() {
        let expr = fn_call("type", vec![int_lit(42)]);
        assert_eq!(
            evaluate(&expr, &empty_record()).unwrap(),
            Value::String("INTEGER".to_string())
        );
    }

    // -----------------------------------------------------------------------
    // Property access
    // -----------------------------------------------------------------------

    #[test]
    fn test_property_access_direct_column() {
        let record = Record::new(
            vec!["n.name".to_string()],
            vec![Value::String("Alice".to_string())],
        );
        let expr = Expression::PropertyAccess {
            object: Box::new(Expression::Identifier("n".to_string())),
            property: "name".to_string(),
        };
        assert_eq!(
            evaluate(&expr, &record).unwrap(),
            Value::String("Alice".to_string())
        );
    }

    #[test]
    fn test_property_access_fallback() {
        let record = Record::new(
            vec!["name".to_string()],
            vec![Value::String("Alice".to_string())],
        );
        let expr = Expression::PropertyAccess {
            object: Box::new(Expression::Identifier("n".to_string())),
            property: "name".to_string(),
        };
        assert_eq!(
            evaluate(&expr, &record).unwrap(),
            Value::String("Alice".to_string())
        );
    }

    // -----------------------------------------------------------------------
    // CASE expression
    // -----------------------------------------------------------------------

    #[test]
    fn test_case_when_match() {
        let expr = Expression::Case {
            operand: None,
            when_clauses: vec![(bool_lit(true), str_lit("yes"))],
            else_clause: Some(Box::new(str_lit("no"))),
        };
        assert_eq!(
            evaluate(&expr, &empty_record()).unwrap(),
            Value::String("yes".to_string())
        );
    }

    #[test]
    fn test_case_when_no_match() {
        let expr = Expression::Case {
            operand: None,
            when_clauses: vec![(bool_lit(false), str_lit("yes"))],
            else_clause: Some(Box::new(str_lit("no"))),
        };
        assert_eq!(
            evaluate(&expr, &empty_record()).unwrap(),
            Value::String("no".to_string())
        );
    }

    #[test]
    fn test_case_no_match_no_else() {
        let expr = Expression::Case {
            operand: None,
            when_clauses: vec![(bool_lit(false), str_lit("yes"))],
            else_clause: None,
        };
        assert_eq!(evaluate(&expr, &empty_record()).unwrap(), Value::Null);
    }

    // -----------------------------------------------------------------------
    // Identifier lookup
    // -----------------------------------------------------------------------

    #[test]
    fn test_identifier_found() {
        let record = Record::new(vec!["x".to_string()], vec![Value::Integer(42)]);
        let expr = Expression::Identifier("x".to_string());
        assert_eq!(evaluate(&expr, &record).unwrap(), Value::Integer(42));
    }

    #[test]
    fn test_identifier_not_found_error() {
        let expr = Expression::Identifier("missing".to_string());
        assert!(evaluate(&expr, &empty_record()).is_err());
    }
}
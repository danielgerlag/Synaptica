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
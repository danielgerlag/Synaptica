use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// GQL-compliant value types supporting all scalar and composite types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Date(NaiveDate),
    Time(NaiveTime),
    Timestamp(NaiveDateTime),
    Duration(Duration),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "NULL",
            Value::Bool(_) => "BOOLEAN",
            Value::Integer(_) => "INTEGER",
            Value::Float(_) => "FLOAT",
            Value::String(_) => "STRING",
            Value::Bytes(_) => "BYTES",
            Value::Date(_) => "DATE",
            Value::Time(_) => "TIME",
            Value::Timestamp(_) => "TIMESTAMP",
            Value::Duration(_) => "DURATION",
            Value::List(_) => "LIST",
            Value::Map(_) => "MAP",
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => write!(f, "NULL"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Integer(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::String(s) => write!(f, "'{}'", s),
            Value::Bytes(b) => write!(f, "X'{}'", hex::encode(b)),
            Value::Date(d) => write!(f, "DATE '{}'", d),
            Value::Time(t) => write!(f, "TIME '{}'", t),
            Value::Timestamp(ts) => write!(f, "TIMESTAMP '{}'", ts),
            Value::Duration(d) => write!(f, "DURATION '{}'", d),
            Value::List(l) => {
                write!(f, "[")?;
                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Map(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
        }
    }
}

/// ISO 8601 duration representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Duration {
    pub months: i32,
    pub days: i32,
    pub nanos: i64,
}

impl Duration {
    pub fn new(months: i32, days: i32, nanos: i64) -> Self {
        Self {
            months,
            days,
            nanos,
        }
    }

    pub fn zero() -> Self {
        Self::new(0, 0, 0)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P")?;
        if self.months != 0 {
            write!(f, "{}M", self.months)?;
        }
        if self.days != 0 {
            write!(f, "{}D", self.days)?;
        }
        if self.nanos != 0 {
            let secs = self.nanos / 1_000_000_000;
            let rem = self.nanos % 1_000_000_000;
            if rem != 0 {
                write!(f, "T{}.{:09}S", secs, rem.unsigned_abs())?;
            } else {
                write!(f, "T{}S", secs)?;
            }
        }
        Ok(())
    }
}

/// GQL data type descriptors for schema definitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    Boolean,
    Integer,
    Float,
    String,
    Bytes,
    Date,
    Time,
    Timestamp,
    Duration,
    List(Box<DataType>),
    Map(Box<DataType>, Box<DataType>),
    Any,
}

/// Helper to encode hex strings (minimal implementation to avoid extra dep).
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}
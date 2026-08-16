use crate::error::HkError;
use indexmap::IndexMap;

/// Represents the structure of a .hk file.
/// Sections are top-level keys in the outer IndexMap to preserve order.
pub type HkConfig = IndexMap<String, HkValue>;

/// Enum for values in the .hk config: supports strings, numbers, booleans, arrays, and maps.
#[derive(Debug, Clone, PartialEq)]
pub enum HkValue {
    String(String),
    Number(f64),
    Bool(bool),
    Array(Vec<HkValue>),
    Map(IndexMap<String, HkValue>),
}

impl HkValue {
    pub fn as_string(&self) -> Result<String, HkError> {
        match self {
            Self::String(s) => Ok(s.clone()),
            Self::Number(n) => Ok(n.to_string()),
            Self::Bool(b) => Ok(b.to_string()),
            _ => Err(HkError::TypeMismatch {
                expected: "string".to_string(),
                found: format!("{:?}", self),
            }),
        }
    }

    pub fn as_number(&self) -> Result<f64, HkError> {
        if let Self::Number(n) = self {
            Ok(*n)
        } else {
            Err(HkError::TypeMismatch {
                expected: "number".to_string(),
                found: format!("{:?}", self),
            })
        }
    }

    pub fn as_bool(&self) -> Result<bool, HkError> {
        if let Self::Bool(b) = self {
            Ok(*b)
        } else {
            Err(HkError::TypeMismatch {
                expected: "bool".to_string(),
                found: format!("{:?}", self),
            })
        }
    }

    pub fn as_array(&self) -> Result<&Vec<HkValue>, HkError> {
        if let Self::Array(a) = self {
            Ok(a)
        } else {
            Err(HkError::TypeMismatch {
                expected: "array".to_string(),
                found: format!("{:?}", self),
            })
        }
    }

    pub fn as_map(&self) -> Result<&IndexMap<String, HkValue>, HkError> {
        if let Self::Map(m) = self {
            Ok(m)
        } else {
            Err(HkError::TypeMismatch {
                expected: "map".to_string(),
                found: format!("{:?}", self),
            })
        }
    }
}

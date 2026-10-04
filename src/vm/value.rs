use std::fmt;

/// A runtime value suitable for storage in VM bytecode constant pools.
///
/// This type is intentionally separate from [`crate::interpreter::Value`].
/// The tree-walking interpreter stores environments, functions, and shared
/// mutable collections, while the VM will eventually use its own stack and
/// object representation.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Int(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
            Self::String(value) => write!(f, "{value}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    #[test]
    fn values_compare_by_type_and_contents() {
        assert_eq!(Value::Null, Value::Null);
        assert_eq!(Value::Bool(true), Value::Bool(true));
        assert_eq!(Value::Int(42), Value::Int(42));
        assert_eq!(Value::Float(3.5), Value::Float(3.5));
        assert_eq!(
            Value::String("hanlin".to_owned()),
            Value::String("hanlin".to_owned())
        );
        assert_ne!(Value::Int(1), Value::Float(1.0));
    }

    #[test]
    fn display_uses_hanlin_literal_values() {
        assert_eq!(Value::Null.to_string(), "null");
        assert_eq!(Value::Bool(false).to_string(), "false");
        assert_eq!(Value::Int(-7).to_string(), "-7");
        assert_eq!(Value::Float(3.5).to_string(), "3.5");
        assert_eq!(Value::String("hello".to_owned()).to_string(), "hello");
    }

    #[test]
    fn debug_identifies_value_variants() {
        assert_eq!(format!("{:?}", Value::Int(9)), "Int(9)");
        assert_eq!(
            format!("{:?}", Value::String("text".to_owned())),
            "String(\"text\")"
        );
    }
}

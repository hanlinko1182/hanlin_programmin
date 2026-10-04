use std::fmt;
use std::rc::Rc;

use super::{Closure, Function};

/// A runtime value suitable for storage in VM bytecode constant pools.
///
/// This type is intentionally separate from [`crate::interpreter::Value`].
/// The tree-walking interpreter stores environments, functions, and shared
/// mutable collections, while the VM uses compiled functions and closures with
/// shared upvalue storage.
#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    /// Immutable compiled code stored in constant pools.
    Function(Rc<Function>),
    /// Callable runtime function paired with captured lexical state.
    Closure(Rc<Closure>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(left), Self::Bool(right)) => left == right,
            (Self::Int(left), Self::Int(right)) => left == right,
            (Self::Float(left), Self::Float(right)) => left == right,
            (Self::String(left), Self::String(right)) => left == right,
            (Self::Function(left), Self::Function(right)) => Rc::ptr_eq(left, right),
            (Self::Closure(left), Self::Closure(right)) => Rc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl Value {
    /// Returns this VM value's truthiness using the tree-walking interpreter's
    /// primitive-value semantics.
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(value) => *value,
            Self::Int(value) => *value != 0,
            Self::Float(value) => *value != 0.0 && !value.is_nan(),
            Self::String(value) => !value.is_empty(),
            Self::Function(_) => true,
            Self::Closure(_) => true,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Int(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
            Self::String(value) => write!(f, "{value}"),
            Self::Function(function) => write!(f, "<fn {}>", function.name()),
            Self::Closure(closure) => write!(f, "<fn {}>", closure.function().name()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::Value;
    use crate::vm::{Arity, Chunk, Closure, Function};

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

    #[test]
    fn truthiness_matches_interpreter_primitives() {
        assert!(!Value::Null.is_truthy());
        assert!(!Value::Bool(false).is_truthy());
        assert!(Value::Bool(true).is_truthy());
        assert!(!Value::Int(0).is_truthy());
        assert!(Value::Int(-1).is_truthy());
        assert!(!Value::Float(0.0).is_truthy());
        assert!(!Value::Float(f64::NAN).is_truthy());
        assert!(Value::Float(0.5).is_truthy());
        assert!(!Value::String(String::new()).is_truthy());
        assert!(Value::String("hanlin".to_owned()).is_truthy());
    }

    #[test]
    fn function_values_use_identity_equality_and_deterministic_display() {
        let function = Rc::new(Function::new("add", Arity::new(2), Chunk::new()));
        let same = Value::Function(Rc::clone(&function));
        let original = Value::Function(function);
        let distinct = Value::Function(Rc::new(Function::new("add", Arity::new(2), Chunk::new())));

        assert_eq!(original.to_string(), "<fn add>");
        assert_eq!(original, same);
        assert_ne!(original, distinct);
        assert!(original.is_truthy());
    }

    #[test]
    fn closure_values_use_identity_equality_and_function_display() {
        let function = Rc::new(Function::new("counter", Arity::new(0), Chunk::new()));
        let closure = Rc::new(Closure::new(Rc::clone(&function)));
        let original = Value::Closure(Rc::clone(&closure));
        let same = Value::Closure(closure);
        let distinct = Value::Closure(Rc::new(Closure::new(function)));

        assert_eq!(original.to_string(), "<fn counter>");
        assert_eq!(original, same);
        assert_ne!(original, distinct);
        assert!(original.is_truthy());
    }
}

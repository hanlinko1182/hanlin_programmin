use std::collections::HashSet;
use std::fmt;
use std::rc::Rc;

use super::{Closure, Function, HeapObject, HeapObjectRef};

/// A runtime value suitable for storage in VM bytecode constant pools.
///
/// This type is intentionally separate from [`crate::interpreter::Value`].
/// The tree-walking interpreter stores environments, functions, and shared
/// mutable collections, while the VM uses compiled functions and closures with
/// shared upvalue storage.
#[derive(Clone)]
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
    /// Shared mutable aggregate allocated in the VM heap-object model.
    Heap(HeapObjectRef),
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
            (Self::Heap(left), Self::Heap(right)) => Rc::ptr_eq(left, right),
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
            Self::Heap(_) => true,
        }
    }

    pub fn array(elements: Vec<Self>) -> Self {
        Self::Heap(HeapObject::array(elements))
    }

    pub fn map(entries: std::collections::HashMap<String, Self>) -> Self {
        Self::Heap(HeapObject::map(entries))
    }

    fn fmt_with(
        &self,
        f: &mut fmt::Formatter<'_>,
        quote_string: bool,
        active: &mut HashSet<*const std::cell::RefCell<HeapObject>>,
    ) -> fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Int(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
            Self::String(value) if quote_string => write!(f, "\"{value}\""),
            Self::String(value) => write!(f, "{value}"),
            Self::Function(function) => write!(f, "<fn {}>", function.name()),
            Self::Closure(closure) => write!(f, "<fn {}>", closure.function().name()),
            Self::Heap(object) => {
                let pointer = Rc::as_ptr(object);
                if !active.insert(pointer) {
                    return write!(f, "<cycle>");
                }
                let object = object.borrow();
                let result = match &*object {
                    HeapObject::Array(elements) => {
                        write!(f, "[")?;
                        for (index, value) in elements.iter().enumerate() {
                            if index > 0 {
                                write!(f, ", ")?;
                            }
                            value.fmt_with(f, true, active)?;
                        }
                        write!(f, "]")
                    }
                    HeapObject::Map(entries) => {
                        let mut entries: Vec<_> = entries.iter().collect();
                        entries.sort_by_key(|(key, _)| *key);
                        write!(f, "{{ ")?;
                        for (index, (key, value)) in entries.into_iter().enumerate() {
                            if index > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{key}: ")?;
                            value.fmt_with(f, true, active)?;
                        }
                        write!(f, " }}")
                    }
                };
                active.remove(&pointer);
                result
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_with(f, false, &mut HashSet::new())
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "Null"),
            Self::Bool(value) => f.debug_tuple("Bool").field(value).finish(),
            Self::Int(value) => f.debug_tuple("Int").field(value).finish(),
            Self::Float(value) => f.debug_tuple("Float").field(value).finish(),
            Self::String(value) => f.debug_tuple("String").field(value).finish(),
            Self::Function(function) => f.debug_tuple("Function").field(function).finish(),
            Self::Closure(closure) => f.debug_tuple("Closure").field(closure).finish(),
            Self::Heap(_) => write!(f, "Heap({self})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::rc::Rc;

    use super::{HeapObject, Value};
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

    #[test]
    fn aggregate_values_use_identity_equality_and_are_truthy() {
        let original = Value::array(Vec::new());
        let same = original.clone();
        let distinct = Value::array(Vec::new());

        assert_eq!(original, same);
        assert_ne!(original, distinct);
        assert!(original.is_truthy());
        assert!(Value::map(HashMap::new()).is_truthy());
    }

    #[test]
    fn aggregate_display_is_nested_and_deterministic() {
        let mut entries = HashMap::new();
        entries.insert("name".to_owned(), Value::String("Han".to_owned()));
        entries.insert(
            "items".to_owned(),
            Value::array(vec![Value::Int(1), Value::Bool(true)]),
        );
        let value = Value::map(entries);

        assert_eq!(value.to_string(), "{ items: [1, true], name: \"Han\" }");
        assert_eq!(value.to_string(), value.to_string());
    }

    #[test]
    fn aggregate_display_handles_reference_cycles() {
        let value = Value::array(vec![Value::Null]);
        let Value::Heap(object) = &value else {
            unreachable!()
        };
        let mut object = object.borrow_mut();
        let HeapObject::Array(elements) = &mut *object else {
            unreachable!()
        };
        elements[0] = value.clone();
        drop(object);

        assert_eq!(value.to_string(), "[<cycle>]");
    }
}

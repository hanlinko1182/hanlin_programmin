//! Heap-backed aggregate values for the VM.
//!
//! VM-14 uses reference counting plus interior mutability so arrays and maps
//! have shared reference semantics. Garbage collection is intentionally
//! deferred; VM-15 may replace this ownership mechanism with a collector
//! without changing Hanlin's observable aggregate behavior.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::Value;

/// Shared ownership of one mutable VM heap object.
pub type HeapObjectRef = Rc<RefCell<HeapObject>>;

/// Mutable aggregate payloads allocated outside the VM operand stack.
#[derive(Debug)]
pub enum HeapObject {
    Array(Vec<Value>),
    Map(HashMap<String, Value>),
}

impl HeapObject {
    pub fn array(elements: Vec<Value>) -> HeapObjectRef {
        Rc::new(RefCell::new(Self::Array(elements)))
    }

    pub fn map(entries: HashMap<String, Value>) -> HeapObjectRef {
        Rc::new(RefCell::new(Self::Map(entries)))
    }
}

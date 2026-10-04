use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use super::{Function, Value};

pub(crate) type Upvalue = Rc<RefCell<UpvalueState>>;

/// Shared storage for a captured variable.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UpvalueState {
    Open(usize),
    Closed(Value),
}

/// A callable compiled function paired with its captured lexical state.
pub struct Closure {
    function: Rc<Function>,
    upvalues: Vec<Upvalue>,
}

impl Closure {
    pub fn new(function: Rc<Function>) -> Self {
        Self {
            function,
            upvalues: Vec::new(),
        }
    }

    pub(crate) fn with_upvalues(function: Rc<Function>, upvalues: Vec<Upvalue>) -> Self {
        Self { function, upvalues }
    }

    pub fn function(&self) -> &Rc<Function> {
        &self.function
    }

    pub fn upvalue_count(&self) -> usize {
        self.upvalues.len()
    }

    pub(crate) fn upvalue(&self, index: usize) -> Option<&Upvalue> {
        self.upvalues.get(index)
    }

    pub(crate) fn upvalues(&self) -> &[Upvalue] {
        &self.upvalues
    }
}

impl fmt::Debug for Closure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Closure")
            .field("function", &self.function.name())
            .field("upvalue_count", &self.upvalues.len())
            .finish()
    }
}

use std::fmt;

use super::{Arity, Chunk, LocalSlot, UpvalueIndex};

/// Describes how a closure captures one value from its enclosing closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpvalueDescriptor {
    /// Capture a frame-relative local from the immediately enclosing call.
    Local(LocalSlot),
    /// Reuse an upvalue already owned by the immediately enclosing closure.
    Upvalue(UpvalueIndex),
}

/// A named Hanlin function compiled to its own bytecode chunk.
///
/// Functions contain bytecode only; they do not retain interpreter AST nodes
/// or an environment. Closure capture is intentionally deferred.
pub struct Function {
    name: String,
    arity: Arity,
    chunk: Chunk,
    upvalues: Vec<UpvalueDescriptor>,
}

impl Function {
    pub fn new(name: impl Into<String>, arity: Arity, chunk: Chunk) -> Self {
        Self {
            name: name.into(),
            arity,
            chunk,
            upvalues: Vec::new(),
        }
    }

    pub fn with_upvalues(
        name: impl Into<String>,
        arity: Arity,
        chunk: Chunk,
        upvalues: Vec<UpvalueDescriptor>,
    ) -> Self {
        Self {
            name: name.into(),
            arity,
            chunk,
            upvalues,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn arity(&self) -> Arity {
        self.arity
    }

    pub const fn chunk(&self) -> &Chunk {
        &self.chunk
    }

    pub fn upvalues(&self) -> &[UpvalueDescriptor] {
        &self.upvalues
    }
}

impl fmt::Debug for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Function")
            .field("name", &self.name)
            .field("arity", &self.arity)
            .field("upvalue_count", &self.upvalues.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{Function, UpvalueDescriptor};
    use crate::vm::{Arity, Chunk, LocalSlot, UpvalueIndex};

    #[test]
    fn function_keeps_deterministic_capture_metadata() {
        let descriptors = vec![
            UpvalueDescriptor::Local(LocalSlot::new(2)),
            UpvalueDescriptor::Upvalue(UpvalueIndex::new(1)),
        ];
        let function =
            Function::with_upvalues("inner", Arity::new(0), Chunk::new(), descriptors.clone());

        assert_eq!(function.upvalues(), descriptors);
        assert!(format!("{function:?}").contains("upvalue_count: 2"));
    }
}

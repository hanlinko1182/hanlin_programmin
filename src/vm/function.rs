use std::fmt;

use super::{Arity, Chunk};

/// A named Hanlin function compiled to its own bytecode chunk.
///
/// Functions contain bytecode only; they do not retain interpreter AST nodes
/// or an environment. Closure capture is intentionally deferred.
pub struct Function {
    name: String,
    arity: Arity,
    chunk: Chunk,
}

impl Function {
    pub fn new(name: impl Into<String>, arity: Arity, chunk: Chunk) -> Self {
        Self {
            name: name.into(),
            arity,
            chunk,
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
}

impl fmt::Debug for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Function")
            .field("name", &self.name)
            .field("arity", &self.arity)
            .finish_non_exhaustive()
    }
}

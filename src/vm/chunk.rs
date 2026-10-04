use std::fmt;

use crate::error::Span;

use super::opcode::{ConstantIndex, OpCode};
use super::value::Value;

/// A bytecode instruction paired with its originating Hanlin source location.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instruction {
    opcode: OpCode,
    span: Span,
}

impl Instruction {
    pub const fn opcode(&self) -> OpCode {
        self.opcode
    }

    pub const fn span(&self) -> Span {
        self.span
    }
}

/// An error encountered while building a bytecode chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkError {
    TooManyConstants,
}

impl fmt::Display for ChunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyConstants => write!(f, "bytecode constant pool exceeds u32 capacity"),
        }
    }
}

impl std::error::Error for ChunkError {}

/// One unit of bytecode, including instructions, constants, and source spans.
///
/// The current compiler does not yet translate AST nodes into chunks. For now,
/// chunks are constructed manually and can be executed by the experimental VM.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    instructions: Vec<Instruction>,
    constants: Vec<Value>,
}

impl Chunk {
    pub const fn new() -> Self {
        Self {
            instructions: Vec::new(),
            constants: Vec::new(),
        }
    }

    /// Adds a value to the constant pool and returns its checked pool index.
    pub fn add_constant(&mut self, value: Value) -> Result<ConstantIndex, ChunkError> {
        let raw_index =
            u32::try_from(self.constants.len()).map_err(|_| ChunkError::TooManyConstants)?;
        self.constants.push(value);
        Ok(ConstantIndex::new(raw_index))
    }

    /// Appends an instruction and its source location, returning its offset.
    pub fn write_instruction(&mut self, opcode: OpCode, span: Span) -> usize {
        let offset = self.instructions.len();
        self.instructions.push(Instruction { opcode, span });
        offset
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub fn constants(&self) -> &[Value] {
        &self.constants
    }

    pub fn instruction(&self, offset: usize) -> Option<&Instruction> {
        self.instructions.get(offset)
    }

    pub fn constant(&self, index: ConstantIndex) -> Option<&Value> {
        let offset = usize::try_from(index.as_u32()).ok()?;
        self.constants.get(offset)
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Chunk;
    use crate::error::Span;
    use crate::vm::{OpCode, Value};

    #[test]
    fn new_chunk_is_empty() {
        let chunk = Chunk::new();

        assert!(chunk.instructions().is_empty());
        assert!(chunk.constants().is_empty());
    }

    #[test]
    fn adds_integer_constant() {
        let mut chunk = Chunk::new();
        let index = chunk.add_constant(Value::Int(42)).unwrap();

        assert_eq!(index.as_u32(), 0);
        assert_eq!(chunk.constant(index), Some(&Value::Int(42)));
    }

    #[test]
    fn adds_float_constant() {
        let mut chunk = Chunk::new();
        let index = chunk.add_constant(Value::Float(3.5)).unwrap();

        assert_eq!(chunk.constant(index), Some(&Value::Float(3.5)));
    }

    #[test]
    fn adds_string_constant() {
        let mut chunk = Chunk::new();
        let index = chunk
            .add_constant(Value::String("hello".to_owned()))
            .unwrap();

        assert_eq!(
            chunk.constant(index),
            Some(&Value::String("hello".to_owned()))
        );
    }

    #[test]
    fn constant_indexes_follow_insertion_order() {
        let mut chunk = Chunk::new();
        let first = chunk.add_constant(Value::Int(10)).unwrap();
        let second = chunk.add_constant(Value::Int(20)).unwrap();
        let third = chunk.add_constant(Value::Int(30)).unwrap();

        assert_eq!(first.as_u32(), 0);
        assert_eq!(second.as_u32(), 1);
        assert_eq!(third.as_u32(), 2);
        assert_eq!(
            chunk.constants(),
            &[Value::Int(10), Value::Int(20), Value::Int(30)]
        );
    }

    #[test]
    fn writes_instruction_at_next_offset() {
        let mut chunk = Chunk::new();
        let offset = chunk.write_instruction(OpCode::True, Span::new(1, 1));

        assert_eq!(offset, 0);
        assert_eq!(chunk.instruction(offset).unwrap().opcode(), OpCode::True);
    }

    #[test]
    fn constant_index_is_an_instruction_operand() {
        let mut chunk = Chunk::new();
        let constant = chunk.add_constant(Value::Int(42)).unwrap();
        let offset = chunk.write_instruction(OpCode::Constant(constant), Span::new(1, 1));

        assert_eq!(
            chunk.instruction(offset).unwrap().opcode(),
            OpCode::Constant(constant)
        );
    }

    #[test]
    fn instruction_preserves_source_span() {
        let mut chunk = Chunk::new();
        let span = Span::new(12, 7);
        let offset = chunk.write_instruction(OpCode::Return, span);

        assert_eq!(chunk.instruction(offset).unwrap().span(), span);
    }
}

//! Human-readable formatting for Hanlin bytecode chunks.
//!
//! The disassembler is debugging infrastructure for tests, future tracing,
//! and future CLI tooling. It formats bytecode but does not execute it.

use std::fmt;
use std::fmt::Write;

use super::{Chunk, ConstantIndex, LocalSlot, OpCode, Value};

/// An error found while disassembling a bytecode chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisassembleError {
    InvalidInstructionOffset {
        offset: usize,
        instruction_count: usize,
    },
    InvalidConstantIndex {
        offset: usize,
        index: u32,
        constant_count: usize,
    },
    InvalidGlobalName {
        offset: usize,
        index: u32,
    },
}

impl fmt::Display for DisassembleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInstructionOffset {
                offset,
                instruction_count,
            } => write!(
                f,
                "instruction offset {offset} is out of bounds for a chunk with {instruction_count} instructions"
            ),
            Self::InvalidConstantIndex {
                offset,
                index,
                constant_count,
            } => write!(
                f,
                "instruction {offset} references constant index {index}, but the chunk contains {constant_count} constants"
            ),
            Self::InvalidGlobalName { offset, index } => write!(
                f,
                "instruction {offset} references constant index {index} as a global name, but it is not a string"
            ),
        }
    }
}

impl std::error::Error for DisassembleError {}

/// Formats an entire chunk as deterministic, human-readable bytecode.
pub fn disassemble_chunk(chunk: &Chunk, name: &str) -> Result<String, DisassembleError> {
    let mut output = format!("== {name} ==\n\n");

    for offset in 0..chunk.instructions().len() {
        writeln!(output, "{}", disassemble_instruction(chunk, offset)?)
            .expect("writing to a String cannot fail");
    }

    Ok(output)
}

/// Formats one instruction at `offset` without a trailing newline.
pub fn disassemble_instruction(chunk: &Chunk, offset: usize) -> Result<String, DisassembleError> {
    let instruction =
        chunk
            .instruction(offset)
            .ok_or(DisassembleError::InvalidInstructionOffset {
                offset,
                instruction_count: chunk.instructions().len(),
            })?;
    let span = instruction.span().to_string();
    let opcode = instruction.opcode();
    let name = opcode_name(opcode);

    match opcode {
        OpCode::Constant(index) => {
            format_indexed_instruction(chunk, offset, &span, name, index, false)
        }
        OpCode::DefineGlobal(index) | OpCode::GetGlobal(index) | OpCode::SetGlobal(index) => {
            format_indexed_instruction(chunk, offset, &span, name, index, true)
        }
        OpCode::GetLocal(slot) | OpCode::SetLocal(slot) => {
            Ok(format_local_instruction(offset, &span, name, slot))
        }
        _ => Ok(format!("{offset:04}  {span:<6} {name}")),
    }
}

fn format_local_instruction(offset: usize, span: &str, name: &str, slot: LocalSlot) -> String {
    format!("{offset:04}  {span:<6} {name:<14} {}", slot.as_u16())
}

fn format_indexed_instruction(
    chunk: &Chunk,
    offset: usize,
    span: &str,
    name: &str,
    index: ConstantIndex,
    require_string: bool,
) -> Result<String, DisassembleError> {
    let value = chunk
        .constant(index)
        .ok_or(DisassembleError::InvalidConstantIndex {
            offset,
            index: index.as_u32(),
            constant_count: chunk.constants().len(),
        })?;
    if require_string && !matches!(value, Value::String(_)) {
        return Err(DisassembleError::InvalidGlobalName {
            offset,
            index: index.as_u32(),
        });
    }

    Ok(format!(
        "{offset:04}  {span:<6} {name:<14} {:<4} {}",
        index.as_u32(),
        format_constant(value)
    ))
}

fn opcode_name(opcode: OpCode) -> &'static str {
    match opcode {
        OpCode::Constant(_) => "CONSTANT",
        OpCode::Null => "NULL",
        OpCode::True => "TRUE",
        OpCode::False => "FALSE",
        OpCode::Pop => "POP",
        OpCode::Add => "ADD",
        OpCode::Subtract => "SUBTRACT",
        OpCode::Multiply => "MULTIPLY",
        OpCode::Divide => "DIVIDE",
        OpCode::Modulo => "MODULO",
        OpCode::Negate => "NEGATE",
        OpCode::Not => "NOT",
        OpCode::Equal => "EQUAL",
        OpCode::NotEqual => "NOT_EQUAL",
        OpCode::Less => "LESS",
        OpCode::LessEqual => "LESS_EQUAL",
        OpCode::Greater => "GREATER",
        OpCode::GreaterEqual => "GREATER_EQUAL",
        OpCode::GetLocal(_) => "GET_LOCAL",
        OpCode::SetLocal(_) => "SET_LOCAL",
        OpCode::DefineGlobal(_) => "DEFINE_GLOBAL",
        OpCode::GetGlobal(_) => "GET_GLOBAL",
        OpCode::SetGlobal(_) => "SET_GLOBAL",
        OpCode::Return => "RETURN",
    }
}

fn format_constant(value: &Value) -> String {
    match value {
        Value::String(value) => {
            let escaped: String = value.chars().flat_map(char::escape_default).collect();
            format!("\"{escaped}\"")
        }
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{disassemble_chunk, disassemble_instruction, DisassembleError};
    use crate::error::Span;
    use crate::vm::{Chunk, ConstantIndex, LocalSlot, OpCode, Value};

    fn chunk_with(opcodes: &[OpCode]) -> Chunk {
        let mut chunk = Chunk::new();
        for (offset, opcode) in opcodes.iter().copied().enumerate() {
            chunk.write_instruction(opcode, Span::new(1, offset + 1));
        }
        chunk
    }

    #[test]
    fn disassembles_empty_chunk() {
        assert_eq!(
            disassemble_chunk(&Chunk::new(), "empty").unwrap(),
            "== empty ==\n\n"
        );
    }

    #[test]
    fn disassembles_integer_constant() {
        let mut chunk = Chunk::new();
        let index = chunk.add_constant(Value::Int(10)).unwrap();
        chunk.write_instruction(OpCode::Constant(index), Span::new(1, 1));

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:1    CONSTANT       0    10"
        );
    }

    #[test]
    fn disassembles_float_constant() {
        let mut chunk = Chunk::new();
        let index = chunk.add_constant(Value::Float(3.5)).unwrap();
        chunk.write_instruction(OpCode::Constant(index), Span::new(1, 1));

        assert!(disassemble_instruction(&chunk, 0)
            .unwrap()
            .ends_with("0    3.5"));
    }

    #[test]
    fn disassembles_quoted_string_constant() {
        let mut chunk = Chunk::new();
        let index = chunk
            .add_constant(Value::String("hello\nworld".to_owned()))
            .unwrap();
        chunk.write_instruction(OpCode::Constant(index), Span::new(2, 10));

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  2:10   CONSTANT       0    \"hello\\nworld\""
        );
    }

    #[test]
    fn disassembles_null_true_and_false() {
        let chunk = chunk_with(&[OpCode::Null, OpCode::True, OpCode::False]);

        assert_eq!(
            disassemble_chunk(&chunk, "literals").unwrap(),
            concat!(
                "== literals ==\n\n",
                "0000  1:1    NULL\n",
                "0001  1:2    TRUE\n",
                "0002  1:3    FALSE\n"
            )
        );
    }

    #[test]
    fn disassembles_arithmetic_opcodes() {
        let chunk = chunk_with(&[
            OpCode::Add,
            OpCode::Subtract,
            OpCode::Multiply,
            OpCode::Divide,
            OpCode::Modulo,
        ]);
        let output = disassemble_chunk(&chunk, "arithmetic").unwrap();

        for name in ["ADD", "SUBTRACT", "MULTIPLY", "DIVIDE", "MODULO"] {
            assert!(output.lines().any(|line| line.ends_with(name)));
        }
    }

    #[test]
    fn disassembles_comparison_opcodes() {
        let chunk = chunk_with(&[
            OpCode::Equal,
            OpCode::NotEqual,
            OpCode::Less,
            OpCode::LessEqual,
            OpCode::Greater,
            OpCode::GreaterEqual,
        ]);
        let output = disassemble_chunk(&chunk, "comparisons").unwrap();

        for name in [
            "EQUAL",
            "NOT_EQUAL",
            "LESS",
            "LESS_EQUAL",
            "GREATER",
            "GREATER_EQUAL",
        ] {
            assert!(output.lines().any(|line| line.ends_with(name)));
        }
    }

    #[test]
    fn disassembles_unary_opcodes() {
        let chunk = chunk_with(&[OpCode::Negate, OpCode::Not]);
        let output = disassemble_chunk(&chunk, "unary").unwrap();

        assert!(output.lines().any(|line| line.ends_with("NEGATE")));
        assert!(output.lines().any(|line| line.ends_with("NOT")));
    }

    #[test]
    fn disassembles_pop() {
        let chunk = chunk_with(&[OpCode::Pop]);
        assert!(disassemble_instruction(&chunk, 0).unwrap().ends_with("POP"));
    }

    #[test]
    fn disassembles_return() {
        let chunk = chunk_with(&[OpCode::Return]);
        assert!(disassemble_instruction(&chunk, 0)
            .unwrap()
            .ends_with("RETURN"));
    }

    #[test]
    fn includes_source_span() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Return, Span::new(42, 17));

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  42:17  RETURN"
        );
    }

    #[test]
    fn increments_instruction_offsets() {
        let chunk = chunk_with(&[OpCode::Null, OpCode::Pop, OpCode::Return]);
        let output = disassemble_chunk(&chunk, "offsets").unwrap();

        assert!(output.contains("0000  "));
        assert!(output.contains("0001  "));
        assert!(output.contains("0002  "));
    }

    #[test]
    fn rejects_invalid_constant_index() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Constant(ConstantIndex::new(7)), Span::new(1, 1));

        assert_eq!(
            disassemble_chunk(&chunk, "invalid"),
            Err(DisassembleError::InvalidConstantIndex {
                offset: 0,
                index: 7,
                constant_count: 0,
            })
        );
    }

    #[test]
    fn rejects_invalid_instruction_offset() {
        let chunk = Chunk::new();

        assert_eq!(
            disassemble_instruction(&chunk, 3),
            Err(DisassembleError::InvalidInstructionOffset {
                offset: 3,
                instruction_count: 0,
            })
        );
    }

    #[test]
    fn disassembles_define_global() {
        let mut chunk = Chunk::new();
        let name = chunk
            .add_constant(Value::String("answer".to_owned()))
            .unwrap();
        chunk.write_instruction(OpCode::DefineGlobal(name), Span::new(1, 5));

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:5    DEFINE_GLOBAL  0    \"answer\""
        );
    }

    #[test]
    fn disassembles_get_global() {
        let mut chunk = Chunk::new();
        let name = chunk
            .add_constant(Value::String("answer".to_owned()))
            .unwrap();
        chunk.write_instruction(OpCode::GetGlobal(name), Span::new(2, 1));

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  2:1    GET_GLOBAL     0    \"answer\""
        );
    }

    #[test]
    fn disassembles_set_global() {
        let mut chunk = Chunk::new();
        let name = chunk
            .add_constant(Value::String("answer".to_owned()))
            .unwrap();
        chunk.write_instruction(OpCode::SetGlobal(name), Span::new(3, 1));

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  3:1    SET_GLOBAL     0    \"answer\""
        );
    }

    #[test]
    fn disassembles_get_local_without_constant_lookup() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), Span::new(1, 1));

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:1    GET_LOCAL      0"
        );
    }

    #[test]
    fn disassembles_set_local_without_constant_lookup() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::SetLocal(LocalSlot::new(1)), Span::new(1, 5));

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:5    SET_LOCAL      1"
        );
    }

    #[test]
    fn rejects_missing_global_name_constant() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::GetGlobal(ConstantIndex::new(4)), Span::new(1, 1));

        assert_eq!(
            disassemble_instruction(&chunk, 0),
            Err(DisassembleError::InvalidConstantIndex {
                offset: 0,
                index: 4,
                constant_count: 0,
            })
        );
    }

    #[test]
    fn rejects_non_string_global_name_without_panicking() {
        let mut chunk = Chunk::new();
        let name = chunk.add_constant(Value::Int(42)).unwrap();
        chunk.write_instruction(OpCode::DefineGlobal(name), Span::new(1, 1));

        assert_eq!(
            disassemble_instruction(&chunk, 0),
            Err(DisassembleError::InvalidGlobalName {
                offset: 0,
                index: 0,
            })
        );
    }

    #[test]
    fn output_is_deterministic() {
        let mut chunk = Chunk::new();
        let index = chunk
            .add_constant(Value::String("hanlin".to_owned()))
            .unwrap();
        chunk.write_instruction(OpCode::Constant(index), Span::new(3, 4));
        chunk.write_instruction(OpCode::Return, Span::new(3, 10));

        let first = disassemble_chunk(&chunk, "main").unwrap();
        let second = disassemble_chunk(&chunk, "main").unwrap();

        assert_eq!(first, second);
        assert_eq!(
            first,
            concat!(
                "== main ==\n\n",
                "0000  3:4    CONSTANT       0    \"hanlin\"\n",
                "0001  3:10   RETURN\n"
            )
        );
    }
}

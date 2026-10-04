//! Human-readable formatting for Hanlin bytecode chunks.
//!
//! The disassembler is debugging infrastructure for tests, future tracing,
//! and future CLI tooling. It formats bytecode but does not execute it.

use std::fmt;
use std::fmt::Write;

use super::opcode::{resolve_jump_target, JumpDirection};
use super::{
    AggregateCount, Arity, Chunk, ConstantIndex, JumpOffset, LocalSlot, OpCode, UpvalueDescriptor,
    UpvalueIndex, Value,
};

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
    InvalidClosureFunction {
        offset: usize,
        index: u32,
    },
    InvalidJumpTarget {
        offset: usize,
        opcode: OpCode,
        target: Option<usize>,
        instruction_count: usize,
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
            Self::InvalidClosureFunction { offset, index } => write!(
                f,
                "instruction {offset} references constant index {index} as a closure function, but it is not a function"
            ),
            Self::InvalidJumpTarget {
                offset,
                opcode,
                target,
                instruction_count,
            } => {
                write!(f, "invalid {opcode:?} target at instruction {offset}")?;
                if let Some(target) = target {
                    write!(
                        f,
                        ": target {target} is outside the chunk's {instruction_count} instructions"
                    )
                } else {
                    write!(f, ": target arithmetic overflowed or underflowed")
                }
            }
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
        OpCode::Closure(index) => format_closure_instruction(chunk, offset, &span, name, index),
        OpCode::GetUpvalue(index) | OpCode::SetUpvalue(index) => {
            Ok(format_upvalue_instruction(offset, &span, name, index))
        }
        OpCode::CloseUpvalue(slot) => Ok(format_local_instruction(offset, &span, name, slot)),
        OpCode::BuildArray(count) | OpCode::BuildMap(count) => {
            Ok(format_aggregate_instruction(offset, &span, name, count))
        }
        OpCode::Call(arity) => Ok(format_arity_instruction(offset, &span, name, arity)),
        OpCode::Jump(jump_offset) | OpCode::JumpIfFalse(jump_offset) => format_jump_instruction(
            chunk,
            offset,
            &span,
            name,
            opcode,
            jump_offset,
            JumpDirection::Forward,
        ),
        OpCode::Loop(jump_offset) => format_jump_instruction(
            chunk,
            offset,
            &span,
            name,
            opcode,
            jump_offset,
            JumpDirection::Backward,
        ),
        _ => Ok(format!("{offset:04}  {span:<6} {name}")),
    }
}

fn format_local_instruction(offset: usize, span: &str, name: &str, slot: LocalSlot) -> String {
    format!("{offset:04}  {span:<6} {name:<14} {}", slot.as_u16())
}

fn format_arity_instruction(offset: usize, span: &str, name: &str, arity: Arity) -> String {
    format!("{offset:04}  {span:<6} {name:<14} {}", arity.as_u8())
}

fn format_aggregate_instruction(
    offset: usize,
    span: &str,
    name: &str,
    count: AggregateCount,
) -> String {
    format!("{offset:04}  {span:<6} {name:<14} {}", count.as_u16())
}

fn format_upvalue_instruction(
    offset: usize,
    span: &str,
    name: &str,
    index: UpvalueIndex,
) -> String {
    format!("{offset:04}  {span:<6} {name:<14} {}", index.as_u8())
}

fn format_closure_instruction(
    chunk: &Chunk,
    offset: usize,
    span: &str,
    name: &str,
    index: ConstantIndex,
) -> Result<String, DisassembleError> {
    let value = chunk
        .constant(index)
        .ok_or(DisassembleError::InvalidConstantIndex {
            offset,
            index: index.as_u32(),
            constant_count: chunk.constants().len(),
        })?;
    let Value::Function(function) = value else {
        return Err(DisassembleError::InvalidClosureFunction {
            offset,
            index: index.as_u32(),
        });
    };
    let mut output = format!(
        "{offset:04}  {span:<6} {name:<14} {:<4} {}",
        index.as_u32(),
        value
    );
    for descriptor in function.upvalues() {
        match descriptor {
            UpvalueDescriptor::Local(slot) => {
                write!(output, "\n               local {}", slot.as_u16())
                    .expect("writing to a String cannot fail");
            }
            UpvalueDescriptor::Upvalue(index) => {
                write!(output, "\n               upvalue {}", index.as_u8())
                    .expect("writing to a String cannot fail");
            }
        }
    }
    Ok(output)
}

fn format_jump_instruction(
    chunk: &Chunk,
    offset: usize,
    span: &str,
    name: &str,
    opcode: OpCode,
    jump_offset: JumpOffset,
    direction: JumpDirection,
) -> Result<String, DisassembleError> {
    let target = resolve_jump_target(offset, jump_offset, direction, chunk.instructions().len())
        .map_err(|error| DisassembleError::InvalidJumpTarget {
            offset,
            opcode,
            target: error.target,
            instruction_count: chunk.instructions().len(),
        })?;

    Ok(format!(
        "{offset:04}  {span:<6} {name:<16} {} -> {target}",
        jump_offset.as_u16()
    ))
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
        OpCode::Jump(_) => "JUMP",
        OpCode::JumpIfFalse(_) => "JUMP_IF_FALSE",
        OpCode::Loop(_) => "LOOP",
        OpCode::Closure(_) => "CLOSURE",
        OpCode::GetUpvalue(_) => "GET_UPVALUE",
        OpCode::SetUpvalue(_) => "SET_UPVALUE",
        OpCode::CloseUpvalue(_) => "CLOSE_UPVALUE",
        OpCode::BuildArray(_) => "BUILD_ARRAY",
        OpCode::BuildMap(_) => "BUILD_MAP",
        OpCode::GetIndex => "GET_INDEX",
        OpCode::SetIndex => "SET_INDEX",
        OpCode::Call(_) => "CALL",
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
    use std::rc::Rc;

    use super::{disassemble_chunk, disassemble_instruction, DisassembleError};
    use crate::error::Span;
    use crate::vm::{
        AggregateCount, Arity, Chunk, ConstantIndex, Function, JumpOffset, LocalSlot, OpCode,
        UpvalueDescriptor, UpvalueIndex, Value,
    };

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
    fn disassembles_function_constant_without_nested_chunk_dump() {
        let mut chunk = Chunk::new();
        let function = Value::Function(Rc::new(Function::new("add", Arity::new(2), Chunk::new())));
        let index = chunk.add_constant(function).unwrap();
        chunk.write_instruction(OpCode::Constant(index), Span::new(1, 1));

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:1    CONSTANT       0    <fn add>"
        );
    }

    #[test]
    fn disassembles_call_arity() {
        let chunk = chunk_with(&[OpCode::Call(Arity::new(2))]);

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:1    CALL           2"
        );
    }

    #[test]
    fn disassembles_closure_with_deterministic_capture_metadata() {
        let mut chunk = Chunk::new();
        let function = Function::with_upvalues(
            "inner",
            Arity::new(0),
            Chunk::new(),
            vec![
                UpvalueDescriptor::Local(LocalSlot::new(2)),
                UpvalueDescriptor::Upvalue(UpvalueIndex::new(1)),
            ],
        );
        let index = chunk
            .add_constant(Value::Function(Rc::new(function)))
            .unwrap();
        chunk.write_instruction(OpCode::Closure(index), Span::new(4, 5));

        let expected = concat!(
            "0000  4:5    CLOSURE        0    <fn inner>\n",
            "               local 2\n",
            "               upvalue 1"
        );
        assert_eq!(disassemble_instruction(&chunk, 0).unwrap(), expected);
        assert_eq!(disassemble_instruction(&chunk, 0).unwrap(), expected);
    }

    #[test]
    fn disassembles_get_and_set_upvalue() {
        let chunk = chunk_with(&[
            OpCode::GetUpvalue(UpvalueIndex::new(1)),
            OpCode::SetUpvalue(UpvalueIndex::new(2)),
        ]);

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:1    GET_UPVALUE    1"
        );
        assert_eq!(
            disassemble_instruction(&chunk, 1).unwrap(),
            "0001  1:2    SET_UPVALUE    2"
        );
    }

    #[test]
    fn disassembles_close_upvalue() {
        let chunk = chunk_with(&[OpCode::CloseUpvalue(LocalSlot::new(3))]);

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:1    CLOSE_UPVALUE  3"
        );
    }

    #[test]
    fn rejects_non_function_closure_constant() {
        let mut chunk = Chunk::new();
        let index = chunk.add_constant(Value::Int(42)).unwrap();
        chunk.write_instruction(OpCode::Closure(index), Span::new(1, 1));

        assert_eq!(
            disassemble_instruction(&chunk, 0),
            Err(DisassembleError::InvalidClosureFunction {
                offset: 0,
                index: 0,
            })
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
    fn disassembles_forward_jump_with_resolved_target() {
        let chunk = chunk_with(&[
            OpCode::Jump(JumpOffset::new(2)),
            OpCode::Null,
            OpCode::Null,
            OpCode::Return,
        ]);

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:1    JUMP             2 -> 3"
        );
    }

    #[test]
    fn disassembles_conditional_jump_with_resolved_target() {
        let chunk = chunk_with(&[
            OpCode::True,
            OpCode::JumpIfFalse(JumpOffset::new(1)),
            OpCode::Null,
            OpCode::Return,
        ]);

        assert_eq!(
            disassemble_instruction(&chunk, 1).unwrap(),
            "0001  1:2    JUMP_IF_FALSE    1 -> 3"
        );
    }

    #[test]
    fn disassembles_loop_with_resolved_target() {
        let chunk = chunk_with(&[
            OpCode::Null,
            OpCode::Return,
            OpCode::Loop(JumpOffset::new(2)),
        ]);

        assert_eq!(
            disassemble_instruction(&chunk, 2).unwrap(),
            "0002  1:3    LOOP             2 -> 1"
        );
    }

    #[test]
    fn rejects_invalid_jump_target() {
        let opcode = OpCode::Jump(JumpOffset::new(1));
        let chunk = chunk_with(&[opcode, OpCode::Return]);

        assert_eq!(
            disassemble_instruction(&chunk, 0),
            Err(DisassembleError::InvalidJumpTarget {
                offset: 0,
                opcode,
                target: Some(2),
                instruction_count: 2,
            })
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

    #[test]
    fn disassembles_aggregate_build_counts() {
        let chunk = chunk_with(&[
            OpCode::BuildArray(AggregateCount::new(3)),
            OpCode::BuildMap(AggregateCount::new(2)),
        ]);

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:1    BUILD_ARRAY    3"
        );
        assert_eq!(
            disassemble_instruction(&chunk, 1).unwrap(),
            "0001  1:2    BUILD_MAP      2"
        );
    }

    #[test]
    fn disassembles_index_instructions() {
        let chunk = chunk_with(&[OpCode::GetIndex, OpCode::SetIndex]);

        assert_eq!(
            disassemble_instruction(&chunk, 0).unwrap(),
            "0000  1:1    GET_INDEX"
        );
        assert_eq!(
            disassemble_instruction(&chunk, 1).unwrap(),
            "0001  1:2    SET_INDEX"
        );
    }
}

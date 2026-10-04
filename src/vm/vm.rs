//! Execution engine for Hanlin bytecode.
//!
//! VM-03 intentionally executes only constants, literal values, stack pops,
//! and returns. Compilation and all other bytecode behavior remain separate
//! future work.

use std::fmt;

use crate::error::Span;

use super::{Chunk, OpCode, Value};

/// A structured failure encountered while executing Hanlin bytecode.
#[derive(Clone, Debug, PartialEq)]
pub enum VmError {
    InvalidConstantReference {
        instruction_offset: usize,
        index: u32,
        constant_count: usize,
        span: Span,
    },
    StackUnderflow {
        instruction_offset: usize,
        opcode: OpCode,
        span: Span,
    },
    InstructionPointerOutOfBounds {
        instruction_pointer: usize,
        instruction_count: usize,
        last_span: Option<Span>,
    },
    UnsupportedOpcode {
        instruction_offset: usize,
        opcode: OpCode,
        span: Span,
    },
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConstantReference {
                instruction_offset,
                index,
                constant_count,
                span,
            } => write!(
                f,
                "instruction {instruction_offset} at {span} references constant index {index}, but the chunk contains {constant_count} constants"
            ),
            Self::StackUnderflow {
                instruction_offset,
                opcode,
                span,
            } => write!(
                f,
                "stack underflow while executing {opcode:?} at instruction {instruction_offset} ({span})"
            ),
            Self::InstructionPointerOutOfBounds {
                instruction_pointer,
                instruction_count,
                last_span,
            } => {
                write!(
                    f,
                    "instruction pointer {instruction_pointer} is out of bounds for a chunk with {instruction_count} instructions"
                )?;
                if let Some(span) = last_span {
                    write!(f, " after instruction at {span}")?;
                }
                Ok(())
            }
            Self::UnsupportedOpcode {
                instruction_offset,
                opcode,
                span,
            } => write!(
                f,
                "unsupported opcode {opcode:?} at instruction {instruction_offset} ({span})"
            ),
        }
    }
}

impl std::error::Error for VmError {}

/// The basic stack virtual machine for Hanlin bytecode.
///
/// The operand stack is private and belongs to one execution. Both it and the
/// instruction pointer are reset before and after every [`run`](Self::run), so
/// one `Vm` can safely execute independent chunks without retaining values.
#[derive(Debug)]
pub struct Vm {
    stack: Vec<Value>,
    instruction_pointer: usize,
}

impl Vm {
    pub const fn new() -> Self {
        Self {
            stack: Vec::new(),
            instruction_pointer: 0,
        }
    }

    /// Executes `chunk` until `Return` or a structured [`VmError`].
    ///
    /// `Return` yields the top stack value. If the stack is empty, it yields
    /// [`Value::Null`]. Execution state is cleared on both success and failure.
    pub fn run(&mut self, chunk: &Chunk) -> Result<Value, VmError> {
        self.reset();
        let result = self.execute(chunk);
        self.reset();
        result
    }

    fn execute(&mut self, chunk: &Chunk) -> Result<Value, VmError> {
        let mut last_span = None;

        loop {
            let instruction_offset = self.instruction_pointer;
            let instruction = chunk.instruction(instruction_offset).copied().ok_or(
                VmError::InstructionPointerOutOfBounds {
                    instruction_pointer: self.instruction_pointer,
                    instruction_count: chunk.instructions().len(),
                    last_span,
                },
            )?;

            self.instruction_pointer += 1;
            let span = instruction.span();
            last_span = Some(span);

            match instruction.opcode() {
                OpCode::Constant(index) => {
                    let value = chunk.constant(index).cloned().ok_or(
                        VmError::InvalidConstantReference {
                            instruction_offset,
                            index: index.as_u32(),
                            constant_count: chunk.constants().len(),
                            span,
                        },
                    )?;
                    self.stack.push(value);
                }
                OpCode::Null => self.stack.push(Value::Null),
                OpCode::True => self.stack.push(Value::Bool(true)),
                OpCode::False => self.stack.push(Value::Bool(false)),
                OpCode::Pop => {
                    self.stack.pop().ok_or(VmError::StackUnderflow {
                        instruction_offset,
                        opcode: OpCode::Pop,
                        span,
                    })?;
                }
                OpCode::Return => return Ok(self.stack.pop().unwrap_or(Value::Null)),
                opcode => {
                    return Err(VmError::UnsupportedOpcode {
                        instruction_offset,
                        opcode,
                        span,
                    });
                }
            }
        }
    }

    fn reset(&mut self) {
        self.stack.clear();
        self.instruction_pointer = 0;
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{Vm, VmError};
    use crate::error::Span;
    use crate::vm::{Chunk, ConstantIndex, OpCode, Value};

    const SPAN: Span = Span { line: 1, col: 1 };

    fn return_chunk(opcode: OpCode) -> Chunk {
        let mut chunk = Chunk::new();
        chunk.write_instruction(opcode, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        chunk
    }

    fn constant_chunk(value: Value) -> Chunk {
        let mut chunk = Chunk::new();
        let index = chunk.add_constant(value).unwrap();
        chunk.write_instruction(OpCode::Constant(index), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        chunk
    }

    #[test]
    fn creates_empty_vm() {
        let vm = Vm::new();

        assert!(vm.stack.is_empty());
        assert_eq!(vm.instruction_pointer, 0);
    }

    #[test]
    fn executes_integer_constant() {
        assert_eq!(
            Vm::new().run(&constant_chunk(Value::Int(42))),
            Ok(Value::Int(42))
        );
    }

    #[test]
    fn executes_float_constant() {
        assert_eq!(
            Vm::new().run(&constant_chunk(Value::Float(3.5))),
            Ok(Value::Float(3.5))
        );
    }

    #[test]
    fn executes_string_constant() {
        assert_eq!(
            Vm::new().run(&constant_chunk(Value::String("hello".to_owned()))),
            Ok(Value::String("hello".to_owned()))
        );
    }

    #[test]
    fn executes_null() {
        assert_eq!(Vm::new().run(&return_chunk(OpCode::Null)), Ok(Value::Null));
    }

    #[test]
    fn executes_true() {
        assert_eq!(
            Vm::new().run(&return_chunk(OpCode::True)),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn executes_false() {
        assert_eq!(
            Vm::new().run(&return_chunk(OpCode::False)),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn pop_removes_top_value() {
        let mut chunk = Chunk::new();
        let ten = chunk.add_constant(Value::Int(10)).unwrap();
        let twenty = chunk.add_constant(Value::Int(20)).unwrap();
        chunk.write_instruction(OpCode::Constant(ten), SPAN);
        chunk.write_instruction(OpCode::Constant(twenty), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(10)));
    }

    #[test]
    fn pop_on_empty_stack_returns_underflow() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Pop, Span::new(4, 2));

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 0,
                opcode: OpCode::Pop,
                span: Span::new(4, 2),
            })
        );
    }

    #[test]
    fn return_yields_top_value() {
        assert_eq!(
            Vm::new().run(&return_chunk(OpCode::True)),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn return_with_empty_stack_yields_null() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Null));
    }

    #[test]
    fn sequential_constants_return_last_value() {
        let mut chunk = Chunk::new();
        let first = chunk.add_constant(Value::Int(10)).unwrap();
        let second = chunk.add_constant(Value::Int(20)).unwrap();
        chunk.write_instruction(OpCode::Constant(first), SPAN);
        chunk.write_instruction(OpCode::Constant(second), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(20)));
    }

    #[test]
    fn invalid_constant_reference_returns_error() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Constant(ConstantIndex::new(9)), Span::new(7, 3));

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidConstantReference {
                instruction_offset: 0,
                index: 9,
                constant_count: 0,
                span: Span::new(7, 3),
            })
        );
    }

    #[test]
    fn unsupported_opcode_returns_error() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Add, Span::new(8, 5));

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::UnsupportedOpcode {
                instruction_offset: 0,
                opcode: OpCode::Add,
                span: Span::new(8, 5),
            })
        );
    }

    #[test]
    fn state_resets_between_runs() {
        let mut first = Chunk::new();
        first.write_instruction(OpCode::True, SPAN);
        first.write_instruction(OpCode::Return, SPAN);

        let mut second = Chunk::new();
        second.write_instruction(OpCode::Return, SPAN);

        let mut vm = Vm::new();
        assert_eq!(vm.run(&first), Ok(Value::Bool(true)));
        assert_eq!(vm.run(&second), Ok(Value::Null));
        assert!(vm.stack.is_empty());
        assert_eq!(vm.instruction_pointer, 0);
    }

    #[test]
    fn incomplete_execution_returns_instruction_pointer_error() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::True, Span::new(5, 6));

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InstructionPointerOutOfBounds {
                instruction_pointer: 1,
                instruction_count: 1,
                last_span: Some(Span::new(5, 6)),
            })
        );
    }

    #[test]
    fn empty_chunk_returns_instruction_pointer_error() {
        assert_eq!(
            Vm::new().run(&Chunk::new()),
            Err(VmError::InstructionPointerOutOfBounds {
                instruction_pointer: 0,
                instruction_count: 0,
                last_span: None,
            })
        );
    }
}

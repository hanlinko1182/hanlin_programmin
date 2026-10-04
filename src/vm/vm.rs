//! Execution engine for Hanlin bytecode.
//!
//! The VM executes manually constructed foundational bytecode. Compilation
//! and higher-level runtime behavior remain separate future work.
//! Integer arithmetic is checked and reports [`VmError::IntegerOverflow`]
//! instead of depending on Rust's debug or release overflow behavior.

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
        needed: usize,
        available: usize,
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
    TypeError {
        instruction_offset: usize,
        opcode: OpCode,
        left: Value,
        right: Option<Value>,
        span: Span,
    },
    DivisionByZero {
        instruction_offset: usize,
        span: Span,
    },
    ModuloByZero {
        instruction_offset: usize,
        span: Span,
    },
    IntegerOverflow {
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
                needed,
                available,
                span,
            } => write!(
                f,
                "stack underflow while executing {opcode:?} at instruction {instruction_offset} ({span}): needed {needed} values, found {available}"
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
            Self::TypeError {
                instruction_offset,
                opcode,
                left,
                right,
                span,
            } => {
                write!(
                    f,
                    "invalid operand types for {opcode:?} at instruction {instruction_offset} ({span}): left={left:?}"
                )?;
                if let Some(right) = right {
                    write!(f, ", right={right:?}")?;
                }
                Ok(())
            }
            Self::DivisionByZero {
                instruction_offset,
                span,
            } => write!(
                f,
                "division by zero at instruction {instruction_offset} ({span})"
            ),
            Self::ModuloByZero {
                instruction_offset,
                span,
            } => write!(
                f,
                "modulo by zero at instruction {instruction_offset} ({span})"
            ),
            Self::IntegerOverflow {
                instruction_offset,
                opcode,
                span,
            } => write!(
                f,
                "integer overflow while executing {opcode:?} at instruction {instruction_offset} ({span})"
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
                        needed: 1,
                        available: 0,
                        span,
                    })?;
                }
                opcode @ (OpCode::Add
                | OpCode::Subtract
                | OpCode::Multiply
                | OpCode::Divide
                | OpCode::Modulo) => {
                    self.execute_binary(opcode, instruction_offset, span)?;
                }
                OpCode::Negate => self.execute_negate(instruction_offset, span)?,
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

    fn execute_binary(
        &mut self,
        opcode: OpCode,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let available = self.stack.len();
        if available < 2 {
            return Err(VmError::StackUnderflow {
                instruction_offset,
                opcode,
                needed: 2,
                available,
                span,
            });
        }

        let right = self.stack.pop().ok_or(VmError::StackUnderflow {
            instruction_offset,
            opcode,
            needed: 2,
            available,
            span,
        })?;
        let left = self.stack.pop().ok_or(VmError::StackUnderflow {
            instruction_offset,
            opcode,
            needed: 2,
            available,
            span,
        })?;
        let result = Self::calculate_binary(opcode, left, right, instruction_offset, span)?;
        self.stack.push(result);
        Ok(())
    }

    fn calculate_binary(
        opcode: OpCode,
        left: Value,
        right: Value,
        instruction_offset: usize,
        span: Span,
    ) -> Result<Value, VmError> {
        if opcode == OpCode::Add {
            match (left, right) {
                (Value::String(left), right) => {
                    return Ok(Value::String(format!("{left}{right}")));
                }
                (left, Value::String(right)) => {
                    return Ok(Value::String(format!("{left}{right}")));
                }
                (left, right) => {
                    return Self::calculate_numeric(opcode, left, right, instruction_offset, span);
                }
            }
        }

        Self::calculate_numeric(opcode, left, right, instruction_offset, span)
    }

    fn calculate_numeric(
        opcode: OpCode,
        left: Value,
        right: Value,
        instruction_offset: usize,
        span: Span,
    ) -> Result<Value, VmError> {
        match (left, right) {
            (Value::Int(left), Value::Int(right)) => match opcode {
                OpCode::Add => {
                    Self::checked_integer(left.checked_add(right), opcode, instruction_offset, span)
                }
                OpCode::Subtract => {
                    Self::checked_integer(left.checked_sub(right), opcode, instruction_offset, span)
                }
                OpCode::Multiply => {
                    Self::checked_integer(left.checked_mul(right), opcode, instruction_offset, span)
                }
                OpCode::Divide => {
                    if right == 0 {
                        Err(VmError::DivisionByZero {
                            instruction_offset,
                            span,
                        })
                    } else {
                        Ok(Value::Float(left as f64 / right as f64))
                    }
                }
                OpCode::Modulo => {
                    if right == 0 {
                        Err(VmError::ModuloByZero {
                            instruction_offset,
                            span,
                        })
                    } else {
                        Self::checked_integer(
                            left.checked_rem(right),
                            opcode,
                            instruction_offset,
                            span,
                        )
                    }
                }
                _ => Err(VmError::UnsupportedOpcode {
                    instruction_offset,
                    opcode,
                    span,
                }),
            },
            (Value::Int(left), Value::Float(right)) => {
                Self::calculate_float(opcode, left as f64, right, instruction_offset, span)
            }
            (Value::Float(left), Value::Int(right)) => {
                Self::calculate_float(opcode, left, right as f64, instruction_offset, span)
            }
            (Value::Float(left), Value::Float(right)) => {
                Self::calculate_float(opcode, left, right, instruction_offset, span)
            }
            (left, right) => Err(VmError::TypeError {
                instruction_offset,
                opcode,
                left,
                right: Some(right),
                span,
            }),
        }
    }

    fn calculate_float(
        opcode: OpCode,
        left: f64,
        right: f64,
        instruction_offset: usize,
        span: Span,
    ) -> Result<Value, VmError> {
        match opcode {
            OpCode::Add => Ok(Value::Float(left + right)),
            OpCode::Subtract => Ok(Value::Float(left - right)),
            OpCode::Multiply => Ok(Value::Float(left * right)),
            OpCode::Divide if right == 0.0 => Err(VmError::DivisionByZero {
                instruction_offset,
                span,
            }),
            OpCode::Divide => Ok(Value::Float(left / right)),
            OpCode::Modulo if right == 0.0 => Err(VmError::ModuloByZero {
                instruction_offset,
                span,
            }),
            OpCode::Modulo => Ok(Value::Float(left % right)),
            _ => Err(VmError::UnsupportedOpcode {
                instruction_offset,
                opcode,
                span,
            }),
        }
    }

    fn checked_integer(
        value: Option<i64>,
        opcode: OpCode,
        instruction_offset: usize,
        span: Span,
    ) -> Result<Value, VmError> {
        value.map(Value::Int).ok_or(VmError::IntegerOverflow {
            instruction_offset,
            opcode,
            span,
        })
    }

    fn execute_negate(&mut self, instruction_offset: usize, span: Span) -> Result<(), VmError> {
        let operand = self.stack.pop().ok_or(VmError::StackUnderflow {
            instruction_offset,
            opcode: OpCode::Negate,
            needed: 1,
            available: 0,
            span,
        })?;
        let result = match operand {
            Value::Int(value) => Self::checked_integer(
                value.checked_neg(),
                OpCode::Negate,
                instruction_offset,
                span,
            )?,
            Value::Float(value) => Value::Float(-value),
            operand => {
                return Err(VmError::TypeError {
                    instruction_offset,
                    opcode: OpCode::Negate,
                    left: operand,
                    right: None,
                    span,
                });
            }
        };
        self.stack.push(result);
        Ok(())
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

    fn binary_chunk(left: Value, right: Value, opcode: OpCode) -> Chunk {
        let mut chunk = Chunk::new();
        let left = chunk.add_constant(left).unwrap();
        let right = chunk.add_constant(right).unwrap();
        chunk.write_instruction(OpCode::Constant(left), SPAN);
        chunk.write_instruction(OpCode::Constant(right), SPAN);
        chunk.write_instruction(opcode, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        chunk
    }

    fn run_binary(left: Value, right: Value, opcode: OpCode) -> Result<Value, VmError> {
        Vm::new().run(&binary_chunk(left, right, opcode))
    }

    fn negate_chunk(value: Value) -> Chunk {
        let mut chunk = Chunk::new();
        let value = chunk.add_constant(value).unwrap();
        chunk.write_instruction(OpCode::Constant(value), SPAN);
        chunk.write_instruction(OpCode::Negate, SPAN);
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
                needed: 1,
                available: 0,
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
        chunk.write_instruction(OpCode::Equal, Span::new(8, 5));

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::UnsupportedOpcode {
                instruction_offset: 0,
                opcode: OpCode::Equal,
                span: Span::new(8, 5),
            })
        );
    }

    #[test]
    fn adds_integers() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Int(20), OpCode::Add),
            Ok(Value::Int(30))
        );
    }

    #[test]
    fn adds_floats() {
        assert_eq!(
            run_binary(Value::Float(1.5), Value::Float(2.5), OpCode::Add),
            Ok(Value::Float(4.0))
        );
    }

    #[test]
    fn adds_integer_and_float() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Float(2.5), OpCode::Add),
            Ok(Value::Float(12.5))
        );
    }

    #[test]
    fn adds_float_and_integer() {
        assert_eq!(
            run_binary(Value::Float(2.5), Value::Int(10), OpCode::Add),
            Ok(Value::Float(12.5))
        );
    }

    #[test]
    fn concatenates_strings_like_interpreter() {
        assert_eq!(
            run_binary(
                Value::String("hello ".to_owned()),
                Value::String("hanlin".to_owned()),
                OpCode::Add,
            ),
            Ok(Value::String("hello hanlin".to_owned()))
        );
    }

    #[test]
    fn concatenates_string_and_primitive_like_interpreter() {
        assert_eq!(
            run_binary(
                Value::String("value: ".to_owned()),
                Value::Int(42),
                OpCode::Add,
            ),
            Ok(Value::String("value: 42".to_owned()))
        );
        assert_eq!(
            run_binary(
                Value::Bool(true),
                Value::String("!".to_owned()),
                OpCode::Add,
            ),
            Ok(Value::String("true!".to_owned()))
        );
    }

    #[test]
    fn subtracts_integers() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Int(3), OpCode::Subtract),
            Ok(Value::Int(7))
        );
    }

    #[test]
    fn subtracts_mixed_numbers() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Float(2.5), OpCode::Subtract),
            Ok(Value::Float(7.5))
        );
        assert_eq!(
            run_binary(Value::Float(10.0), Value::Int(4), OpCode::Subtract),
            Ok(Value::Float(6.0))
        );
    }

    #[test]
    fn multiplies_integers() {
        assert_eq!(
            run_binary(Value::Int(4), Value::Int(5), OpCode::Multiply),
            Ok(Value::Int(20))
        );
    }

    #[test]
    fn multiplies_mixed_numbers() {
        assert_eq!(
            run_binary(Value::Float(2.5), Value::Int(4), OpCode::Multiply),
            Ok(Value::Float(10.0))
        );
    }

    #[test]
    fn integer_division_returns_float() {
        assert_eq!(
            run_binary(Value::Int(5), Value::Int(2), OpCode::Divide),
            Ok(Value::Float(2.5))
        );
    }

    #[test]
    fn divides_floats() {
        assert_eq!(
            run_binary(Value::Float(7.5), Value::Float(2.5), OpCode::Divide),
            Ok(Value::Float(3.0))
        );
        assert_eq!(
            run_binary(Value::Int(9), Value::Float(2.0), OpCode::Divide),
            Ok(Value::Float(4.5))
        );
        assert_eq!(
            run_binary(Value::Float(9.0), Value::Int(2), OpCode::Divide),
            Ok(Value::Float(4.5))
        );
    }

    #[test]
    fn integer_division_by_zero_returns_error() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Int(0), OpCode::Divide),
            Err(VmError::DivisionByZero {
                instruction_offset: 2,
                span: SPAN,
            })
        );
    }

    #[test]
    fn float_division_by_zero_returns_error() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Float(0.0), OpCode::Divide),
            Err(VmError::DivisionByZero {
                instruction_offset: 2,
                span: SPAN,
            })
        );
    }

    #[test]
    fn computes_integer_modulo() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Int(3), OpCode::Modulo),
            Ok(Value::Int(1))
        );
    }

    #[test]
    fn computes_float_modulo() {
        assert_eq!(
            run_binary(Value::Float(10.5), Value::Float(3.0), OpCode::Modulo),
            Ok(Value::Float(1.5))
        );
    }

    #[test]
    fn computes_mixed_numeric_modulo() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Float(3.0), OpCode::Modulo),
            Ok(Value::Float(1.0))
        );
        assert_eq!(
            run_binary(Value::Float(10.5), Value::Int(3), OpCode::Modulo),
            Ok(Value::Float(1.5))
        );
    }

    #[test]
    fn modulo_by_zero_returns_error() {
        assert_eq!(
            run_binary(Value::Float(10.0), Value::Int(0), OpCode::Modulo),
            Err(VmError::ModuloByZero {
                instruction_offset: 2,
                span: SPAN,
            })
        );
    }

    #[test]
    fn negates_integer() {
        assert_eq!(
            Vm::new().run(&negate_chunk(Value::Int(10))),
            Ok(Value::Int(-10))
        );
    }

    #[test]
    fn negates_float() {
        assert_eq!(
            Vm::new().run(&negate_chunk(Value::Float(3.14))),
            Ok(Value::Float(-3.14))
        );
    }

    #[test]
    fn negate_rejects_non_numeric_value() {
        assert_eq!(
            Vm::new().run(&negate_chunk(Value::String("no".to_owned()))),
            Err(VmError::TypeError {
                instruction_offset: 1,
                opcode: OpCode::Negate,
                left: Value::String("no".to_owned()),
                right: None,
                span: SPAN,
            })
        );
    }

    #[test]
    fn binary_arithmetic_rejects_invalid_types() {
        assert_eq!(
            run_binary(Value::Bool(true), Value::Int(1), OpCode::Add),
            Err(VmError::TypeError {
                instruction_offset: 2,
                opcode: OpCode::Add,
                left: Value::Bool(true),
                right: Some(Value::Int(1)),
                span: SPAN,
            })
        );
        assert!(matches!(
            run_binary(
                Value::String("hello".to_owned()),
                Value::Int(1),
                OpCode::Subtract,
            ),
            Err(VmError::TypeError {
                opcode: OpCode::Subtract,
                ..
            })
        ));
        assert!(matches!(
            run_binary(Value::Null, Value::Int(2), OpCode::Multiply),
            Err(VmError::TypeError {
                opcode: OpCode::Multiply,
                ..
            })
        ));
    }

    #[test]
    fn binary_operation_reports_missing_right_operand() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Add, SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 0,
                opcode: OpCode::Add,
                needed: 2,
                available: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn binary_operation_reports_missing_left_operand() {
        let mut chunk = Chunk::new();
        let right = chunk.add_constant(Value::Int(1)).unwrap();
        chunk.write_instruction(OpCode::Constant(right), SPAN);
        chunk.write_instruction(OpCode::Add, SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 1,
                opcode: OpCode::Add,
                needed: 2,
                available: 1,
                span: SPAN,
            })
        );
    }

    #[test]
    fn vm_is_reusable_after_arithmetic_error() {
        let invalid = binary_chunk(Value::Bool(true), Value::Int(1), OpCode::Add);
        let valid = binary_chunk(Value::Int(10), Value::Int(20), OpCode::Add);
        let mut vm = Vm::new();

        assert!(matches!(vm.run(&invalid), Err(VmError::TypeError { .. })));
        assert_eq!(vm.run(&valid), Ok(Value::Int(30)));
        assert!(vm.stack.is_empty());
        assert_eq!(vm.instruction_pointer, 0);
    }

    #[test]
    fn integer_addition_overflow_returns_error() {
        assert!(matches!(
            run_binary(Value::Int(i64::MAX), Value::Int(1), OpCode::Add),
            Err(VmError::IntegerOverflow {
                opcode: OpCode::Add,
                ..
            })
        ));
    }

    #[test]
    fn integer_subtraction_overflow_returns_error() {
        assert!(matches!(
            run_binary(Value::Int(i64::MIN), Value::Int(1), OpCode::Subtract),
            Err(VmError::IntegerOverflow {
                opcode: OpCode::Subtract,
                ..
            })
        ));
    }

    #[test]
    fn integer_multiplication_overflow_returns_error() {
        assert!(matches!(
            run_binary(Value::Int(i64::MAX), Value::Int(2), OpCode::Multiply),
            Err(VmError::IntegerOverflow {
                opcode: OpCode::Multiply,
                ..
            })
        ));
    }

    #[test]
    fn integer_modulo_overflow_returns_error() {
        assert!(matches!(
            run_binary(Value::Int(i64::MIN), Value::Int(-1), OpCode::Modulo),
            Err(VmError::IntegerOverflow {
                opcode: OpCode::Modulo,
                ..
            })
        ));
    }

    #[test]
    fn integer_negation_overflow_returns_error() {
        assert_eq!(
            Vm::new().run(&negate_chunk(Value::Int(i64::MIN))),
            Err(VmError::IntegerOverflow {
                instruction_offset: 1,
                opcode: OpCode::Negate,
                span: SPAN,
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

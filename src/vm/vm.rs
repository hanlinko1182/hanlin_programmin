//! Execution engine for Hanlin bytecode.
//!
//! The VM executes manually constructed foundational bytecode. Compilation
//! and higher-level runtime behavior remain separate future work.
//! Integer arithmetic is checked and reports [`VmError::IntegerOverflow`]
//! instead of depending on Rust's debug or release overflow behavior.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use crate::error::Span;

use super::{Chunk, LocalSlot, OpCode, Value};

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
    InvalidLocalSlot {
        instruction_offset: usize,
        slot: LocalSlot,
        stack_len: usize,
        span: Span,
    },
    InvalidGlobalName {
        instruction_offset: usize,
        index: u32,
        value: Value,
        span: Span,
    },
    UndefinedGlobal {
        instruction_offset: usize,
        name: String,
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
            Self::InvalidLocalSlot {
                instruction_offset,
                slot,
                stack_len,
                span,
            } => write!(
                f,
                "local slot {} is invalid for a stack with {stack_len} values at instruction {instruction_offset} ({span})",
                slot.as_u16()
            ),
            Self::InvalidGlobalName {
                instruction_offset,
                index,
                value,
                span,
            } => write!(
                f,
                "global name at constant index {index} is not a string at instruction {instruction_offset} ({span}): {value:?}"
            ),
            Self::UndefinedGlobal {
                instruction_offset,
                name,
                span,
            } => write!(
                f,
                "undefined global '{name}' at instruction {instruction_offset} ({span})"
            ),
        }
    }
}

impl std::error::Error for VmError {}

/// The basic stack virtual machine for Hanlin bytecode.
///
/// The operand stack is private and belongs to one execution. Both it and the
/// instruction pointer are reset before and after every [`run`](Self::run).
/// Global bindings are intentionally retained across runs for future REPL use.
#[derive(Debug)]
pub struct Vm {
    stack: Vec<Value>,
    instruction_pointer: usize,
    globals: HashMap<String, Value>,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            instruction_pointer: 0,
            globals: HashMap::new(),
        }
    }

    /// Executes `chunk` until `Return` or a structured [`VmError`].
    ///
    /// `Return` yields the top stack value. If the stack is empty, it yields
    /// [`Value::Null`]. The stack and instruction pointer are cleared on both
    /// success and failure, while global bindings persist.
    pub fn run(&mut self, chunk: &Chunk) -> Result<Value, VmError> {
        self.reset_execution_state();
        let result = self.execute(chunk);
        self.reset_execution_state();
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
                opcode @ (OpCode::Equal
                | OpCode::NotEqual
                | OpCode::Less
                | OpCode::LessEqual
                | OpCode::Greater
                | OpCode::GreaterEqual) => {
                    self.execute_comparison(opcode, instruction_offset, span)?;
                }
                OpCode::Not => self.execute_not(instruction_offset, span)?,
                OpCode::GetLocal(slot) => {
                    self.get_local(slot, instruction_offset, span)?;
                }
                OpCode::SetLocal(slot) => {
                    self.set_local(slot, instruction_offset, span)?;
                }
                OpCode::DefineGlobal(index) => {
                    self.define_global(chunk, index, instruction_offset, span)?;
                }
                OpCode::GetGlobal(index) => {
                    self.get_global(chunk, index, instruction_offset, span)?;
                }
                OpCode::SetGlobal(index) => {
                    self.set_global(chunk, index, instruction_offset, span)?;
                }
                OpCode::Return => return Ok(self.stack.pop().unwrap_or(Value::Null)),
            }
        }
    }

    fn execute_binary(
        &mut self,
        opcode: OpCode,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let (left, right) = self.pop_binary_operands(opcode, instruction_offset, span)?;
        let result = Self::calculate_binary(opcode, left, right, instruction_offset, span)?;
        self.stack.push(result);
        Ok(())
    }

    fn pop_binary_operands(
        &mut self,
        opcode: OpCode,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(Value, Value), VmError> {
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
        Ok((left, right))
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
        let operand = self.pop_unary_operand(OpCode::Negate, instruction_offset, span)?;
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

    fn execute_comparison(
        &mut self,
        opcode: OpCode,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let (left, right) = self.pop_binary_operands(opcode, instruction_offset, span)?;
        let result = match opcode {
            OpCode::Equal => Self::values_equal(&left, &right),
            OpCode::NotEqual => !Self::values_equal(&left, &right),
            OpCode::Less | OpCode::LessEqual | OpCode::Greater | OpCode::GreaterEqual => {
                Self::compare_ordering(opcode, left, right, instruction_offset, span)?
            }
            _ => {
                return Err(VmError::UnsupportedOpcode {
                    instruction_offset,
                    opcode,
                    span,
                });
            }
        };
        self.stack.push(Value::Bool(result));
        Ok(())
    }

    fn values_equal(left: &Value, right: &Value) -> bool {
        match (left, right) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(left), Value::Bool(right)) => left == right,
            (Value::Int(left), Value::Int(right)) => left == right,
            (Value::Float(left), Value::Float(right)) => left == right,
            (Value::Int(left), Value::Float(right)) => *left as f64 == *right,
            (Value::Float(left), Value::Int(right)) => *left == *right as f64,
            (Value::String(left), Value::String(right)) => left == right,
            _ => false,
        }
    }

    fn compare_ordering(
        opcode: OpCode,
        left: Value,
        right: Value,
        instruction_offset: usize,
        span: Span,
    ) -> Result<bool, VmError> {
        let ordering = match (&left, &right) {
            (Value::Int(left), Value::Int(right)) => left.partial_cmp(right),
            (Value::Float(left), Value::Float(right)) => left.partial_cmp(right),
            (Value::Int(left), Value::Float(right)) => (*left as f64).partial_cmp(right),
            (Value::Float(left), Value::Int(right)) => left.partial_cmp(&(*right as f64)),
            (Value::String(left), Value::String(right)) => left.partial_cmp(right),
            _ => {
                return Err(VmError::TypeError {
                    instruction_offset,
                    opcode,
                    left,
                    right: Some(right),
                    span,
                });
            }
        };

        Self::ordering_result(opcode, ordering).ok_or(VmError::UnsupportedOpcode {
            instruction_offset,
            opcode,
            span,
        })
    }

    fn ordering_result(opcode: OpCode, ordering: Option<Ordering>) -> Option<bool> {
        match opcode {
            OpCode::Less => Some(ordering == Some(Ordering::Less)),
            OpCode::LessEqual => Some(matches!(ordering, Some(Ordering::Less | Ordering::Equal))),
            OpCode::Greater => Some(ordering == Some(Ordering::Greater)),
            OpCode::GreaterEqual => Some(matches!(
                ordering,
                Some(Ordering::Greater | Ordering::Equal)
            )),
            _ => None,
        }
    }

    fn execute_not(&mut self, instruction_offset: usize, span: Span) -> Result<(), VmError> {
        let operand = self.pop_unary_operand(OpCode::Not, instruction_offset, span)?;
        self.stack.push(Value::Bool(!operand.is_truthy()));
        Ok(())
    }

    fn pop_unary_operand(
        &mut self,
        opcode: OpCode,
        instruction_offset: usize,
        span: Span,
    ) -> Result<Value, VmError> {
        self.stack.pop().ok_or(VmError::StackUnderflow {
            instruction_offset,
            opcode,
            needed: 1,
            available: 0,
            span,
        })
    }

    fn get_local(
        &mut self,
        slot: LocalSlot,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let stack_index = self.resolve_local_index(slot, instruction_offset, span)?;
        let value = self.stack[stack_index].clone();
        self.stack.push(value);
        Ok(())
    }

    fn set_local(
        &mut self,
        slot: LocalSlot,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let stack_index = self.resolve_local_index(slot, instruction_offset, span)?;
        let value = self
            .stack
            .last()
            .cloned()
            .ok_or(VmError::InvalidLocalSlot {
                instruction_offset,
                slot,
                stack_len: self.stack.len(),
                span,
            })?;
        self.stack[stack_index] = value;
        Ok(())
    }

    /// Resolves a frame-relative local slot to its absolute stack index.
    ///
    /// VM-07 has a single implicit frame starting at zero. Future call frames
    /// can supply a different base here without changing either local opcode.
    fn resolve_local_index(
        &self,
        slot: LocalSlot,
        instruction_offset: usize,
        span: Span,
    ) -> Result<usize, VmError> {
        let frame_base = 0usize;
        let stack_len = self.stack.len();
        frame_base
            .checked_add(slot.as_usize())
            .filter(|&stack_index| stack_index < stack_len)
            .ok_or(VmError::InvalidLocalSlot {
                instruction_offset,
                slot,
                stack_len,
                span,
            })
    }

    fn define_global(
        &mut self,
        chunk: &Chunk,
        index: super::ConstantIndex,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let name = Self::global_name(chunk, index, instruction_offset, span)?;
        let value =
            self.pop_unary_operand(OpCode::DefineGlobal(index), instruction_offset, span)?;
        self.globals.insert(name, value);
        Ok(())
    }

    fn get_global(
        &mut self,
        chunk: &Chunk,
        index: super::ConstantIndex,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let name = Self::global_name(chunk, index, instruction_offset, span)?;
        let value = self
            .globals
            .get(&name)
            .cloned()
            .ok_or_else(|| VmError::UndefinedGlobal {
                instruction_offset,
                name: name.clone(),
                span,
            })?;
        self.stack.push(value);
        Ok(())
    }

    fn set_global(
        &mut self,
        chunk: &Chunk,
        index: super::ConstantIndex,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let name = Self::global_name(chunk, index, instruction_offset, span)?;
        let value = self.stack.last().cloned().ok_or(VmError::StackUnderflow {
            instruction_offset,
            opcode: OpCode::SetGlobal(index),
            needed: 1,
            available: 0,
            span,
        })?;
        let binding = self
            .globals
            .get_mut(&name)
            .ok_or_else(|| VmError::UndefinedGlobal {
                instruction_offset,
                name: name.clone(),
                span,
            })?;
        *binding = value;
        Ok(())
    }

    fn global_name(
        chunk: &Chunk,
        index: super::ConstantIndex,
        instruction_offset: usize,
        span: Span,
    ) -> Result<String, VmError> {
        let value = chunk
            .constant(index)
            .ok_or(VmError::InvalidConstantReference {
                instruction_offset,
                index: index.as_u32(),
                constant_count: chunk.constants().len(),
                span,
            })?;
        match value {
            Value::String(name) => Ok(name.clone()),
            value => Err(VmError::InvalidGlobalName {
                instruction_offset,
                index: index.as_u32(),
                value: value.clone(),
                span,
            }),
        }
    }

    fn reset_execution_state(&mut self) {
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
    use crate::vm::{Chunk, ConstantIndex, LocalSlot, OpCode, Value};

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

    fn unary_chunk(value: Value, opcode: OpCode) -> Chunk {
        let mut chunk = Chunk::new();
        let value = chunk.add_constant(value).unwrap();
        chunk.write_instruction(OpCode::Constant(value), SPAN);
        chunk.write_instruction(opcode, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        chunk
    }

    fn negate_chunk(value: Value) -> Chunk {
        unary_chunk(value, OpCode::Negate)
    }

    fn run_unary(value: Value, opcode: OpCode) -> Result<Value, VmError> {
        Vm::new().run(&unary_chunk(value, opcode))
    }

    fn define_global_chunk(name: &str, value: Value) -> Chunk {
        let mut chunk = Chunk::new();
        let name = chunk.add_constant(Value::String(name.to_owned())).unwrap();
        let value = chunk.add_constant(value).unwrap();
        chunk.write_instruction(OpCode::Constant(value), SPAN);
        chunk.write_instruction(OpCode::DefineGlobal(name), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        chunk
    }

    fn get_global_chunk(name: &str) -> Chunk {
        let mut chunk = Chunk::new();
        let name = chunk.add_constant(Value::String(name.to_owned())).unwrap();
        chunk.write_instruction(OpCode::GetGlobal(name), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        chunk
    }

    fn set_global_chunk(name: &str, value: Value) -> Chunk {
        let mut chunk = Chunk::new();
        let name = chunk.add_constant(Value::String(name.to_owned())).unwrap();
        let value = chunk.add_constant(value).unwrap();
        chunk.write_instruction(OpCode::Constant(value), SPAN);
        chunk.write_instruction(OpCode::SetGlobal(name), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        chunk
    }

    fn chunk_with_stack_values(values: &[Value]) -> Chunk {
        let mut chunk = Chunk::new();
        for value in values {
            let index = chunk.add_constant(value.clone()).unwrap();
            chunk.write_instruction(OpCode::Constant(index), SPAN);
        }
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
    fn integer_equality_is_supported() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Int(10), OpCode::Equal),
            Ok(Value::Bool(true))
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
    fn null_equals_null() {
        assert_eq!(
            run_binary(Value::Null, Value::Null, OpCode::Equal),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn null_is_not_equal_to_bool() {
        assert_eq!(
            run_binary(Value::Null, Value::Bool(false), OpCode::NotEqual),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_boolean_equality() {
        assert_eq!(
            run_binary(Value::Bool(true), Value::Bool(true), OpCode::Equal),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            run_binary(Value::Bool(true), Value::Bool(false), OpCode::Equal),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn compares_float_equality() {
        assert_eq!(
            run_binary(Value::Float(3.5), Value::Float(3.5), OpCode::Equal),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_mixed_numeric_equality() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Float(10.0), OpCode::Equal),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_mixed_numeric_inequality() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Float(10.5), OpCode::NotEqual),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_string_equality() {
        assert_eq!(
            run_binary(
                Value::String("hanlin".to_owned()),
                Value::String("hanlin".to_owned()),
                OpCode::Equal,
            ),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_string_inequality() {
        assert_eq!(
            run_binary(
                Value::String("hanlin".to_owned()),
                Value::String("vm".to_owned()),
                OpCode::NotEqual,
            ),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn unrelated_primitive_types_compare_unequal() {
        for (left, right) in [
            (Value::Int(1), Value::String("1".to_owned())),
            (Value::Bool(true), Value::Int(1)),
            (Value::Null, Value::Bool(false)),
        ] {
            assert_eq!(
                run_binary(left, right, OpCode::Equal),
                Ok(Value::Bool(false))
            );
        }
    }

    #[test]
    fn compares_integer_less() {
        assert_eq!(
            run_binary(Value::Int(5), Value::Int(10), OpCode::Less),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_integer_less_equal() {
        assert_eq!(
            run_binary(Value::Int(5), Value::Int(5), OpCode::LessEqual),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_integer_greater() {
        assert_eq!(
            run_binary(Value::Int(10), Value::Int(5), OpCode::Greater),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_integer_greater_equal() {
        assert_eq!(
            run_binary(Value::Int(6), Value::Int(6), OpCode::GreaterEqual),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_float_ordering() {
        assert_eq!(
            run_binary(Value::Float(1.5), Value::Float(2.5), OpCode::Less),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            run_binary(Value::Float(2.5), Value::Float(1.5), OpCode::Greater,),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn compares_mixed_numeric_ordering() {
        assert_eq!(
            run_binary(Value::Int(5), Value::Float(5.5), OpCode::Less),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            run_binary(Value::Float(6.0), Value::Int(6), OpCode::GreaterEqual),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn invalid_ordering_types_return_type_error() {
        assert_eq!(
            run_binary(Value::Bool(true), Value::Bool(false), OpCode::Less),
            Err(VmError::TypeError {
                instruction_offset: 2,
                opcode: OpCode::Less,
                left: Value::Bool(true),
                right: Some(Value::Bool(false)),
                span: SPAN,
            })
        );
        assert!(matches!(
            run_binary(Value::Null, Value::Int(1), OpCode::Greater),
            Err(VmError::TypeError {
                opcode: OpCode::Greater,
                ..
            })
        ));
        assert!(matches!(
            run_binary(Value::String("abc".to_owned()), Value::Int(3), OpCode::Less,),
            Err(VmError::TypeError {
                opcode: OpCode::Less,
                ..
            })
        ));
    }

    #[test]
    fn orders_strings_lexically_like_interpreter() {
        let apple = Value::String("apple".to_owned());
        let banana = Value::String("banana".to_owned());

        assert_eq!(
            run_binary(apple.clone(), banana.clone(), OpCode::Less),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            run_binary(apple.clone(), apple.clone(), OpCode::LessEqual),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            run_binary(banana.clone(), apple.clone(), OpCode::Greater),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            run_binary(banana.clone(), banana, OpCode::GreaterEqual),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn logical_not_of_null_is_true() {
        assert_eq!(run_unary(Value::Null, OpCode::Not), Ok(Value::Bool(true)));
    }

    #[test]
    fn logical_not_of_bool_inverts_truthiness() {
        assert_eq!(
            run_unary(Value::Bool(false), OpCode::Not),
            Ok(Value::Bool(true))
        );
        assert_eq!(
            run_unary(Value::Bool(true), OpCode::Not),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn logical_not_of_integer_zero_is_true() {
        assert_eq!(run_unary(Value::Int(0), OpCode::Not), Ok(Value::Bool(true)));
    }

    #[test]
    fn logical_not_of_nonzero_integer_is_false() {
        assert_eq!(
            run_unary(Value::Int(-1), OpCode::Not),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn logical_not_of_float_zero_is_true() {
        assert_eq!(
            run_unary(Value::Float(0.0), OpCode::Not),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn logical_not_of_nonzero_float_is_false() {
        assert_eq!(
            run_unary(Value::Float(0.5), OpCode::Not),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn logical_not_of_empty_string_is_true() {
        assert_eq!(
            run_unary(Value::String(String::new()), OpCode::Not),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn logical_not_of_nonempty_string_is_false() {
        assert_eq!(
            run_unary(Value::String("hanlin".to_owned()), OpCode::Not),
            Ok(Value::Bool(false))
        );
    }

    #[test]
    fn logical_not_of_nan_matches_interpreter() {
        assert_eq!(
            run_unary(Value::Float(f64::NAN), OpCode::Not),
            Ok(Value::Bool(true))
        );
    }

    #[test]
    fn comparison_stack_underflow_is_structured() {
        let mut chunk = Chunk::new();
        let right = chunk.add_constant(Value::Int(1)).unwrap();
        chunk.write_instruction(OpCode::Constant(right), SPAN);
        chunk.write_instruction(OpCode::Less, SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 1,
                opcode: OpCode::Less,
                needed: 2,
                available: 1,
                span: SPAN,
            })
        );
    }

    #[test]
    fn logical_not_stack_underflow_is_structured() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Not, SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 0,
                opcode: OpCode::Not,
                needed: 1,
                available: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn vm_is_reusable_after_comparison_error() {
        let invalid = binary_chunk(Value::Null, Value::Int(1), OpCode::Less);
        let valid = binary_chunk(Value::Int(1), Value::Int(2), OpCode::Less);
        let mut vm = Vm::new();

        assert!(matches!(vm.run(&invalid), Err(VmError::TypeError { .. })));
        assert_eq!(vm.run(&valid), Ok(Value::Bool(true)));
        assert!(vm.stack.is_empty());
        assert_eq!(vm.instruction_pointer, 0);
    }

    #[test]
    fn comparison_result_is_returned() {
        let chunk = binary_chunk(Value::Int(2), Value::Int(3), OpCode::Less);
        assert_eq!(Vm::new().run(&chunk), Ok(Value::Bool(true)));
    }

    #[test]
    fn logical_not_result_is_returned() {
        let chunk = unary_chunk(Value::Bool(false), OpCode::Not);
        assert_eq!(Vm::new().run(&chunk), Ok(Value::Bool(true)));
    }

    #[test]
    fn defines_integer_global() {
        let mut vm = Vm::new();

        assert_eq!(
            vm.run(&define_global_chunk("answer", Value::Int(42))),
            Ok(Value::Null)
        );
        assert_eq!(vm.globals.get("answer"), Some(&Value::Int(42)));
    }

    #[test]
    fn defines_float_global() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("pi", Value::Float(3.14)))
            .unwrap();

        assert_eq!(vm.globals.get("pi"), Some(&Value::Float(3.14)));
    }

    #[test]
    fn defines_string_global() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk(
            "language",
            Value::String("hanlin".to_owned()),
        ))
        .unwrap();

        assert_eq!(
            vm.globals.get("language"),
            Some(&Value::String("hanlin".to_owned()))
        );
    }

    #[test]
    fn gets_defined_global() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("answer", Value::Int(42)))
            .unwrap();

        assert_eq!(vm.run(&get_global_chunk("answer")), Ok(Value::Int(42)));
    }

    #[test]
    fn sets_existing_global() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("answer", Value::Int(42)))
            .unwrap();
        vm.run(&set_global_chunk("answer", Value::Int(100)))
            .unwrap();

        assert_eq!(vm.globals.get("answer"), Some(&Value::Int(100)));
    }

    #[test]
    fn set_global_leaves_assigned_value_on_stack() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("answer", Value::Int(42)))
            .unwrap();

        assert_eq!(
            vm.run(&set_global_chunk("answer", Value::Int(100))),
            Ok(Value::Int(100))
        );
    }

    #[test]
    fn define_global_consumes_declared_value() {
        let mut vm = Vm::new();

        assert_eq!(
            vm.run(&define_global_chunk("x", Value::Int(10))),
            Ok(Value::Null)
        );
    }

    #[test]
    fn undefined_get_returns_error() {
        assert_eq!(
            Vm::new().run(&get_global_chunk("missing")),
            Err(VmError::UndefinedGlobal {
                instruction_offset: 0,
                name: "missing".to_owned(),
                span: SPAN,
            })
        );
    }

    #[test]
    fn undefined_set_returns_error() {
        assert_eq!(
            Vm::new().run(&set_global_chunk("missing", Value::Int(1))),
            Err(VmError::UndefinedGlobal {
                instruction_offset: 1,
                name: "missing".to_owned(),
                span: SPAN,
            })
        );
    }

    #[test]
    fn non_string_global_name_returns_error() {
        let mut chunk = Chunk::new();
        let name = chunk.add_constant(Value::Int(7)).unwrap();
        let value = chunk.add_constant(Value::Int(42)).unwrap();
        chunk.write_instruction(OpCode::Constant(value), SPAN);
        chunk.write_instruction(OpCode::DefineGlobal(name), SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidGlobalName {
                instruction_offset: 1,
                index: 0,
                value: Value::Int(7),
                span: SPAN,
            })
        );
    }

    #[test]
    fn missing_global_name_constant_returns_error() {
        let mut chunk = Chunk::new();
        let value = chunk.add_constant(Value::Int(42)).unwrap();
        chunk.write_instruction(OpCode::Constant(value), SPAN);
        chunk.write_instruction(OpCode::DefineGlobal(ConstantIndex::new(9)), SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidConstantReference {
                instruction_offset: 1,
                index: 9,
                constant_count: 1,
                span: SPAN,
            })
        );
    }

    #[test]
    fn multiple_globals_remain_independent() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("x", Value::Int(10))).unwrap();
        vm.run(&define_global_chunk("y", Value::Int(20))).unwrap();

        assert_eq!(vm.run(&get_global_chunk("x")), Ok(Value::Int(10)));
        assert_eq!(vm.run(&get_global_chunk("y")), Ok(Value::Int(20)));
    }

    #[test]
    fn overwriting_one_global_does_not_affect_another() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("x", Value::Int(10))).unwrap();
        vm.run(&define_global_chunk("y", Value::Int(20))).unwrap();
        vm.run(&set_global_chunk("x", Value::Int(99))).unwrap();

        assert_eq!(vm.run(&get_global_chunk("x")), Ok(Value::Int(99)));
        assert_eq!(vm.run(&get_global_chunk("y")), Ok(Value::Int(20)));
    }

    #[test]
    fn globals_persist_across_runs() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("x", Value::Int(10))).unwrap();

        assert_eq!(vm.run(&get_global_chunk("x")), Ok(Value::Int(10)));
    }

    #[test]
    fn operand_stack_resets_while_globals_persist() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("x", Value::Int(10))).unwrap();

        let mut leaves_extra_value = Chunk::new();
        let extra = leaves_extra_value.add_constant(Value::Int(999)).unwrap();
        let name = leaves_extra_value
            .add_constant(Value::String("x".to_owned()))
            .unwrap();
        leaves_extra_value.write_instruction(OpCode::Constant(extra), SPAN);
        leaves_extra_value.write_instruction(OpCode::GetGlobal(name), SPAN);
        leaves_extra_value.write_instruction(OpCode::Return, SPAN);
        assert_eq!(vm.run(&leaves_extra_value), Ok(Value::Int(10)));

        let mut empty_return = Chunk::new();
        empty_return.write_instruction(OpCode::Return, SPAN);
        assert_eq!(vm.run(&empty_return), Ok(Value::Null));
        assert_eq!(vm.run(&get_global_chunk("x")), Ok(Value::Int(10)));
    }

    #[test]
    fn vm_is_reusable_after_undefined_global_error() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("x", Value::Int(10))).unwrap();

        assert!(matches!(
            vm.run(&get_global_chunk("missing")),
            Err(VmError::UndefinedGlobal { .. })
        ));
        assert_eq!(vm.run(&get_global_chunk("x")), Ok(Value::Int(10)));
        assert!(vm.stack.is_empty());
        assert_eq!(vm.instruction_pointer, 0);
    }

    #[test]
    fn malformed_global_instruction_does_not_panic() {
        let mut chunk = Chunk::new();
        let name = chunk.add_constant(Value::Bool(true)).unwrap();
        chunk.write_instruction(OpCode::GetGlobal(name), SPAN);

        assert!(matches!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidGlobalName { .. })
        ));
    }

    #[test]
    fn define_global_without_value_returns_stack_underflow() {
        let mut chunk = Chunk::new();
        let name = chunk.add_constant(Value::String("x".to_owned())).unwrap();
        chunk.write_instruction(OpCode::DefineGlobal(name), SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 0,
                opcode: OpCode::DefineGlobal(name),
                needed: 1,
                available: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn set_global_without_value_returns_stack_underflow() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("x", Value::Int(10))).unwrap();

        let mut chunk = Chunk::new();
        let name = chunk.add_constant(Value::String("x".to_owned())).unwrap();
        chunk.write_instruction(OpCode::SetGlobal(name), SPAN);

        assert_eq!(
            vm.run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 0,
                opcode: OpCode::SetGlobal(name),
                needed: 1,
                available: 0,
                span: SPAN,
            })
        );
        assert_eq!(vm.run(&get_global_chunk("x")), Ok(Value::Int(10)));
    }

    #[test]
    fn gets_local_slot_zero() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10), Value::Int(20)]);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(10)));
    }

    #[test]
    fn gets_nonzero_local_slot() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10), Value::Int(20)]);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(1)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(20)));
    }

    #[test]
    fn get_local_clones_without_removing_original() {
        let mut chunk = chunk_with_stack_values(&[Value::String("local".to_owned())]);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::String("local".to_owned())));
    }

    #[test]
    fn sets_local_slot_zero() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10), Value::Int(99)]);
        chunk.write_instruction(OpCode::SetLocal(LocalSlot::new(0)), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(99)));
    }

    #[test]
    fn sets_nonzero_local_slot() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10), Value::Int(20), Value::Int(99)]);
        chunk.write_instruction(OpCode::SetLocal(LocalSlot::new(1)), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(1)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(99)));
    }

    #[test]
    fn set_local_leaves_assigned_value_on_top() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10), Value::Int(99)]);
        chunk.write_instruction(OpCode::SetLocal(LocalSlot::new(0)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(99)));
    }

    #[test]
    fn set_local_updates_only_target_slot() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10), Value::Int(20), Value::Int(99)]);
        chunk.write_instruction(OpCode::SetLocal(LocalSlot::new(0)), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(1)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(20)));
    }

    #[test]
    fn invalid_get_local_returns_slot_error() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10)]);
        let span = Span::new(4, 7);
        let slot = LocalSlot::new(1);
        chunk.write_instruction(OpCode::GetLocal(slot), span);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidLocalSlot {
                instruction_offset: 1,
                slot,
                stack_len: 1,
                span,
            })
        );
    }

    #[test]
    fn invalid_set_local_returns_slot_error_without_growing_stack() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10)]);
        let span = Span::new(5, 8);
        let slot = LocalSlot::new(1);
        chunk.write_instruction(OpCode::SetLocal(slot), span);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidLocalSlot {
                instruction_offset: 1,
                slot,
                stack_len: 1,
                span,
            })
        );
    }

    #[test]
    fn empty_stack_local_access_returns_slot_error() {
        for opcode in [
            OpCode::GetLocal(LocalSlot::new(0)),
            OpCode::SetLocal(LocalSlot::new(0)),
        ] {
            let mut chunk = Chunk::new();
            chunk.write_instruction(opcode, SPAN);

            assert_eq!(
                Vm::new().run(&chunk),
                Err(VmError::InvalidLocalSlot {
                    instruction_offset: 0,
                    slot: LocalSlot::new(0),
                    stack_len: 0,
                    span: SPAN,
                })
            );
        }
    }

    #[test]
    fn locals_work_with_arithmetic_temporaries() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10), Value::Int(20)]);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), SPAN);
        chunk.write_instruction(OpCode::Add, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(30)));
    }

    #[test]
    fn globals_and_locals_remain_independent() {
        let mut chunk = Chunk::new();
        let name = chunk.add_constant(Value::String("x".to_owned())).unwrap();
        let global = chunk.add_constant(Value::Int(100)).unwrap();
        let local = chunk.add_constant(Value::Int(10)).unwrap();
        chunk.write_instruction(OpCode::Constant(global), SPAN);
        chunk.write_instruction(OpCode::DefineGlobal(name), SPAN);
        chunk.write_instruction(OpCode::Constant(local), SPAN);
        chunk.write_instruction(OpCode::GetGlobal(name), SPAN);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), SPAN);
        chunk.write_instruction(OpCode::Add, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        let mut vm = Vm::new();
        assert_eq!(vm.run(&chunk), Ok(Value::Int(110)));
        assert_eq!(vm.run(&get_global_chunk("x")), Ok(Value::Int(100)));
    }

    #[test]
    fn globals_persist_while_local_runs_reset() {
        let mut vm = Vm::new();
        vm.run(&define_global_chunk("x", Value::Int(7))).unwrap();

        let mut local = chunk_with_stack_values(&[Value::Int(99)]);
        local.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), SPAN);
        local.write_instruction(OpCode::Return, SPAN);

        assert_eq!(vm.run(&local), Ok(Value::Int(99)));
        assert_eq!(vm.run(&get_global_chunk("x")), Ok(Value::Int(7)));
    }

    #[test]
    fn locals_do_not_persist_across_runs() {
        let mut vm = Vm::new();
        assert_eq!(vm.run(&constant_chunk(Value::Int(10))), Ok(Value::Int(10)));

        let mut get_previous_local = Chunk::new();
        get_previous_local.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), SPAN);
        assert_eq!(
            vm.run(&get_previous_local),
            Err(VmError::InvalidLocalSlot {
                instruction_offset: 0,
                slot: LocalSlot::new(0),
                stack_len: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn vm_is_reusable_after_invalid_local_error() {
        let mut invalid = Chunk::new();
        invalid.write_instruction(OpCode::GetLocal(LocalSlot::new(3)), SPAN);

        let mut valid = chunk_with_stack_values(&[Value::Int(42)]);
        valid.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), SPAN);
        valid.write_instruction(OpCode::Return, SPAN);

        let mut vm = Vm::new();
        assert!(matches!(
            vm.run(&invalid),
            Err(VmError::InvalidLocalSlot { .. })
        ));
        assert_eq!(vm.run(&valid), Ok(Value::Int(42)));
        assert!(vm.stack.is_empty());
        assert_eq!(vm.instruction_pointer, 0);
    }

    #[test]
    fn maximum_local_slot_is_checked_at_runtime() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(1)]);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::MAX), SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidLocalSlot {
                instruction_offset: 1,
                slot: LocalSlot::MAX,
                stack_len: 1,
                span: SPAN,
            })
        );
    }

    #[test]
    fn malformed_local_access_does_not_panic() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::SetLocal(LocalSlot::MAX), SPAN);

        let result = std::panic::catch_unwind(|| Vm::new().run(&chunk));
        assert!(matches!(
            result,
            Ok(Err(VmError::InvalidLocalSlot {
                slot: LocalSlot::MAX,
                stack_len: 0,
                ..
            }))
        ));
    }

    #[test]
    fn get_local_followed_by_return_yields_local_value() {
        let mut chunk = chunk_with_stack_values(&[Value::Bool(true)]);
        chunk.write_instruction(OpCode::GetLocal(LocalSlot::new(0)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Bool(true)));
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

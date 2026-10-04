//! Execution engine for Hanlin bytecode.
//!
//! The VM executes manually constructed bytecode and chunks produced by the
//! AST compiler, including named closures, shared lexical upvalues through
//! iterative call frames, and reference-counted mutable aggregate objects.
//! Integer arithmetic is checked and reports [`VmError::IntegerOverflow`]
//! instead of depending on Rust's debug or release overflow behavior.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use crate::error::Span;

use super::closure::{Upvalue, UpvalueState};
use super::opcode::{resolve_jump_target, JumpDirection};
use super::{
    AggregateCount, Arity, Chunk, Closure, ConstantIndex, Function, GcStats, Heap, HeapError,
    HeapObject, JumpOffset, LocalSlot, OpCode, UpvalueDescriptor, UpvalueIndex, Value,
};

/// Maximum number of active Hanlin function calls, excluding the script frame.
pub const MAX_CALL_FRAMES: usize = 1024;
/// Small object-count floor for the VM-15 automatic collector.
pub const MIN_GC_THRESHOLD: usize = 8;

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
    InvalidJumpTarget {
        instruction_offset: usize,
        opcode: OpCode,
        target: Option<usize>,
        instruction_count: usize,
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
    NotCallable {
        instruction_offset: usize,
        value: Value,
        span: Span,
    },
    ArityMismatch {
        instruction_offset: usize,
        function: String,
        expected: Arity,
        got: Arity,
        span: Span,
    },
    CallStackOverflow {
        instruction_offset: usize,
        limit: usize,
        span: Span,
    },
    InvalidClosureFunction {
        instruction_offset: usize,
        index: u32,
        value: Value,
        span: Span,
    },
    InvalidUpvalue {
        instruction_offset: usize,
        opcode: OpCode,
        index: UpvalueIndex,
        upvalue_count: usize,
        span: Span,
    },
    InvalidCaptureLocal {
        instruction_offset: usize,
        slot: LocalSlot,
        stack_len: usize,
        span: Span,
    },
    InvalidCaptureUpvalue {
        instruction_offset: usize,
        index: UpvalueIndex,
        upvalue_count: usize,
        span: Span,
    },
    InvalidOpenUpvalue {
        instruction_offset: usize,
        stack_index: usize,
        stack_len: usize,
        span: Span,
    },
    IndexTypeError {
        instruction_offset: usize,
        index: Value,
        span: Span,
    },
    IndexOutOfBounds {
        instruction_offset: usize,
        index: i64,
        length: usize,
        assignment: bool,
        span: Span,
    },
    InvalidIndexTarget {
        instruction_offset: usize,
        target: Value,
        assignment: bool,
        span: Span,
    },
    InvalidMapKey {
        instruction_offset: usize,
        key: Value,
        span: Span,
    },
    HeapAccess {
        instruction_offset: usize,
        error: HeapError,
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
            Self::InvalidJumpTarget {
                instruction_offset,
                opcode,
                target,
                instruction_count,
                span,
            } => {
                write!(
                    f,
                    "invalid jump while executing {opcode:?} at instruction {instruction_offset} ({span})"
                )?;
                if let Some(target) = target {
                    write!(
                        f,
                        ": target {target} is outside the chunk's {instruction_count} instructions"
                    )
                } else {
                    write!(f, ": target arithmetic overflowed or underflowed")
                }
            }
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
            Self::NotCallable {
                instruction_offset,
                value,
                span,
            } => write!(
                f,
                "value {value:?} is not callable at instruction {instruction_offset} ({span})"
            ),
            Self::ArityMismatch {
                instruction_offset,
                function,
                expected,
                got,
                span,
            } => write!(
                f,
                "function '{function}' expects {} arguments, but got {} at instruction {instruction_offset} ({span})",
                expected.as_u8(),
                got.as_u8()
            ),
            Self::CallStackOverflow {
                instruction_offset,
                limit,
                span,
            } => write!(
                f,
                "call stack limit {limit} exceeded at instruction {instruction_offset} ({span})"
            ),
            Self::InvalidClosureFunction {
                instruction_offset,
                index,
                value,
                span,
            } => write!(
                f,
                "constant index {index} is not a compiled function for CLOSURE at instruction {instruction_offset} ({span}): {value:?}"
            ),
            Self::InvalidUpvalue {
                instruction_offset,
                opcode,
                index,
                upvalue_count,
                span,
            } => write!(
                f,
                "upvalue index {} is invalid for a closure with {upvalue_count} upvalues while executing {opcode:?} at instruction {instruction_offset} ({span})",
                index.as_u8()
            ),
            Self::InvalidCaptureLocal {
                instruction_offset,
                slot,
                stack_len,
                span,
            } => write!(
                f,
                "cannot capture local slot {} from a stack with {stack_len} values at instruction {instruction_offset} ({span})",
                slot.as_u16()
            ),
            Self::InvalidCaptureUpvalue {
                instruction_offset,
                index,
                upvalue_count,
                span,
            } => write!(
                f,
                "cannot capture enclosing upvalue {} from a closure with {upvalue_count} upvalues at instruction {instruction_offset} ({span})",
                index.as_u8()
            ),
            Self::InvalidOpenUpvalue {
                instruction_offset,
                stack_index,
                stack_len,
                span,
            } => write!(
                f,
                "open upvalue points to stack index {stack_index}, but the stack has {stack_len} values at instruction {instruction_offset} ({span})"
            ),
            Self::IndexTypeError {
                instruction_offset,
                index,
                span,
            } => write!(
                f,
                "array/string index must be an integer at instruction {instruction_offset} ({span}), got {index:?}"
            ),
            Self::IndexOutOfBounds {
                instruction_offset,
                index,
                length,
                assignment,
                span,
            } => write!(
                f,
                "index {index} is out of bounds for {} of length {length} at instruction {instruction_offset} ({span})",
                if *assignment { "assignment to aggregate" } else { "aggregate" }
            ),
            Self::InvalidIndexTarget {
                instruction_offset,
                target,
                assignment,
                span,
            } => write!(
                f,
                "cannot {} property/index of {target:?} at instruction {instruction_offset} ({span})",
                if *assignment { "assign to" } else { "read" }
            ),
            Self::InvalidMapKey {
                instruction_offset,
                key,
                span,
            } => write!(
                f,
                "map/object index must be a string at instruction {instruction_offset} ({span}), got {key:?}"
            ),
            Self::HeapAccess {
                instruction_offset,
                error,
                span,
            } => write!(
                f,
                "heap access failed at instruction {instruction_offset} ({span}): {error}"
            ),
        }
    }
}

impl std::error::Error for VmError {}

/// The basic stack virtual machine for Hanlin bytecode.
///
/// The operand stack is private and belongs to one execution. Both it and the
/// call-frame stack are reset before and after every [`run`](Self::run).
/// Global bindings are intentionally retained across runs for future REPL use.
#[derive(Debug)]
pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<CallFrame>,
    open_upvalues: Vec<Upvalue>,
    globals: HashMap<String, Value>,
    heap: Heap,
    next_gc: usize,
    minimum_gc_threshold: usize,
}

#[derive(Debug)]
struct CallFrame {
    closure: Rc<Closure>,
    instruction_pointer: usize,
    base: usize,
    last_span: Option<Span>,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            frames: Vec::new(),
            open_upvalues: Vec::new(),
            globals: HashMap::new(),
            heap: Heap::new(),
            next_gc: MIN_GC_THRESHOLD,
            minimum_gc_threshold: MIN_GC_THRESHOLD,
        }
    }

    /// Formats a value using this VM's heap for aggregate dereferencing.
    pub fn format_value(&self, value: &Value) -> Result<String, HeapError> {
        self.heap.format_value(value)
    }

    pub const fn heap_live_count(&self) -> usize {
        self.heap.live_count()
    }

    pub fn heap_capacity(&self) -> usize {
        self.heap.capacity()
    }

    pub const fn gc_threshold(&self) -> usize {
        self.next_gc
    }

    /// Overrides the object-count trigger. Primarily useful for deterministic
    /// stress tests; subsequent collections still apply the live*2 policy.
    pub fn set_gc_threshold(&mut self, threshold: usize) {
        let threshold = threshold.max(1);
        self.minimum_gc_threshold = threshold;
        self.next_gc = threshold;
    }

    /// Runs a full collection over stack, globals, frames, and upvalues.
    pub fn collect_garbage(&mut self) -> Result<GcStats, HeapError> {
        let mut roots = self.stack.clone();
        roots.extend(self.globals.values().cloned());
        roots.extend(
            self.frames
                .iter()
                .map(|frame| Value::Closure(Rc::clone(&frame.closure))),
        );
        let upvalues = self.open_upvalues.clone();
        let stats = self.heap.collect(&roots, &self.stack, &upvalues)?;
        self.next_gc = stats.after.saturating_mul(2).max(self.minimum_gc_threshold);
        Ok(stats)
    }

    /// Executes `chunk` until `Return` or a structured [`VmError`].
    ///
    /// `Return` yields the top stack value. If the stack is empty, it yields
    /// [`Value::Null`]. The stack and call frames are cleared on both
    /// success and failure, while global bindings persist.
    pub fn run(&mut self, chunk: &Chunk) -> Result<Value, VmError> {
        self.reset_execution_state();
        let script = Rc::new(Closure::new(Rc::new(Function::new(
            "<script>",
            Arity::new(0),
            chunk.clone(),
        ))));
        self.frames.push(CallFrame {
            closure: script,
            instruction_pointer: 0,
            base: 0,
            last_span: None,
        });
        let result = self.execute();
        self.reset_execution_state();
        result
    }

    fn execute(&mut self) -> Result<Value, VmError> {
        loop {
            let (closure, instruction_offset, instruction) = {
                let frame = self
                    .frames
                    .last_mut()
                    .expect("the script frame remains active until execution returns");
                let instruction_offset = frame.instruction_pointer;
                let instruction = frame
                    .closure
                    .function()
                    .chunk()
                    .instruction(instruction_offset)
                    .copied()
                    .ok_or(VmError::InstructionPointerOutOfBounds {
                        instruction_pointer: instruction_offset,
                        instruction_count: frame.closure.function().chunk().instructions().len(),
                        last_span: frame.last_span,
                    })?;
                frame.instruction_pointer += 1;
                frame.last_span = Some(instruction.span());
                (Rc::clone(&frame.closure), instruction_offset, instruction)
            };
            let chunk = closure.function().chunk();
            let span = instruction.span();

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
                OpCode::Jump(offset) => {
                    self.execute_jump(
                        chunk,
                        OpCode::Jump(offset),
                        offset,
                        JumpDirection::Forward,
                        instruction_offset,
                        span,
                    )?;
                }
                OpCode::JumpIfFalse(offset) => {
                    self.execute_jump_if_false(chunk, offset, instruction_offset, span)?;
                }
                OpCode::Loop(offset) => {
                    self.execute_jump(
                        chunk,
                        OpCode::Loop(offset),
                        offset,
                        JumpDirection::Backward,
                        instruction_offset,
                        span,
                    )?;
                }
                OpCode::Closure(index) => {
                    self.execute_closure(chunk, index, instruction_offset, span)?;
                }
                OpCode::GetUpvalue(index) => {
                    self.get_upvalue(index, instruction_offset, span)?;
                }
                OpCode::SetUpvalue(index) => {
                    self.set_upvalue(index, instruction_offset, span)?;
                }
                OpCode::CloseUpvalue(slot) => {
                    self.close_upvalue(slot, instruction_offset, span)?;
                }
                OpCode::BuildArray(count) => {
                    self.build_array(count, instruction_offset, span)?;
                }
                OpCode::BuildMap(count) => {
                    self.build_map(count, instruction_offset, span)?;
                }
                OpCode::GetIndex => {
                    self.get_index(instruction_offset, span)?;
                }
                OpCode::SetIndex => {
                    self.set_index(instruction_offset, span)?;
                }
                OpCode::Call(arity) => {
                    self.execute_call(arity, instruction_offset, span)?;
                }
                OpCode::Return => {
                    if let Some(result) = self.execute_return() {
                        return Ok(result);
                    }
                }
            }
        }
    }

    fn build_array(
        &mut self,
        count: AggregateCount,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let needed = count.as_usize();
        let available = self.stack.len();
        if available < needed {
            return Err(VmError::StackUnderflow {
                instruction_offset,
                opcode: OpCode::BuildArray(count),
                needed,
                available,
                span,
            });
        }
        self.maybe_collect_for_allocation(instruction_offset, span)?;
        let elements = self.stack.split_off(available - needed);
        let reference = self
            .heap
            .allocate(HeapObject::Array(elements))
            .map_err(|error| VmError::HeapAccess {
                instruction_offset,
                error,
                span,
            })?;
        self.stack.push(Value::Heap(reference));
        Ok(())
    }

    fn build_map(
        &mut self,
        count: AggregateCount,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let needed = count.as_usize() * 2;
        let available = self.stack.len();
        if available < needed {
            return Err(VmError::StackUnderflow {
                instruction_offset,
                opcode: OpCode::BuildMap(count),
                needed,
                available,
                span,
            });
        }
        self.maybe_collect_for_allocation(instruction_offset, span)?;
        let start = available - needed;
        for pair in self.stack[start..].as_chunks::<2>().0 {
            if !matches!(pair[0], Value::String(_)) {
                return Err(VmError::InvalidMapKey {
                    instruction_offset,
                    key: pair[0].clone(),
                    span,
                });
            }
        }

        let pairs = self.stack.split_off(start);
        let mut entries = HashMap::with_capacity(count.as_usize());
        let mut pairs = pairs.into_iter();
        while let Some(key) = pairs.next() {
            let value = pairs.next().expect("validated map pairs contain a value");
            let Value::String(key) = key else {
                unreachable!("map keys were validated before stack mutation")
            };
            entries.insert(key, value);
        }
        let reference = self
            .heap
            .allocate(HeapObject::Map(entries))
            .map_err(|error| VmError::HeapAccess {
                instruction_offset,
                error,
                span,
            })?;
        self.stack.push(Value::Heap(reference));
        Ok(())
    }

    fn get_index(&mut self, instruction_offset: usize, span: Span) -> Result<(), VmError> {
        self.require_stack(OpCode::GetIndex, 2, instruction_offset, span)?;
        let index = self.stack.pop().expect("stack depth was validated");
        let target = self.stack.pop().expect("stack depth was validated");

        let result = match target {
            Value::Heap(reference) => {
                match self
                    .heap
                    .get(reference)
                    .map_err(|error| VmError::HeapAccess {
                        instruction_offset,
                        error,
                        span,
                    })? {
                    HeapObject::Array(elements) => {
                        let index = Self::integer_index(index, instruction_offset, span)?;
                        Self::checked_element(elements, index, false, instruction_offset, span)?
                    }
                    HeapObject::Map(entries) => {
                        let Value::String(key) = index else {
                            return Err(VmError::InvalidMapKey {
                                instruction_offset,
                                key: index,
                                span,
                            });
                        };
                        entries.get(&key).cloned().unwrap_or(Value::Null)
                    }
                }
            }
            Value::String(value) => {
                let index = Self::integer_index(index, instruction_offset, span)?;
                let characters: Vec<char> = value.chars().collect();
                let character =
                    Self::checked_element(&characters, index, false, instruction_offset, span)?;
                Value::String(character.to_string())
            }
            _ => {
                return Err(VmError::InvalidIndexTarget {
                    instruction_offset,
                    target,
                    assignment: false,
                    span,
                })
            }
        };
        self.stack.push(result);
        Ok(())
    }

    fn set_index(&mut self, instruction_offset: usize, span: Span) -> Result<(), VmError> {
        self.require_stack(OpCode::SetIndex, 3, instruction_offset, span)?;
        let value = self.stack.pop().expect("stack depth was validated");
        let index = self.stack.pop().expect("stack depth was validated");
        let target = self.stack.pop().expect("stack depth was validated");

        match target {
            Value::Heap(reference) => {
                match self
                    .heap
                    .get_mut(reference)
                    .map_err(|error| VmError::HeapAccess {
                        instruction_offset,
                        error,
                        span,
                    })? {
                    HeapObject::Array(elements) => {
                        let index = Self::integer_index(index, instruction_offset, span)?;
                        let length = elements.len();
                        let element =
                            elements.get_mut(usize::try_from(index).unwrap_or(usize::MAX));
                        let Some(element) = element else {
                            return Err(VmError::IndexOutOfBounds {
                                instruction_offset,
                                index,
                                length,
                                assignment: true,
                                span,
                            });
                        };
                        *element = value.clone();
                    }
                    HeapObject::Map(entries) => {
                        let Value::String(key) = index else {
                            return Err(VmError::InvalidMapKey {
                                instruction_offset,
                                key: index,
                                span,
                            });
                        };
                        entries.insert(key, value.clone());
                    }
                }
            }
            _ => {
                return Err(VmError::InvalidIndexTarget {
                    instruction_offset,
                    target,
                    assignment: true,
                    span,
                })
            }
        }
        self.stack.push(value);
        Ok(())
    }

    /// Automatic collection is a pre-allocation safe point: every child value
    /// for BUILD_ARRAY/BUILD_MAP is still on the operand stack. Allocation and
    /// pushing the resulting handle then happen without another collection, so
    /// no separate temporary-root guard is required.
    fn maybe_collect_for_allocation(
        &mut self,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        if self.heap.live_count() >= self.next_gc {
            self.collect_garbage()
                .map_err(|error| VmError::HeapAccess {
                    instruction_offset,
                    error,
                    span,
                })?;
        }
        Ok(())
    }

    fn require_stack(
        &self,
        opcode: OpCode,
        needed: usize,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let available = self.stack.len();
        if available < needed {
            Err(VmError::StackUnderflow {
                instruction_offset,
                opcode,
                needed,
                available,
                span,
            })
        } else {
            Ok(())
        }
    }

    fn integer_index(index: Value, instruction_offset: usize, span: Span) -> Result<i64, VmError> {
        match index {
            Value::Int(index) => Ok(index),
            Value::Float(index) if index.fract() == 0.0 => Ok(index as i64),
            index => Err(VmError::IndexTypeError {
                instruction_offset,
                index,
                span,
            }),
        }
    }

    fn checked_element<T: Clone>(
        elements: &[T],
        index: i64,
        assignment: bool,
        instruction_offset: usize,
        span: Span,
    ) -> Result<T, VmError> {
        elements
            .get(usize::try_from(index).unwrap_or(usize::MAX))
            .cloned()
            .ok_or(VmError::IndexOutOfBounds {
                instruction_offset,
                index,
                length: elements.len(),
                assignment,
                span,
            })
    }

    fn execute_closure(
        &mut self,
        chunk: &Chunk,
        index: ConstantIndex,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let value = chunk
            .constant(index)
            .cloned()
            .ok_or(VmError::InvalidConstantReference {
                instruction_offset,
                index: index.as_u32(),
                constant_count: chunk.constants().len(),
                span,
            })?;
        let Value::Function(function) = value else {
            return Err(VmError::InvalidClosureFunction {
                instruction_offset,
                index: index.as_u32(),
                value,
                span,
            });
        };

        let (frame_base, enclosing) = {
            let frame = self
                .frames
                .last()
                .expect("closure construction requires an active frame");
            (frame.base, Rc::clone(&frame.closure))
        };
        let mut upvalues = Vec::with_capacity(function.upvalues().len());
        for descriptor in function.upvalues() {
            let upvalue = match descriptor {
                UpvalueDescriptor::Local(slot) => {
                    let stack_index = frame_base
                        .checked_add(slot.as_usize())
                        .filter(|index| *index < self.stack.len());
                    let Some(stack_index) = stack_index else {
                        return Err(VmError::InvalidCaptureLocal {
                            instruction_offset,
                            slot: *slot,
                            stack_len: self.stack.len(),
                            span,
                        });
                    };
                    self.capture_upvalue(stack_index)
                }
                UpvalueDescriptor::Upvalue(index) => enclosing
                    .upvalue(index.as_usize())
                    .cloned()
                    .ok_or(VmError::InvalidCaptureUpvalue {
                    instruction_offset,
                    index: *index,
                    upvalue_count: enclosing.upvalue_count(),
                    span,
                })?,
            };
            upvalues.push(upvalue);
        }
        self.stack
            .push(Value::Closure(Rc::new(Closure::with_upvalues(
                function, upvalues,
            ))));
        Ok(())
    }

    fn get_upvalue(
        &mut self,
        index: UpvalueIndex,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let upvalue =
            self.current_upvalue(index, OpCode::GetUpvalue(index), instruction_offset, span)?;
        let value = match &*upvalue.borrow() {
            UpvalueState::Open(stack_index) => {
                self.stack
                    .get(*stack_index)
                    .cloned()
                    .ok_or(VmError::InvalidOpenUpvalue {
                        instruction_offset,
                        stack_index: *stack_index,
                        stack_len: self.stack.len(),
                        span,
                    })?
            }
            UpvalueState::Closed(value) => value.clone(),
        };
        self.stack.push(value);
        Ok(())
    }

    fn set_upvalue(
        &mut self,
        index: UpvalueIndex,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let value = self.stack.last().cloned().ok_or(VmError::StackUnderflow {
            instruction_offset,
            opcode: OpCode::SetUpvalue(index),
            needed: 1,
            available: 0,
            span,
        })?;
        let upvalue =
            self.current_upvalue(index, OpCode::SetUpvalue(index), instruction_offset, span)?;
        let mut state = upvalue.borrow_mut();
        match &mut *state {
            UpvalueState::Open(stack_index) => {
                let stack_len = self.stack.len();
                let slot = self
                    .stack
                    .get_mut(*stack_index)
                    .ok_or(VmError::InvalidOpenUpvalue {
                        instruction_offset,
                        stack_index: *stack_index,
                        stack_len,
                        span,
                    })?;
                *slot = value;
            }
            UpvalueState::Closed(closed) => *closed = value,
        }
        Ok(())
    }

    fn current_upvalue(
        &self,
        index: UpvalueIndex,
        opcode: OpCode,
        instruction_offset: usize,
        span: Span,
    ) -> Result<Upvalue, VmError> {
        let closure = &self
            .frames
            .last()
            .expect("upvalue access requires an active frame")
            .closure;
        closure
            .upvalue(index.as_usize())
            .cloned()
            .ok_or(VmError::InvalidUpvalue {
                instruction_offset,
                opcode,
                index,
                upvalue_count: closure.upvalue_count(),
                span,
            })
    }

    fn capture_upvalue(&mut self, stack_index: usize) -> Upvalue {
        if let Some(upvalue) = self.open_upvalues.iter().find(|upvalue| {
            matches!(*upvalue.borrow(), UpvalueState::Open(index) if index == stack_index)
        }) {
            return Rc::clone(upvalue);
        }
        let upvalue = Rc::new(RefCell::new(UpvalueState::Open(stack_index)));
        self.open_upvalues.push(Rc::clone(&upvalue));
        upvalue
    }

    fn close_upvalue(
        &mut self,
        slot: LocalSlot,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let base = self
            .frames
            .last()
            .expect("upvalue closing requires an active frame")
            .base;
        let stack_index = base
            .checked_add(slot.as_usize())
            .filter(|index| *index < self.stack.len())
            .ok_or(VmError::InvalidCaptureLocal {
                instruction_offset,
                slot,
                stack_len: self.stack.len(),
                span,
            })?;
        self.close_upvalues_matching(|index| index == stack_index);
        Ok(())
    }

    fn close_upvalues_from(&mut self, first_stack_index: usize) {
        self.close_upvalues_matching(|index| index >= first_stack_index);
    }

    fn close_upvalues_matching(&mut self, should_close: impl Fn(usize) -> bool) {
        for upvalue in &self.open_upvalues {
            let stack_index = match *upvalue.borrow() {
                UpvalueState::Open(index) if should_close(index) => index,
                _ => continue,
            };
            if let Some(value) = self.stack.get(stack_index).cloned() {
                *upvalue.borrow_mut() = UpvalueState::Closed(value);
            }
        }
        self.open_upvalues
            .retain(|upvalue| matches!(*upvalue.borrow(), UpvalueState::Open(_)));
    }

    fn execute_call(
        &mut self,
        arity: Arity,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let needed = arity.as_usize() + 1;
        if self.stack.len() < needed {
            return Err(VmError::StackUnderflow {
                instruction_offset,
                opcode: OpCode::Call(arity),
                needed,
                available: self.stack.len(),
                span,
            });
        }
        let callee_index = self.stack.len() - needed;
        let callee = self.stack[callee_index].clone();
        let Value::Closure(closure) = callee else {
            return Err(VmError::NotCallable {
                instruction_offset,
                value: callee,
                span,
            });
        };
        if closure.function().arity() != arity {
            return Err(VmError::ArityMismatch {
                instruction_offset,
                function: closure.function().name().to_owned(),
                expected: closure.function().arity(),
                got: arity,
                span,
            });
        }
        let active_calls = self.frames.len().saturating_sub(1);
        if active_calls >= MAX_CALL_FRAMES {
            return Err(VmError::CallStackOverflow {
                instruction_offset,
                limit: MAX_CALL_FRAMES,
                span,
            });
        }
        self.frames.push(CallFrame {
            closure,
            instruction_pointer: 0,
            base: callee_index + 1,
            last_span: None,
        });
        Ok(())
    }

    fn execute_return(&mut self) -> Option<Value> {
        let result = self.stack.pop().unwrap_or(Value::Null);
        if self.frames.len() == 1 {
            self.frames.pop();
            return Some(result);
        }

        let frame = self
            .frames
            .pop()
            .expect("a non-script return always has a function frame");
        let callee_index = frame
            .base
            .checked_sub(1)
            .expect("function frame bases follow their callee");
        self.close_upvalues_from(frame.base);
        self.stack.truncate(callee_index);
        self.stack.push(result);
        None
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
            (Value::Heap(left), Value::Heap(right)) => left == right,
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
    /// The script frame starts at zero. Function frames start immediately after
    /// their callee, so slot zero addresses the first argument.
    fn resolve_local_index(
        &self,
        slot: LocalSlot,
        instruction_offset: usize,
        span: Span,
    ) -> Result<usize, VmError> {
        let frame_base = self
            .frames
            .last()
            .expect("local access requires an active frame")
            .base;
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

    fn execute_jump(
        &mut self,
        chunk: &Chunk,
        opcode: OpCode,
        offset: JumpOffset,
        direction: JumpDirection,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let target =
            Self::checked_jump_target(chunk, opcode, offset, direction, instruction_offset, span)?;
        self.current_frame_mut().instruction_pointer = target;
        Ok(())
    }

    fn execute_jump_if_false(
        &mut self,
        chunk: &Chunk,
        offset: JumpOffset,
        instruction_offset: usize,
        span: Span,
    ) -> Result<(), VmError> {
        let opcode = OpCode::JumpIfFalse(offset);
        let should_jump = !self
            .stack
            .last()
            .ok_or(VmError::StackUnderflow {
                instruction_offset,
                opcode,
                needed: 1,
                available: 0,
                span,
            })?
            .is_truthy();
        let target = Self::checked_jump_target(
            chunk,
            opcode,
            offset,
            JumpDirection::Forward,
            instruction_offset,
            span,
        )?;

        if should_jump {
            self.current_frame_mut().instruction_pointer = target;
        }
        Ok(())
    }

    fn checked_jump_target(
        chunk: &Chunk,
        opcode: OpCode,
        offset: JumpOffset,
        direction: JumpDirection,
        instruction_offset: usize,
        span: Span,
    ) -> Result<usize, VmError> {
        resolve_jump_target(
            instruction_offset,
            offset,
            direction,
            chunk.instructions().len(),
        )
        .map_err(|error| VmError::InvalidJumpTarget {
            instruction_offset,
            opcode,
            target: error.target,
            instruction_count: chunk.instructions().len(),
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
        self.close_upvalues_from(0);
        self.stack.clear();
        self.frames.clear();
        self.open_upvalues.clear();
    }

    fn current_frame_mut(&mut self) -> &mut CallFrame {
        self.frames
            .last_mut()
            .expect("instruction execution requires an active frame")
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::{Vm, VmError};
    use crate::error::Span;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::vm::{
        AggregateCount, Arity, Chunk, Compiler, ConstantIndex, Function, HeapError, HeapObject,
        HeapRef, JumpOffset, LocalSlot, OpCode, UpvalueDescriptor, UpvalueIndex, Value,
    };

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

    fn compile_source(source: &str) -> Chunk {
        let tokens = Lexer::new(source).tokenize().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        Compiler::new().compile(&program).unwrap()
    }

    fn run_source(vm: &mut Vm, source: &str) -> Value {
        vm.run(&compile_source(source)).unwrap()
    }

    fn read_global_value(vm: &mut Vm, name: &str) -> Value {
        vm.run(&get_global_chunk(name)).unwrap()
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
        assert!(vm.frames.is_empty());
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
        assert!(vm.frames.is_empty());
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
        assert!(vm.frames.is_empty());
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
        assert!(vm.frames.is_empty());
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
        assert!(vm.frames.is_empty());
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

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Vm::new().run(&chunk)));
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
    fn jump_skips_instructions() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10)]);
        let skipped = chunk.add_constant(Value::Int(99)).unwrap();
        chunk.write_instruction(OpCode::Jump(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::Constant(skipped), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(10)));
    }

    #[test]
    fn jump_preserves_stack() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10), Value::Int(20)]);
        chunk.write_instruction(OpCode::Jump(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(20)));
    }

    #[test]
    fn jump_if_false_jumps_on_false() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::False, SPAN);
        chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::True, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Bool(false)));
    }

    #[test]
    fn jump_if_false_continues_on_true() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::True, SPAN);
        chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::False, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Bool(false)));
    }

    #[test]
    fn jump_if_false_follows_null_truthiness() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Null, SPAN);
        chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::True, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Null));
    }

    #[test]
    fn jump_if_false_follows_integer_zero_truthiness() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(0)]);
        let skipped = chunk.add_constant(Value::Int(99)).unwrap();
        chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::Constant(skipped), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(0)));
    }

    #[test]
    fn jump_if_false_continues_on_nonzero_integer() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(1)]);
        let next = chunk.add_constant(Value::Int(99)).unwrap();
        chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::Constant(next), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(99)));
    }

    #[test]
    fn jump_if_false_follows_empty_string_truthiness() {
        let mut chunk = chunk_with_stack_values(&[Value::String(String::new())]);
        let skipped = chunk
            .add_constant(Value::String("not empty".to_owned()))
            .unwrap();
        chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::Constant(skipped), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::String(String::new())));
    }

    #[test]
    fn jump_if_false_reuses_float_truthiness() {
        for condition in [Value::Float(0.0), Value::Float(f64::NAN)] {
            let mut chunk = chunk_with_stack_values(&[condition]);
            let skipped = chunk.add_constant(Value::Int(99)).unwrap();
            chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(1)), SPAN);
            chunk.write_instruction(OpCode::Constant(skipped), SPAN);
            chunk.write_instruction(OpCode::Return, SPAN);

            assert!(matches!(Vm::new().run(&chunk), Ok(Value::Float(_))));
        }
    }

    #[test]
    fn jump_if_false_does_not_pop_condition() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::False, SPAN);
        chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(0)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Bool(false)));
    }

    #[test]
    fn jump_if_false_on_empty_stack_returns_underflow() {
        let opcode = OpCode::JumpIfFalse(JumpOffset::new(0));
        let mut chunk = Chunk::new();
        chunk.write_instruction(opcode, Span::new(4, 2));
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 0,
                opcode,
                needed: 1,
                available: 0,
                span: Span::new(4, 2),
            })
        );
    }

    #[test]
    fn loop_jumps_backward() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10)]);
        chunk.write_instruction(OpCode::Jump(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        chunk.write_instruction(OpCode::Loop(JumpOffset::new(2)), SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(10)));
    }

    #[test]
    fn loop_preserves_stack() {
        let mut chunk = chunk_with_stack_values(&[Value::Int(10), Value::Int(20)]);
        chunk.write_instruction(OpCode::Jump(JumpOffset::new(1)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        chunk.write_instruction(OpCode::Loop(JumpOffset::new(2)), SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(20)));
    }

    #[test]
    fn zero_offset_jump_targets_next_instruction() {
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::Jump(JumpOffset::new(0)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Null));
    }

    #[test]
    fn forward_jump_to_chunk_end_is_rejected() {
        let opcode = OpCode::Jump(JumpOffset::new(1));
        let mut chunk = Chunk::new();
        chunk.write_instruction(opcode, Span::new(7, 3));
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidJumpTarget {
                instruction_offset: 0,
                opcode,
                target: Some(2),
                instruction_count: 2,
                span: Span::new(7, 3),
            })
        );
    }

    #[test]
    fn backward_jump_underflow_is_rejected() {
        let opcode = OpCode::Loop(JumpOffset::new(2));
        let mut chunk = Chunk::new();
        chunk.write_instruction(opcode, Span::new(8, 4));
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidJumpTarget {
                instruction_offset: 0,
                opcode,
                target: None,
                instruction_count: 2,
                span: Span::new(8, 4),
            })
        );
    }

    #[test]
    fn forward_jump_past_chunk_is_rejected() {
        let opcode = OpCode::Jump(JumpOffset::MAX);
        let mut chunk = Chunk::new();
        chunk.write_instruction(opcode, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidJumpTarget {
                instruction_offset: 0,
                opcode,
                target: Some(usize::from(u16::MAX) + 1),
                instruction_count: 2,
                span: SPAN,
            })
        );
    }

    #[test]
    fn last_instruction_is_valid_target_but_chunk_end_is_not() {
        let mut valid = Chunk::new();
        valid.write_instruction(OpCode::Jump(JumpOffset::new(1)), SPAN);
        valid.write_instruction(OpCode::True, SPAN);
        valid.write_instruction(OpCode::Return, SPAN);
        assert_eq!(Vm::new().run(&valid), Ok(Value::Null));

        let invalid_opcode = OpCode::Jump(JumpOffset::new(2));
        let mut invalid = Chunk::new();
        invalid.write_instruction(invalid_opcode, SPAN);
        invalid.write_instruction(OpCode::True, SPAN);
        invalid.write_instruction(OpCode::Return, SPAN);
        assert!(matches!(
            Vm::new().run(&invalid),
            Err(VmError::InvalidJumpTarget {
                target: Some(3),
                instruction_count: 3,
                ..
            })
        ));
    }

    #[test]
    fn jump_if_false_validates_untaken_target() {
        let opcode = OpCode::JumpIfFalse(JumpOffset::new(2));
        let mut chunk = Chunk::new();
        chunk.write_instruction(OpCode::True, SPAN);
        chunk.write_instruction(opcode, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidJumpTarget {
                instruction_offset: 1,
                opcode,
                target: Some(4),
                instruction_count: 3,
                span: SPAN,
            })
        );
    }

    #[test]
    fn vm_is_reusable_after_invalid_jump_error() {
        let mut invalid = Chunk::new();
        invalid.write_instruction(OpCode::Jump(JumpOffset::new(1)), SPAN);
        invalid.write_instruction(OpCode::Return, SPAN);

        let mut valid = Chunk::new();
        valid.write_instruction(OpCode::True, SPAN);
        valid.write_instruction(OpCode::Return, SPAN);

        let mut vm = Vm::new();
        assert!(matches!(
            vm.run(&invalid),
            Err(VmError::InvalidJumpTarget { .. })
        ));
        assert_eq!(vm.run(&valid), Ok(Value::Bool(true)));
        assert!(vm.stack.is_empty());
        assert!(vm.frames.is_empty());
    }

    #[test]
    fn executes_manual_if_else_bytecode() {
        let mut chunk = Chunk::new();
        let then_value = chunk
            .add_constant(Value::String("then".to_owned()))
            .unwrap();
        let else_value = chunk
            .add_constant(Value::String("else".to_owned()))
            .unwrap();
        chunk.write_instruction(OpCode::False, SPAN);
        chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(3)), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::Constant(then_value), SPAN);
        chunk.write_instruction(OpCode::Jump(JumpOffset::new(2)), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::Constant(else_value), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::String("else".to_owned())));
    }

    #[test]
    fn executes_manual_loop_bytecode() {
        let mut chunk = Chunk::new();
        let name = chunk
            .add_constant(Value::String("counter".to_owned()))
            .unwrap();
        let zero = chunk.add_constant(Value::Int(0)).unwrap();
        let three = chunk.add_constant(Value::Int(3)).unwrap();
        let one = chunk.add_constant(Value::Int(1)).unwrap();

        chunk.write_instruction(OpCode::Constant(zero), SPAN);
        chunk.write_instruction(OpCode::DefineGlobal(name), SPAN);
        chunk.write_instruction(OpCode::GetGlobal(name), SPAN);
        chunk.write_instruction(OpCode::Constant(three), SPAN);
        chunk.write_instruction(OpCode::Less, SPAN);
        chunk.write_instruction(OpCode::JumpIfFalse(JumpOffset::new(7)), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::GetGlobal(name), SPAN);
        chunk.write_instruction(OpCode::Constant(one), SPAN);
        chunk.write_instruction(OpCode::Add, SPAN);
        chunk.write_instruction(OpCode::SetGlobal(name), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::Loop(JumpOffset::new(11)), SPAN);
        chunk.write_instruction(OpCode::Pop, SPAN);
        chunk.write_instruction(OpCode::GetGlobal(name), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&chunk), Ok(Value::Int(3)));
    }

    #[test]
    fn invalid_get_upvalue_is_structured_error() {
        let chunk = return_chunk(OpCode::GetUpvalue(UpvalueIndex::new(0)));

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidUpvalue {
                instruction_offset: 0,
                opcode: OpCode::GetUpvalue(UpvalueIndex::new(0)),
                index: UpvalueIndex::new(0),
                upvalue_count: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn invalid_set_upvalue_is_structured_error() {
        let mut chunk = Chunk::new();
        let value = chunk.add_constant(Value::Int(1)).unwrap();
        chunk.write_instruction(OpCode::Constant(value), SPAN);
        chunk.write_instruction(OpCode::SetUpvalue(UpvalueIndex::new(0)), SPAN);

        assert!(matches!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidUpvalue {
                opcode: OpCode::SetUpvalue(_),
                upvalue_count: 0,
                ..
            })
        ));
    }

    #[test]
    fn closure_rejects_non_function_constant() {
        let mut chunk = Chunk::new();
        let value = chunk.add_constant(Value::Int(42)).unwrap();
        chunk.write_instruction(OpCode::Closure(value), SPAN);

        assert!(matches!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidClosureFunction {
                index: 0,
                value: Value::Int(42),
                ..
            })
        ));
    }

    #[test]
    fn closure_rejects_invalid_enclosing_local_capture() {
        let mut chunk = Chunk::new();
        let function = Function::with_upvalues(
            "bad",
            Arity::new(0),
            Chunk::new(),
            vec![UpvalueDescriptor::Local(LocalSlot::new(0))],
        );
        let function = chunk
            .add_constant(Value::Function(Rc::new(function)))
            .unwrap();
        chunk.write_instruction(OpCode::Closure(function), SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidCaptureLocal {
                instruction_offset: 0,
                slot: LocalSlot::new(0),
                stack_len: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn closure_rejects_invalid_enclosing_upvalue_capture() {
        let mut chunk = Chunk::new();
        let function = Function::with_upvalues(
            "bad",
            Arity::new(0),
            Chunk::new(),
            vec![UpvalueDescriptor::Upvalue(UpvalueIndex::new(0))],
        );
        let function = chunk
            .add_constant(Value::Function(Rc::new(function)))
            .unwrap();
        chunk.write_instruction(OpCode::Closure(function), SPAN);

        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::InvalidCaptureUpvalue {
                instruction_offset: 0,
                index: UpvalueIndex::new(0),
                upvalue_count: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn malformed_closure_capture_does_not_panic() {
        let mut chunk = Chunk::new();
        let function = Function::with_upvalues(
            "bad",
            Arity::new(0),
            Chunk::new(),
            vec![UpvalueDescriptor::Local(LocalSlot::MAX)],
        );
        let function = chunk
            .add_constant(Value::Function(Rc::new(function)))
            .unwrap();
        chunk.write_instruction(OpCode::Closure(function), SPAN);

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Vm::new().run(&chunk)));
        assert!(matches!(
            result,
            Ok(Err(VmError::InvalidCaptureLocal {
                slot: LocalSlot::MAX,
                ..
            }))
        ));
    }

    #[test]
    fn close_upvalue_preserves_captured_value_after_run() {
        let mut inner_chunk = Chunk::new();
        inner_chunk.write_instruction(OpCode::GetUpvalue(UpvalueIndex::new(0)), SPAN);
        inner_chunk.write_instruction(OpCode::Return, SPAN);
        let inner = Function::with_upvalues(
            "inner",
            Arity::new(0),
            inner_chunk,
            vec![UpvalueDescriptor::Local(LocalSlot::new(0))],
        );

        let mut factory = Chunk::new();
        let captured = factory.add_constant(Value::Int(42)).unwrap();
        let inner = factory
            .add_constant(Value::Function(Rc::new(inner)))
            .unwrap();
        factory.write_instruction(OpCode::Constant(captured), SPAN);
        factory.write_instruction(OpCode::Closure(inner), SPAN);
        factory.write_instruction(OpCode::CloseUpvalue(LocalSlot::new(0)), SPAN);
        factory.write_instruction(OpCode::Return, SPAN);

        let closure = Vm::new().run(&factory).unwrap();
        let mut call = Chunk::new();
        let closure = call.add_constant(closure).unwrap();
        call.write_instruction(OpCode::Constant(closure), SPAN);
        call.write_instruction(OpCode::Call(Arity::new(0)), SPAN);
        call.write_instruction(OpCode::Return, SPAN);

        assert_eq!(Vm::new().run(&call), Ok(Value::Int(42)));
    }

    #[test]
    fn vm_is_reusable_after_closure_runtime_error() {
        let bad = return_chunk(OpCode::GetUpvalue(UpvalueIndex::new(0)));
        let good = constant_chunk(Value::Int(42));
        let mut vm = Vm::new();

        assert!(matches!(vm.run(&bad), Err(VmError::InvalidUpvalue { .. })));
        assert_eq!(vm.run(&good), Ok(Value::Int(42)));
        assert!(vm.open_upvalues.is_empty());
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
        assert!(vm.frames.is_empty());
        assert!(vm.open_upvalues.is_empty());
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

    #[test]
    fn build_array_preserves_stack_source_order() {
        let mut chunk = Chunk::new();
        for value in [Value::Int(1), Value::Int(2), Value::Int(3)] {
            let value = chunk.add_constant(value).unwrap();
            chunk.write_instruction(OpCode::Constant(value), SPAN);
        }
        chunk.write_instruction(OpCode::BuildArray(AggregateCount::new(3)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        let mut vm = Vm::new();
        let value = vm.run(&chunk).unwrap();
        assert_eq!(vm.format_value(&value).unwrap(), "[1, 2, 3]");
    }

    #[test]
    fn malformed_array_build_reports_stack_underflow() {
        let chunk = return_chunk(OpCode::BuildArray(AggregateCount::new(2)));
        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 0,
                opcode: OpCode::BuildArray(AggregateCount::new(2)),
                needed: 2,
                available: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn malformed_map_build_reports_stack_underflow() {
        let chunk = return_chunk(OpCode::BuildMap(AggregateCount::new(1)));
        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 0,
                opcode: OpCode::BuildMap(AggregateCount::new(1)),
                needed: 2,
                available: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn map_build_rejects_non_string_key_without_panicking() {
        let mut chunk = Chunk::new();
        let key = chunk.add_constant(Value::Int(1)).unwrap();
        let value = chunk.add_constant(Value::Int(2)).unwrap();
        chunk.write_instruction(OpCode::Constant(key), SPAN);
        chunk.write_instruction(OpCode::Constant(value), SPAN);
        chunk.write_instruction(OpCode::BuildMap(AggregateCount::new(1)), SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Vm::new().run(&chunk)));
        assert!(matches!(result, Ok(Err(VmError::InvalidMapKey { .. }))));
    }

    #[test]
    fn malformed_set_index_reports_stack_underflow() {
        let chunk = return_chunk(OpCode::SetIndex);
        assert_eq!(
            Vm::new().run(&chunk),
            Err(VmError::StackUnderflow {
                instruction_offset: 0,
                opcode: OpCode::SetIndex,
                needed: 3,
                available: 0,
                span: SPAN,
            })
        );
    }

    #[test]
    fn explicit_gc_preserves_global_and_collects_unrooted_object() {
        let mut vm = Vm::new();
        run_source(
            &mut vm,
            "let kept = [1]; let garbage = [2]; garbage = null;",
        );
        assert_eq!(vm.heap_live_count(), 2);
        let stats = vm.collect_garbage().unwrap();
        assert_eq!(stats.after, 1);
        let kept = read_global_value(&mut vm, "kept");
        assert_eq!(vm.format_value(&kept).unwrap(), "[1]");
    }

    #[test]
    fn removing_final_global_root_allows_collection() {
        let mut vm = Vm::new();
        run_source(&mut vm, "let kept = [1];");
        run_source(&mut vm, "kept = null;");
        assert_eq!(vm.collect_garbage().unwrap().after, 0);
    }

    #[test]
    fn operand_stack_value_survives_explicit_gc() {
        let mut vm = Vm::new();
        let reference = vm.heap.allocate(HeapObject::Array(Vec::new())).unwrap();
        vm.stack.push(Value::Heap(reference));
        assert_eq!(vm.collect_garbage().unwrap().after, 1);
        assert!(vm.heap.get(reference).is_ok());
    }

    #[test]
    fn automatic_gc_triggers_at_object_threshold() {
        let mut vm = Vm::new();
        vm.set_gc_threshold(1);
        run_source(&mut vm, "[]; []; []; []; [];");
        assert_eq!(vm.heap_live_count(), 1);
        assert_eq!(vm.heap_capacity(), 1);
        assert_eq!(vm.gc_threshold(), 1);
    }

    #[test]
    fn low_threshold_preserves_nested_allocation_operands() {
        let mut vm = Vm::new();
        vm.set_gc_threshold(1);
        run_source(&mut vm, "let root = [[[1]], [[2]], [[3]]];");
        let root = read_global_value(&mut vm, "root");
        assert_eq!(vm.format_value(&root).unwrap(), "[[[1]], [[2]], [[3]]]");
    }

    #[test]
    fn function_argument_aggregate_survives_gc_during_call() {
        let mut vm = Vm::new();
        vm.set_gc_threshold(1);
        run_source(
            &mut vm,
            "fn keep(value) { let temp = []; return value[0]; } let result = keep([42]);",
        );
        assert_eq!(read_global_value(&mut vm, "result"), Value::Int(42));
    }

    #[test]
    fn function_local_aggregate_survives_gc_during_call() {
        let mut vm = Vm::new();
        vm.set_gc_threshold(1);
        run_source(
            &mut vm,
            "fn keep() { let value = [42]; let temp = []; return value[0]; } let result = keep();",
        );
        assert_eq!(read_global_value(&mut vm, "result"), Value::Int(42));
    }

    #[test]
    fn returned_aggregate_survives_after_caller_receives_it() {
        let mut vm = Vm::new();
        vm.set_gc_threshold(1);
        run_source(
            &mut vm,
            "fn make() { return [42]; } let returned = make(); let temp = []; let result = returned[0];",
        );
        assert_eq!(read_global_value(&mut vm, "result"), Value::Int(42));
    }

    #[test]
    fn closed_upvalue_aggregate_survives_explicit_collection() {
        let mut vm = Vm::new();
        run_source(
            &mut vm,
            concat!(
                "fn make() { let values = [42]; ",
                "fn get() { return values[0]; } return get; } let get = make();"
            ),
        );
        vm.collect_garbage().unwrap();
        run_source(&mut vm, "let result = get();");
        assert_eq!(read_global_value(&mut vm, "result"), Value::Int(42));
    }

    #[test]
    fn captured_map_survives_collection_between_invocations() {
        let mut vm = Vm::new();
        run_source(
            &mut vm,
            concat!(
                "fn make() { let value = { answer: 42 }; ",
                "fn get() { return value.answer; } return get; } let get = make();"
            ),
        );
        vm.collect_garbage().unwrap();
        run_source(&mut vm, "let result = get();");
        assert_eq!(read_global_value(&mut vm, "result"), Value::Int(42));
    }

    #[test]
    fn alias_identity_and_mutation_survive_collection() {
        let mut vm = Vm::new();
        run_source(&mut vm, "let a = [1]; let b = a;");
        vm.collect_garbage().unwrap();
        run_source(&mut vm, "b[0] = 9; let result = a[0];");
        assert_eq!(read_global_value(&mut vm, "result"), Value::Int(9));
    }

    #[test]
    fn recursion_with_allocations_survives_aggressive_gc() {
        let mut vm = Vm::new();
        vm.set_gc_threshold(1);
        run_source(
            &mut vm,
            concat!(
                "fn descend(n, kept) { let temp = []; ",
                "if (n == 0) { return kept[0]; } return descend(n - 1, kept); } ",
                "let result = descend(20, [42]);"
            ),
        );
        assert_eq!(read_global_value(&mut vm, "result"), Value::Int(42));
    }

    #[test]
    fn stale_heap_reference_becomes_structured_vm_error() {
        let mut vm = Vm::new();
        let stale = vm.heap.allocate(HeapObject::Array(Vec::new())).unwrap();
        vm.collect_garbage().unwrap();
        let mut chunk = Chunk::new();
        let target = chunk.add_constant(Value::Heap(stale)).unwrap();
        let index = chunk.add_constant(Value::Int(0)).unwrap();
        chunk.write_instruction(OpCode::Constant(target), SPAN);
        chunk.write_instruction(OpCode::Constant(index), SPAN);
        chunk.write_instruction(OpCode::GetIndex, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        assert!(matches!(
            vm.run(&chunk),
            Err(VmError::HeapAccess {
                error: HeapError::StaleReference { .. },
                ..
            })
        ));
    }

    #[test]
    fn slot_reuse_keeps_language_identity_distinct() {
        let mut vm = Vm::new();
        let old = vm.heap.allocate(HeapObject::Array(Vec::new())).unwrap();
        vm.collect_garbage().unwrap();
        let new = vm.heap.allocate(HeapObject::Array(Vec::new())).unwrap();
        assert_eq!(old.slot(), new.slot());
        assert_ne!(Value::Heap(old), Value::Heap(new));
    }

    #[test]
    fn invalid_heap_reference_is_structured_at_runtime() {
        let mut vm = Vm::new();
        let invalid = HeapRef::new(99, 0);
        let mut chunk = Chunk::new();
        let target = chunk.add_constant(Value::Heap(invalid)).unwrap();
        let index = chunk.add_constant(Value::Int(0)).unwrap();
        chunk.write_instruction(OpCode::Constant(target), SPAN);
        chunk.write_instruction(OpCode::Constant(index), SPAN);
        chunk.write_instruction(OpCode::GetIndex, SPAN);
        chunk.write_instruction(OpCode::Return, SPAN);
        assert!(matches!(
            vm.run(&chunk),
            Err(VmError::HeapAccess {
                error: HeapError::InvalidReference { .. },
                ..
            })
        ));
    }
}

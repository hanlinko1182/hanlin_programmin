//! Bytecode types and the experimental stack virtual machine for Hanlin.
//!
//! Bytecode can be constructed manually or compiled from Hanlin's supported
//! AST subset, including named closures, lexical upvalues, iterative call
//! frames, and shared heap-backed arrays and maps. There is no VM CLI path yet,
//! so the tree-walking interpreter remains the default language execution
//! engine.

pub mod chunk;
pub mod closure;
pub mod compiler;
pub mod disassembler;
pub mod function;
pub mod heap_object;
#[path = "vm.rs"]
mod machine;
pub mod opcode;
pub mod value;

pub use chunk::{Chunk, ChunkError, Instruction};
pub use closure::Closure;
pub use compiler::{CompileError, Compiler, MAX_UPVALUES};
pub use disassembler::{disassemble_chunk, disassemble_instruction, DisassembleError};
pub use function::{Function, UpvalueDescriptor};
pub use heap_object::{HeapObject, HeapObjectRef};
pub use machine::{Vm, VmError, MAX_CALL_FRAMES};
pub use opcode::{
    AggregateCount, AggregateCountError, Arity, ArityError, ConstantIndex, JumpOffset,
    JumpOffsetError, LocalSlot, LocalSlotError, OpCode, UpvalueIndex, UpvalueIndexError,
};
pub use value::Value;

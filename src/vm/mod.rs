//! Bytecode types and the experimental stack virtual machine for Hanlin.
//!
//! Bytecode can be constructed manually or compiled from Hanlin's supported
//! AST subset, including named functions and iterative call frames. There is
//! no VM CLI path yet, so the tree-walking interpreter remains the default
//! language execution engine.

pub mod chunk;
pub mod compiler;
pub mod disassembler;
pub mod function;
#[path = "vm.rs"]
mod machine;
pub mod opcode;
pub mod value;

pub use chunk::{Chunk, ChunkError, Instruction};
pub use compiler::{CompileError, Compiler};
pub use disassembler::{disassemble_chunk, disassemble_instruction, DisassembleError};
pub use function::Function;
pub use machine::{Vm, VmError, MAX_CALL_FRAMES};
pub use opcode::{
    Arity, ArityError, ConstantIndex, JumpOffset, JumpOffsetError, LocalSlot, LocalSlotError,
    OpCode,
};
pub use value::Value;

//! Bytecode types and the experimental stack virtual machine for Hanlin.
//!
//! Bytecode can be constructed manually or compiled from Hanlin's initial
//! supported AST subset. There is no VM CLI path yet, so the tree-walking
//! interpreter remains the default language execution engine.

pub mod chunk;
pub mod compiler;
pub mod disassembler;
#[path = "vm.rs"]
mod machine;
pub mod opcode;
pub mod value;

pub use chunk::{Chunk, ChunkError, Instruction};
pub use compiler::{CompileError, Compiler};
pub use disassembler::{disassemble_chunk, disassemble_instruction, DisassembleError};
pub use machine::{Vm, VmError};
pub use opcode::{ConstantIndex, JumpOffset, JumpOffsetError, LocalSlot, LocalSlotError, OpCode};
pub use value::Value;

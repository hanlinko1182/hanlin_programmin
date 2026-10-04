//! Bytecode types and the experimental stack virtual machine for Hanlin.
//!
//! The VM currently executes only manually constructed foundational bytecode.
//! Hanlin has no bytecode compiler or VM CLI path yet, so the tree-walking
//! interpreter remains the default language execution engine.

pub mod chunk;
pub mod disassembler;
#[path = "vm.rs"]
mod machine;
pub mod opcode;
pub mod value;

pub use chunk::{Chunk, ChunkError, Instruction};
pub use disassembler::{disassemble_chunk, disassemble_instruction, DisassembleError};
pub use machine::{Vm, VmError};
pub use opcode::{ConstantIndex, OpCode};
pub use value::Value;

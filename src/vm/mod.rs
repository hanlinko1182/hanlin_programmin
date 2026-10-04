//! Foundational bytecode types for Hanlin's future stack-based virtual machine.
//!
//! This module defines a bytecode representation only. The tree-walking
//! interpreter remains Hanlin's execution engine; bytecode compilation and
//! execution will be added in later VM phases.

pub mod chunk;
pub mod disassembler;
pub mod opcode;
pub mod value;

pub use chunk::{Chunk, ChunkError, Instruction};
pub use disassembler::{disassemble_chunk, disassemble_instruction, DisassembleError};
pub use opcode::{ConstantIndex, OpCode};
pub use value::Value;

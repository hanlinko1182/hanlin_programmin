/// An index into a [`Chunk`](super::chunk::Chunk)'s constant pool.
///
/// The explicit width prevents a future bytecode encoder from accidentally
/// truncating a platform-sized index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConstantIndex(u32);

impl ConstantIndex {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

/// A single operation understood by Hanlin's future stack-based VM.
///
/// Variants carry their operands directly, as `Constant` does here. This
/// keeps the representation strongly typed while leaving room for later
/// operands such as local slots, jump offsets, and call arity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpCode {
    Constant(ConstantIndex),
    Null,
    True,
    False,
    Pop,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Negate,
    Not,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    DefineGlobal(ConstantIndex),
    GetGlobal(ConstantIndex),
    SetGlobal(ConstantIndex),
    Return,
}

use std::fmt;

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

/// A stack-relative local variable slot.
///
/// Slots use an explicit, compact width suitable for bytecode operands. Use
/// [`TryFrom<usize>`] when converting a platform-sized index so values outside
/// the supported range are rejected instead of truncated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalSlot(u16);

impl LocalSlot {
    pub const MAX: Self = Self(u16::MAX);

    pub const fn new(slot: u16) -> Self {
        Self(slot)
    }

    pub const fn as_u16(self) -> u16 {
        self.0
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u16> for LocalSlot {
    fn from(slot: u16) -> Self {
        Self::new(slot)
    }
}

impl TryFrom<usize> for LocalSlot {
    type Error = LocalSlotError;

    fn try_from(slot: usize) -> Result<Self, Self::Error> {
        u16::try_from(slot)
            .map(Self::new)
            .map_err(|_| LocalSlotError { slot })
    }
}

/// A platform-sized local slot index that cannot fit in [`LocalSlot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalSlotError {
    slot: usize,
}

impl LocalSlotError {
    pub const fn slot(self) -> usize {
        self.slot
    }
}

impl fmt::Display for LocalSlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "local slot {} exceeds the maximum supported slot {}",
            self.slot,
            LocalSlot::MAX.as_u16()
        )
    }
}

impl std::error::Error for LocalSlotError {}

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
    GetLocal(LocalSlot),
    SetLocal(LocalSlot),
    DefineGlobal(ConstantIndex),
    GetGlobal(ConstantIndex),
    SetGlobal(ConstantIndex),
    Return,
}

#[cfg(test)]
mod tests {
    use super::{LocalSlot, LocalSlotError};

    #[test]
    fn constructs_valid_local_slot() {
        let slot = LocalSlot::new(42);

        assert_eq!(slot.as_u16(), 42);
        assert_eq!(slot.as_usize(), 42);
    }

    #[test]
    fn local_slot_supports_equality_and_debugging() {
        let slot = LocalSlot::new(7);

        assert_eq!(slot, LocalSlot::from(7));
        assert_eq!(format!("{slot:?}"), "LocalSlot(7)");
    }

    #[test]
    fn supports_maximum_local_slot() {
        let slot = LocalSlot::try_from(usize::from(u16::MAX)).unwrap();

        assert_eq!(slot, LocalSlot::MAX);
        assert_eq!(slot.as_u16(), u16::MAX);
    }

    #[test]
    fn rejects_local_slot_overflow() {
        let attempted = usize::from(u16::MAX) + 1;
        let error = LocalSlot::try_from(attempted).unwrap_err();

        assert_eq!(error, LocalSlotError { slot: attempted });
        assert_eq!(error.slot(), attempted);
    }
}

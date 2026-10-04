use std::fmt;

/// Number of array elements or map pairs consumed by a build instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AggregateCount(u16);

impl AggregateCount {
    pub const MAX: Self = Self(u16::MAX);

    pub const fn new(count: u16) -> Self {
        Self(count)
    }

    pub const fn as_u16(self) -> u16 {
        self.0
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<usize> for AggregateCount {
    type Error = AggregateCountError;

    fn try_from(count: usize) -> Result<Self, Self::Error> {
        u16::try_from(count)
            .map(Self::new)
            .map_err(|_| AggregateCountError { count })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AggregateCountError {
    count: usize,
}

impl AggregateCountError {
    pub const fn count(self) -> usize {
        self.count
    }
}

impl fmt::Display for AggregateCountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "aggregate count {} exceeds the maximum supported count {}",
            self.count,
            AggregateCount::MAX.as_u16()
        )
    }
}

impl std::error::Error for AggregateCountError {}

/// The number of arguments supplied by a call instruction.
///
/// The explicit byte-sized representation keeps the bytecode operand bounded.
/// Convert from [`usize`] with [`TryFrom`] so oversized call sites are rejected
/// instead of silently truncated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Arity(u8);

impl Arity {
    pub const MAX: Self = Self(u8::MAX);

    pub const fn new(arity: u8) -> Self {
        Self(arity)
    }

    pub const fn as_u8(self) -> u8 {
        self.0
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u8> for Arity {
    fn from(arity: u8) -> Self {
        Self::new(arity)
    }
}

impl TryFrom<usize> for Arity {
    type Error = ArityError;

    fn try_from(arity: usize) -> Result<Self, Self::Error> {
        u8::try_from(arity)
            .map(Self::new)
            .map_err(|_| ArityError { arity })
    }
}

/// A platform-sized argument count that cannot fit in [`Arity`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArityError {
    arity: usize,
}

impl ArityError {
    pub const fn arity(self) -> usize {
        self.arity
    }
}

impl fmt::Display for ArityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "arity {} exceeds the maximum supported arity {}",
            self.arity,
            Arity::MAX.as_u8()
        )
    }
}

impl std::error::Error for ArityError {}

/// An index into a closure's bounded upvalue array.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UpvalueIndex(u8);

impl UpvalueIndex {
    pub const MAX: Self = Self(u8::MAX);

    pub const fn new(index: u8) -> Self {
        Self(index)
    }

    pub const fn as_u8(self) -> u8 {
        self.0
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl TryFrom<usize> for UpvalueIndex {
    type Error = UpvalueIndexError;

    fn try_from(index: usize) -> Result<Self, Self::Error> {
        u8::try_from(index)
            .map(Self::new)
            .map_err(|_| UpvalueIndexError { index })
    }
}

/// A platform-sized capture index that cannot fit in [`UpvalueIndex`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpvalueIndexError {
    index: usize,
}

impl UpvalueIndexError {
    pub const fn index(self) -> usize {
        self.index
    }
}

impl fmt::Display for UpvalueIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "upvalue index {} exceeds the maximum supported index {}",
            self.index,
            UpvalueIndex::MAX.as_u8()
        )
    }
}

impl std::error::Error for UpvalueIndexError {}

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

/// An instruction-relative distance used by control-flow bytecode.
///
/// The VM applies this distance to the instruction pointer after the jump
/// instruction has been fetched. Use [`TryFrom<usize>`] for checked conversion
/// from indexes or distances calculated by future compiler backpatching.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JumpOffset(u16);

impl JumpOffset {
    pub const MAX: Self = Self(u16::MAX);

    pub const fn new(offset: u16) -> Self {
        Self(offset)
    }

    pub const fn as_u16(self) -> u16 {
        self.0
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u16> for JumpOffset {
    fn from(offset: u16) -> Self {
        Self::new(offset)
    }
}

impl TryFrom<usize> for JumpOffset {
    type Error = JumpOffsetError;

    fn try_from(offset: usize) -> Result<Self, Self::Error> {
        u16::try_from(offset)
            .map(Self::new)
            .map_err(|_| JumpOffsetError { offset })
    }
}

/// A platform-sized jump distance that cannot fit in [`JumpOffset`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JumpOffsetError {
    offset: usize,
}

impl JumpOffsetError {
    pub const fn offset(self) -> usize {
        self.offset
    }
}

impl fmt::Display for JumpOffsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "jump offset {} exceeds the maximum supported offset {}",
            self.offset,
            JumpOffset::MAX.as_u16()
        )
    }
}

impl std::error::Error for JumpOffsetError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JumpDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JumpTargetError {
    pub(crate) target: Option<usize>,
}

/// Resolves a jump relative to the post-fetch instruction pointer.
///
/// Targets must identify an existing instruction. In particular, an offset
/// resolving exactly to `instruction_count` is rejected rather than treated
/// as implicit program termination.
pub(crate) fn resolve_jump_target(
    instruction_offset: usize,
    offset: JumpOffset,
    direction: JumpDirection,
    instruction_count: usize,
) -> Result<usize, JumpTargetError> {
    let post_fetch_ip = instruction_offset
        .checked_add(1)
        .ok_or(JumpTargetError { target: None })?;
    let target = match direction {
        JumpDirection::Forward => post_fetch_ip.checked_add(offset.as_usize()),
        JumpDirection::Backward => post_fetch_ip.checked_sub(offset.as_usize()),
    }
    .ok_or(JumpTargetError { target: None })?;

    if target < instruction_count {
        Ok(target)
    } else {
        Err(JumpTargetError {
            target: Some(target),
        })
    }
}

/// A single operation understood by Hanlin's future stack-based VM.
///
/// Variants carry their operands directly. This keeps constant indexes, local
/// slots, upvalue indexes, jump distances, and call arities strongly typed.
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
    Jump(JumpOffset),
    JumpIfFalse(JumpOffset),
    Loop(JumpOffset),
    Closure(ConstantIndex),
    GetUpvalue(UpvalueIndex),
    SetUpvalue(UpvalueIndex),
    CloseUpvalue(LocalSlot),
    BuildArray(AggregateCount),
    BuildMap(AggregateCount),
    GetIndex,
    SetIndex,
    Call(Arity),
    Return,
}

#[cfg(test)]
mod tests {
    use super::{
        AggregateCount, AggregateCountError, Arity, ArityError, JumpOffset, JumpOffsetError,
        LocalSlot, LocalSlotError, UpvalueIndex, UpvalueIndexError,
    };

    #[test]
    fn constructs_and_checks_upvalue_index() {
        assert_eq!(UpvalueIndex::new(7).as_usize(), 7);
        let attempted = usize::from(u8::MAX) + 1;
        let error = UpvalueIndex::try_from(attempted).unwrap_err();
        assert_eq!(error, UpvalueIndexError { index: attempted });
        assert_eq!(error.index(), attempted);
    }

    #[test]
    fn constructs_valid_arity() {
        let arity = Arity::new(3);

        assert_eq!(arity.as_u8(), 3);
        assert_eq!(arity.as_usize(), 3);
    }

    #[test]
    fn rejects_arity_overflow() {
        let attempted = usize::from(u8::MAX) + 1;
        let error = Arity::try_from(attempted).unwrap_err();

        assert_eq!(error, ArityError { arity: attempted });
        assert_eq!(error.arity(), attempted);
    }

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

    #[test]
    fn constructs_valid_jump_offset() {
        let offset = JumpOffset::new(42);

        assert_eq!(offset.as_u16(), 42);
        assert_eq!(offset.as_usize(), 42);
    }

    #[test]
    fn jump_offset_supports_equality_and_debugging() {
        let offset = JumpOffset::new(7);

        assert_eq!(offset, JumpOffset::from(7));
        assert_eq!(format!("{offset:?}"), "JumpOffset(7)");
    }

    #[test]
    fn supports_maximum_jump_offset() {
        let offset = JumpOffset::try_from(usize::from(u16::MAX)).unwrap();

        assert_eq!(offset, JumpOffset::MAX);
        assert_eq!(offset.as_u16(), u16::MAX);
    }

    #[test]
    fn rejects_jump_offset_overflow() {
        let attempted = usize::from(u16::MAX) + 1;
        let error = JumpOffset::try_from(attempted).unwrap_err();

        assert_eq!(error, JumpOffsetError { offset: attempted });
        assert_eq!(error.offset(), attempted);
    }

    #[test]
    fn constructs_checked_aggregate_count() {
        let count = AggregateCount::try_from(42_usize).unwrap();
        assert_eq!(count.as_u16(), 42);
        assert_eq!(count.as_usize(), 42);
    }

    #[test]
    fn rejects_aggregate_count_overflow() {
        let attempted = usize::from(u16::MAX) + 1;
        let error = AggregateCount::try_from(attempted).unwrap_err();
        assert_eq!(error, AggregateCountError { count: attempted });
        assert_eq!(error.count(), attempted);
    }
}

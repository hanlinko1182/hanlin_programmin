//! Non-moving mark-and-sweep heap for VM aggregate objects.
//!
//! Objects live in stable slots and [`HeapRef`] carries both a slot index and
//! generation. Sweeping releases unreachable payloads without compacting the
//! arena; reused slots receive a new generation so stale handles cannot alias
//! new objects. Marking uses an explicit work list and traces arrays, maps,
//! closures, upvalues, functions, and function constants without recursive
//! graph traversal. Closures and upvalues remain `Rc`-managed in VM-15: heap
//! handles are plain values, so mixed closure/aggregate cycles contain no
//! owning edge back into the heap and are reclaimed when unreachable.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use super::closure::{Upvalue, UpvalueState};
use super::{Closure, Function, Value};

/// Stable identity for one heap allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HeapRef {
    slot: u32,
    generation: u64,
}

impl HeapRef {
    pub const fn new(slot: u32, generation: u64) -> Self {
        Self { slot, generation }
    }

    pub const fn slot(self) -> u32 {
        self.slot
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// Aggregate payload managed exclusively by [`Heap`].
#[derive(Clone, Debug, PartialEq)]
pub enum HeapObject {
    Array(Vec<Value>),
    Map(HashMap<String, Value>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeapError {
    CapacityOverflow,
    InvalidReference {
        reference: HeapRef,
        capacity: usize,
    },
    StaleReference {
        reference: HeapRef,
        current_generation: u64,
        occupied: bool,
    },
    InvalidOpenUpvalue {
        stack_index: usize,
        stack_len: usize,
    },
}

impl fmt::Display for HeapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityOverflow => write!(f, "VM heap exceeds u32 slot capacity"),
            Self::InvalidReference {
                reference,
                capacity,
            } => write!(
                f,
                "heap reference {}:{} is outside heap capacity {capacity}",
                reference.slot, reference.generation
            ),
            Self::StaleReference {
                reference,
                current_generation,
                occupied,
            } => write!(
                f,
                "heap reference {}:{} is stale (slot generation {current_generation}, occupied={occupied})",
                reference.slot, reference.generation
            ),
            Self::InvalidOpenUpvalue {
                stack_index,
                stack_len,
            } => write!(
                f,
                "open upvalue points to stack index {stack_index}, but the stack has {stack_len} values"
            ),
        }
    }
}

impl std::error::Error for HeapError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GcStats {
    pub before: usize,
    pub after: usize,
    pub collected: usize,
}

#[derive(Debug)]
struct HeapEntry {
    marked: bool,
    object: HeapObject,
}

#[derive(Debug)]
struct HeapSlot {
    generation: u64,
    entry: Option<HeapEntry>,
}

/// VM-owned, non-compacting heap arena.
#[derive(Debug, Default)]
pub struct Heap {
    slots: Vec<HeapSlot>,
    free_slots: Vec<usize>,
    live_count: usize,
}

enum TraceWork {
    Value(Value),
    Object(HeapRef),
    Closure(Rc<Closure>),
    Function(Rc<Function>),
    Upvalue(Upvalue),
}

impl Heap {
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_slots: Vec::new(),
            live_count: 0,
        }
    }

    pub fn allocate(&mut self, object: HeapObject) -> Result<HeapRef, HeapError> {
        let reference = if let Some(slot_index) = self.free_slots.pop() {
            let slot = &mut self.slots[slot_index];
            debug_assert!(slot.entry.is_none());
            slot.entry = Some(HeapEntry {
                marked: false,
                object,
            });
            HeapRef::new(
                u32::try_from(slot_index).map_err(|_| HeapError::CapacityOverflow)?,
                slot.generation,
            )
        } else {
            let slot_index = self.slots.len();
            let slot = u32::try_from(slot_index).map_err(|_| HeapError::CapacityOverflow)?;
            self.slots.push(HeapSlot {
                generation: 0,
                entry: Some(HeapEntry {
                    marked: false,
                    object,
                }),
            });
            HeapRef::new(slot, 0)
        };
        self.live_count += 1;
        Ok(reference)
    }

    pub fn get(&self, reference: HeapRef) -> Result<&HeapObject, HeapError> {
        Ok(&self.entry(reference)?.object)
    }

    pub fn get_mut(&mut self, reference: HeapRef) -> Result<&mut HeapObject, HeapError> {
        Ok(&mut self.entry_mut(reference)?.object)
    }

    pub const fn live_count(&self) -> usize {
        self.live_count
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn format_value(&self, value: &Value) -> Result<String, HeapError> {
        let mut output = String::new();
        self.format_value_into(value, false, &mut HashSet::new(), &mut output)?;
        Ok(output)
    }

    pub(crate) fn collect(
        &mut self,
        roots: &[Value],
        stack: &[Value],
        root_upvalues: &[Upvalue],
    ) -> Result<GcStats, HeapError> {
        for slot in &mut self.slots {
            if let Some(entry) = &mut slot.entry {
                entry.marked = false;
            }
        }

        let before = self.live_count;
        let mut work = Vec::new();
        work.extend(roots.iter().cloned().map(TraceWork::Value));
        work.extend(root_upvalues.iter().cloned().map(TraceWork::Upvalue));
        let mut seen_closures = HashSet::new();
        let mut seen_functions = HashSet::new();
        let mut seen_upvalues = HashSet::new();

        while let Some(item) = work.pop() {
            match item {
                TraceWork::Value(value) => match value {
                    Value::Heap(reference) => {
                        let entry = self.entry_mut(reference)?;
                        if !entry.marked {
                            entry.marked = true;
                            work.push(TraceWork::Object(reference));
                        }
                    }
                    Value::Closure(closure) => work.push(TraceWork::Closure(closure)),
                    Value::Function(function) => work.push(TraceWork::Function(function)),
                    Value::Null
                    | Value::Bool(_)
                    | Value::Int(_)
                    | Value::Float(_)
                    | Value::String(_) => {}
                },
                TraceWork::Object(reference) => {
                    let children: Vec<Value> = match self.get(reference)? {
                        HeapObject::Array(values) => values.clone(),
                        HeapObject::Map(entries) => entries.values().cloned().collect(),
                    };
                    work.extend(children.into_iter().map(TraceWork::Value));
                }
                TraceWork::Closure(closure) => {
                    let identity = Rc::as_ptr(&closure) as usize;
                    if seen_closures.insert(identity) {
                        work.push(TraceWork::Function(Rc::clone(closure.function())));
                        work.extend(closure.upvalues().iter().cloned().map(TraceWork::Upvalue));
                    }
                }
                TraceWork::Function(function) => {
                    let identity = Rc::as_ptr(&function) as usize;
                    if seen_functions.insert(identity) {
                        work.extend(
                            function
                                .chunk()
                                .constants()
                                .iter()
                                .cloned()
                                .map(TraceWork::Value),
                        );
                    }
                }
                TraceWork::Upvalue(upvalue) => {
                    let identity = Rc::as_ptr(&upvalue) as usize;
                    if seen_upvalues.insert(identity) {
                        match &*upvalue.borrow() {
                            UpvalueState::Open(stack_index) => {
                                let value = stack.get(*stack_index).cloned().ok_or(
                                    HeapError::InvalidOpenUpvalue {
                                        stack_index: *stack_index,
                                        stack_len: stack.len(),
                                    },
                                )?;
                                work.push(TraceWork::Value(value));
                            }
                            UpvalueState::Closed(value) => {
                                work.push(TraceWork::Value(value.clone()));
                            }
                        }
                    }
                }
            }
        }

        self.sweep();
        Ok(GcStats {
            before,
            after: self.live_count,
            collected: before - self.live_count,
        })
    }

    fn entry(&self, reference: HeapRef) -> Result<&HeapEntry, HeapError> {
        let slot = self.slot(reference)?;
        slot.entry.as_ref().ok_or(HeapError::StaleReference {
            reference,
            current_generation: slot.generation,
            occupied: false,
        })
    }

    fn entry_mut(&mut self, reference: HeapRef) -> Result<&mut HeapEntry, HeapError> {
        let slot = self.slot_mut(reference)?;
        let generation = slot.generation;
        slot.entry.as_mut().ok_or(HeapError::StaleReference {
            reference,
            current_generation: generation,
            occupied: false,
        })
    }

    fn slot(&self, reference: HeapRef) -> Result<&HeapSlot, HeapError> {
        let index = reference.slot as usize;
        let slot = self.slots.get(index).ok_or(HeapError::InvalidReference {
            reference,
            capacity: self.slots.len(),
        })?;
        if slot.generation != reference.generation {
            return Err(HeapError::StaleReference {
                reference,
                current_generation: slot.generation,
                occupied: slot.entry.is_some(),
            });
        }
        Ok(slot)
    }

    fn slot_mut(&mut self, reference: HeapRef) -> Result<&mut HeapSlot, HeapError> {
        let capacity = self.slots.len();
        let index = reference.slot as usize;
        let slot = self
            .slots
            .get_mut(index)
            .ok_or(HeapError::InvalidReference {
                reference,
                capacity,
            })?;
        if slot.generation != reference.generation {
            return Err(HeapError::StaleReference {
                reference,
                current_generation: slot.generation,
                occupied: slot.entry.is_some(),
            });
        }
        Ok(slot)
    }

    fn sweep(&mut self) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let Some(entry) = &mut slot.entry else {
                continue;
            };
            if entry.marked {
                entry.marked = false;
                continue;
            }

            slot.entry = None;
            self.live_count -= 1;
            if let Some(generation) = slot.generation.checked_add(1) {
                slot.generation = generation;
                self.free_slots.push(index);
            }
        }
    }

    fn format_value_into(
        &self,
        value: &Value,
        quote_string: bool,
        active: &mut HashSet<HeapRef>,
        output: &mut String,
    ) -> Result<(), HeapError> {
        use fmt::Write;

        match value {
            Value::Null => output.push_str("null"),
            Value::Bool(value) => write!(output, "{value}").expect("writing to String cannot fail"),
            Value::Int(value) => write!(output, "{value}").expect("writing to String cannot fail"),
            Value::Float(value) => {
                write!(output, "{value}").expect("writing to String cannot fail")
            }
            Value::String(value) if quote_string => {
                write!(output, "\"{value}\"").expect("writing to String cannot fail");
            }
            Value::String(value) => output.push_str(value),
            Value::Function(function) => {
                write!(output, "<fn {}>", function.name()).expect("writing to String cannot fail");
            }
            Value::Closure(closure) => {
                write!(output, "<fn {}>", closure.function().name())
                    .expect("writing to String cannot fail");
            }
            Value::Heap(reference) => {
                if !active.insert(*reference) {
                    output.push_str("<cycle>");
                    return Ok(());
                }
                match self.get(*reference)? {
                    HeapObject::Array(elements) => {
                        output.push('[');
                        for (index, element) in elements.iter().enumerate() {
                            if index > 0 {
                                output.push_str(", ");
                            }
                            self.format_value_into(element, true, active, output)?;
                        }
                        output.push(']');
                    }
                    HeapObject::Map(entries) => {
                        let mut entries: Vec<_> = entries.iter().collect();
                        entries.sort_by_key(|(key, _)| *key);
                        output.push_str("{ ");
                        for (index, (key, value)) in entries.into_iter().enumerate() {
                            if index > 0 {
                                output.push_str(", ");
                            }
                            write!(output, "{key}: ").expect("writing to String cannot fail");
                            self.format_value_into(value, true, active, output)?;
                        }
                        output.push_str(" }");
                    }
                }
                active.remove(reference);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{GcStats, Heap, HeapError, HeapObject, HeapRef};
    use crate::vm::closure::UpvalueState;
    use crate::vm::{Arity, Chunk, Closure, Function, Value};

    fn array(heap: &mut Heap, values: Vec<Value>) -> HeapRef {
        heap.allocate(HeapObject::Array(values)).unwrap()
    }

    fn map(heap: &mut Heap, values: impl IntoIterator<Item = (&'static str, Value)>) -> HeapRef {
        heap.allocate(HeapObject::Map(
            values
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        ))
        .unwrap()
    }

    fn collect(heap: &mut Heap, roots: &[Value]) -> GcStats {
        heap.collect(roots, &[], &[]).unwrap()
    }

    #[test]
    fn new_heap_has_no_live_objects_or_capacity() {
        let heap = Heap::new();
        assert_eq!(heap.live_count(), 0);
        assert_eq!(heap.capacity(), 0);
    }

    #[test]
    fn allocates_and_reads_array() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, vec![Value::Int(1)]);
        assert_eq!(
            heap.get(reference),
            Ok(&HeapObject::Array(vec![Value::Int(1)]))
        );
        assert_eq!(heap.live_count(), 1);
    }

    #[test]
    fn allocates_and_reads_map() {
        let mut heap = Heap::new();
        let reference = map(&mut heap, [("answer", Value::Int(42))]);
        let HeapObject::Map(entries) = heap.get(reference).unwrap() else {
            panic!("expected map")
        };
        assert_eq!(entries.get("answer"), Some(&Value::Int(42)));
    }

    #[test]
    fn mutates_object_through_checked_lookup() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, vec![Value::Int(1)]);
        let HeapObject::Array(values) = heap.get_mut(reference).unwrap() else {
            panic!("expected array")
        };
        values[0] = Value::Int(9);
        assert_eq!(heap.format_value(&Value::Heap(reference)).unwrap(), "[9]");
    }

    #[test]
    fn rejects_reference_outside_arena() {
        let heap = Heap::new();
        let reference = HeapRef::new(3, 0);
        assert_eq!(
            heap.get(reference),
            Err(HeapError::InvalidReference {
                reference,
                capacity: 0
            })
        );
    }

    #[test]
    fn unreachable_object_is_swept() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        assert_eq!(collect(&mut heap, &[]).collected, 1);
        assert!(matches!(
            heap.get(reference),
            Err(HeapError::StaleReference { .. })
        ));
    }

    #[test]
    fn rooted_object_survives_collection() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        assert_eq!(collect(&mut heap, &[Value::Heap(reference)]).collected, 0);
        assert!(heap.get(reference).is_ok());
    }

    #[test]
    fn repeated_collection_resets_mark_state() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        assert_eq!(collect(&mut heap, &[Value::Heap(reference)]).after, 1);
        assert_eq!(collect(&mut heap, &[Value::Heap(reference)]).after, 1);
        assert_eq!(collect(&mut heap, &[]).after, 0);
    }

    #[test]
    fn swept_slot_is_reused_with_new_generation() {
        let mut heap = Heap::new();
        let old = array(&mut heap, Vec::new());
        collect(&mut heap, &[]);
        let new = array(&mut heap, vec![Value::Int(2)]);
        assert_eq!(old.slot(), new.slot());
        assert_ne!(old.generation(), new.generation());
        assert_eq!(heap.capacity(), 1);
    }

    #[test]
    fn stale_handle_cannot_alias_reused_slot() {
        let mut heap = Heap::new();
        let old = array(&mut heap, Vec::new());
        collect(&mut heap, &[]);
        let new = array(&mut heap, Vec::new());
        assert!(heap.get(new).is_ok());
        assert!(matches!(
            heap.get(old),
            Err(HeapError::StaleReference { occupied: true, .. })
        ));
    }

    #[test]
    fn heap_identity_includes_generation() {
        assert_ne!(HeapRef::new(1, 0), HeapRef::new(1, 1));
        assert_eq!(HeapRef::new(1, 1), HeapRef::new(1, 1));
    }

    #[test]
    fn nested_array_is_traced() {
        let mut heap = Heap::new();
        let child = array(&mut heap, vec![Value::Int(1)]);
        let parent = array(&mut heap, vec![Value::Heap(child)]);
        assert_eq!(collect(&mut heap, &[Value::Heap(parent)]).after, 2);
    }

    #[test]
    fn nested_map_is_traced() {
        let mut heap = Heap::new();
        let child = map(&mut heap, [("value", Value::Int(1))]);
        let parent = map(&mut heap, [("child", Value::Heap(child))]);
        assert_eq!(collect(&mut heap, &[Value::Heap(parent)]).after, 2);
    }

    #[test]
    fn unreachable_self_cycle_is_collected() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        let HeapObject::Array(values) = heap.get_mut(reference).unwrap() else {
            panic!("expected array")
        };
        values.push(Value::Heap(reference));
        assert_eq!(collect(&mut heap, &[]).collected, 1);
    }

    #[test]
    fn unreachable_two_array_cycle_is_collected() {
        let mut heap = Heap::new();
        let left = array(&mut heap, Vec::new());
        let right = array(&mut heap, vec![Value::Heap(left)]);
        let HeapObject::Array(values) = heap.get_mut(left).unwrap() else {
            panic!("expected array")
        };
        values.push(Value::Heap(right));
        assert_eq!(collect(&mut heap, &[]).collected, 2);
    }

    #[test]
    fn unreachable_array_map_cycle_is_collected() {
        let mut heap = Heap::new();
        let array_ref = array(&mut heap, Vec::new());
        let map_ref = map(&mut heap, [("array", Value::Heap(array_ref))]);
        let HeapObject::Array(values) = heap.get_mut(array_ref).unwrap() else {
            panic!("expected array")
        };
        values.push(Value::Heap(map_ref));
        assert_eq!(collect(&mut heap, &[]).collected, 2);
    }

    #[test]
    fn unreachable_two_map_cycle_is_collected() {
        let mut heap = Heap::new();
        let left = map(&mut heap, []);
        let right = map(&mut heap, [("left", Value::Heap(left))]);
        let HeapObject::Map(entries) = heap.get_mut(left).unwrap() else {
            panic!("expected map")
        };
        entries.insert("right".to_owned(), Value::Heap(right));
        assert_eq!(collect(&mut heap, &[]).collected, 2);
    }

    #[test]
    fn reachable_cycle_survives() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        let HeapObject::Array(values) = heap.get_mut(reference).unwrap() else {
            panic!("expected array")
        };
        values.push(Value::Heap(reference));
        assert_eq!(collect(&mut heap, &[Value::Heap(reference)]).after, 1);
    }

    #[test]
    fn removing_cycle_root_collects_entire_graph() {
        let mut heap = Heap::new();
        let left = array(&mut heap, Vec::new());
        let right = array(&mut heap, vec![Value::Heap(left)]);
        let HeapObject::Array(values) = heap.get_mut(left).unwrap() else {
            panic!("expected array")
        };
        values.push(Value::Heap(right));
        assert_eq!(collect(&mut heap, &[Value::Heap(left)]).after, 2);
        assert_eq!(collect(&mut heap, &[]).collected, 2);
    }

    #[test]
    fn map_formatting_sorts_keys() {
        let mut heap = Heap::new();
        let reference = map(
            &mut heap,
            [
                ("name", Value::String("Han".to_owned())),
                ("age", Value::Int(20)),
            ],
        );
        assert_eq!(
            heap.format_value(&Value::Heap(reference)).unwrap(),
            "{ age: 20, name: \"Han\" }"
        );
    }

    #[test]
    fn nested_formatting_uses_heap_context() {
        let mut heap = Heap::new();
        let child = array(&mut heap, vec![Value::Int(1)]);
        let parent = array(&mut heap, vec![Value::Heap(child)]);
        assert_eq!(heap.format_value(&Value::Heap(parent)).unwrap(), "[[1]]");
    }

    #[test]
    fn cycle_formatting_is_safe() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        let HeapObject::Array(values) = heap.get_mut(reference).unwrap() else {
            panic!("expected array")
        };
        values.push(Value::Heap(reference));
        assert_eq!(
            heap.format_value(&Value::Heap(reference)).unwrap(),
            "[<cycle>]"
        );
    }

    #[test]
    fn formatting_stale_reference_is_error() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        collect(&mut heap, &[]);
        assert!(matches!(
            heap.format_value(&Value::Heap(reference)),
            Err(HeapError::StaleReference { .. })
        ));
    }

    #[test]
    fn function_constant_traces_heap_reference() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        let mut chunk = Chunk::new();
        chunk.add_constant(Value::Heap(reference)).unwrap();
        let function = Value::Function(Rc::new(Function::new("root", Arity::new(0), chunk)));
        assert_eq!(collect(&mut heap, &[function]).after, 1);
    }

    #[test]
    fn closure_closed_upvalue_traces_heap_reference() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        let upvalue = Rc::new(RefCell::new(UpvalueState::Closed(Value::Heap(reference))));
        let function = Rc::new(Function::new("root", Arity::new(0), Chunk::new()));
        let closure = Value::Closure(Rc::new(Closure::with_upvalues(function, vec![upvalue])));
        assert_eq!(collect(&mut heap, &[closure]).after, 1);
    }

    #[test]
    fn open_upvalue_traces_stack_reference() {
        let mut heap = Heap::new();
        let reference = array(&mut heap, Vec::new());
        let upvalue = Rc::new(RefCell::new(UpvalueState::Open(0)));
        let stats = heap
            .collect(&[], &[Value::Heap(reference)], &[upvalue])
            .unwrap();
        assert_eq!(stats.after, 1);
    }

    #[test]
    fn malformed_open_upvalue_is_structured_error() {
        let mut heap = Heap::new();
        let upvalue = Rc::new(RefCell::new(UpvalueState::Open(3)));
        assert_eq!(
            heap.collect(&[], &[], &[upvalue]),
            Err(HeapError::InvalidOpenUpvalue {
                stack_index: 3,
                stack_len: 0
            })
        );
    }

    #[test]
    fn deep_heap_graph_uses_iterative_mark_worklist() {
        let mut heap = Heap::new();
        let mut root = array(&mut heap, Vec::new());
        for _ in 0..10_000 {
            root = array(&mut heap, vec![Value::Heap(root)]);
        }
        assert_eq!(collect(&mut heap, &[Value::Heap(root)]).after, 10_001);
    }

    #[test]
    fn collection_statistics_report_before_after_and_collected() {
        let mut heap = Heap::new();
        let kept = array(&mut heap, Vec::new());
        array(&mut heap, Vec::new());
        assert_eq!(
            collect(&mut heap, &[Value::Heap(kept)]),
            GcStats {
                before: 2,
                after: 1,
                collected: 1
            }
        );
    }
}

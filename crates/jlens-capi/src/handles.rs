//! Tables of handles: the objects the host knows only by a `u64`.

use std::sync::{Arc, Mutex, PoisonError};

/// A table of objects known by handle. Its lock is held only to find, add or remove an entry:
/// the work on an object happens on the `Arc` it hands out, so that threads working on
/// different objects never wait for each other.
pub struct Table<T> {
    slots: Mutex<Slots<T>>,
}

struct Slots<T> {
    entries: Vec<Entry<T>>,
    /// The indices of the empty entries, reused last in, first out.
    free: Vec<u32>,
}

struct Entry<T> {
    /// Bumped each time the entry is emptied, so that the handles of earlier objects in this
    /// entry stop matching.
    generation: u32,
    value: Option<Arc<T>>,
}

impl<T> Table<T> {
    /// An empty table.
    pub const fn new() -> Self {
        Table {
            slots: Mutex::new(Slots {
                entries: Vec::new(),
                free: Vec::new(),
            }),
        }
    }

    /// Adds `value` and returns its handle, or `None` when the table is full (`u32::MAX - 1`
    /// live objects).
    pub fn insert(&self, value: T) -> Option<u64> {
        self.insert_arc(Arc::new(value))
    }

    /// Adds the object behind `value`, as [`Table::insert`] does, for a caller that keeps a
    /// reference to it too.
    pub fn insert_arc(&self, value: Arc<T>) -> Option<u64> {
        let value = Some(value);
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let index = match slots.free.pop() {
            Some(index) => index,
            None => {
                let index = u32::try_from(slots.entries.len())
                    .ok()
                    .filter(|&index| index < u32::MAX)?;
                slots.entries.push(Entry {
                    generation: 0,
                    value: None,
                });
                index
            }
        };
        let entry = &mut slots.entries[index as usize];
        entry.value = value;
        Some(handle(entry.generation, index))
    }

    /// The object of `handle`, if it is live.
    pub fn get(&self, handle: u64) -> Option<Arc<T>> {
        let (generation, index) = parts(handle)?;
        let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = slots.entries.get(index as usize)?;
        match entry.generation == generation {
            true => entry.value.clone(),
            false => None,
        }
    }

    /// Removes the object of `handle` and returns it, if it was live. Its handle is unknown
    /// from then on, even after the entry holds another object.
    pub fn remove(&self, handle: u64) -> Option<Arc<T>> {
        let (generation, index) = parts(handle)?;
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = slots.entries.get_mut(index as usize)?;
        if entry.generation != generation {
            return None;
        }
        let value = entry.value.take()?;
        entry.generation = entry.generation.wrapping_add(1);
        slots.free.push(index);
        Some(value)
    }
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// The handle of entry `index` in its `generation`: the generation in the high half, the index
/// plus one in the low half, so that no handle is 0.
fn handle(generation: u32, index: u32) -> u64 {
    (u64::from(generation) << 32) | u64::from(index + 1)
}

fn parts(handle: u64) -> Option<(u32, u32)> {
    let index = (handle as u32).checked_sub(1)?;
    Some(((handle >> 32) as u32, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_freed_handle_stays_unknown_after_its_entry_is_reused() {
        let table = Table::new();
        let first = table.insert("first").unwrap();
        assert_eq!(table.remove(first).as_deref(), Some(&"first"));
        let second = table.insert("second").unwrap();
        assert_ne!(first, second);
        assert_eq!(first as u32, second as u32, "the entry is reused");
        assert!(table.get(first).is_none());
        assert!(table.remove(first).is_none());
        assert_eq!(table.get(second).as_deref(), Some(&"second"));
    }

    #[test]
    fn no_handle_is_zero_and_zero_is_unknown() {
        let table = Table::new();
        assert_ne!(table.insert(()).unwrap(), 0);
        assert!(table.get(0).is_none());
        assert!(table.remove(0).is_none());
        assert!(
            table.get(1 << 32).is_none(),
            "index 0 means none in any generation"
        );
    }
}

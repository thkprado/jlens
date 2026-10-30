//! Tables of handles: the objects the host knows only by a `u64`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread::LocalKey;

/// A table of objects known by handle. Its lock is held only to add or remove an entry, or to
/// find one a thread has not cached: the work on an object happens on the `Arc` it hands out, so
/// that threads working on different objects never wait for each other.
///
/// A thread finds the objects it uses again without the lock ([`Table::cached`]): each entry's
/// live handle is also kept where any thread reads it with one atomic load, and the thread keeps
/// the object in a cache of its own, so that threads evaluating the same expression against the
/// same document write nothing they share (roadmap 0.26 #10).
pub struct Table<T> {
    slots: Mutex<Slots<T>>,
    /// The handle of the object in each entry, 0 when it is empty, in chunks that double in
    /// size, made as the table grows and never moved, so that they are read without the lock.
    live: [OnceLock<Box<[AtomicU64]>>; CHUNKS],
    /// The number of objects removed so far, which tells a thread's cache when to drop the
    /// objects it holds that were removed.
    removed: AtomicU64,
}

/// The entries of the first chunk of `live`; chunk `k` holds `FIRST << k`.
const FIRST: usize = 32;
/// Enough chunks for `u32::MAX` entries.
const CHUNKS: usize = 28;

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

/// The objects of a table that a thread found, which it finds again without the table's lock:
/// a `thread_local!` for each table that is used that way ([`Table::cached`]).
pub struct Cache<T> {
    /// `Table::removed` as this cache last saw it.
    removed: u64,
    entries: Vec<(u64, Found<T>)>,
}

/// An object found by [`Table::cached`], shared with its thread's cache: cloning it writes
/// nothing other threads read, where cloning the `Arc` would write its count, which every
/// thread evaluating the same expression shares.
#[allow(clippy::redundant_allocation)]
pub struct Found<T>(Rc<Arc<T>>);

impl<T> Found<T> {
    /// The `Arc` the table holds the object by.
    pub fn arc(&self) -> &Arc<T> {
        &self.0
    }
}

impl<T> Clone for Found<T> {
    fn clone(&self) -> Self {
        Found(self.0.clone())
    }
}

impl<T> std::ops::Deref for Found<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// The objects a [`Cache`] holds: the ones an evaluation finds are few (an expression, a
/// document, options, bindings, a scratch buffer).
const CACHED: usize = 8;

impl<T> Cache<T> {
    /// An empty cache.
    pub const fn new() -> Self {
        Cache {
            removed: 0,
            entries: Vec::new(),
        }
    }
}

impl<T> Default for Cache<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Table<T> {
    /// An empty table.
    pub const fn new() -> Self {
        Table {
            slots: Mutex::new(Slots {
                entries: Vec::new(),
                free: Vec::new(),
            }),
            live: [const { OnceLock::new() }; CHUNKS],
            removed: AtomicU64::new(0),
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
        let handle = handle(entry.generation, index);
        // Every index below `u32::MAX` has its chunk.
        if let Some(live) = self.live_entry(index, true) {
            live.store(handle, Ordering::Release);
        }
        Some(handle)
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

    /// The object of `handle`, if it is live, as [`Table::get`] finds it, but kept in this
    /// thread's `cache` and found there again without the table's lock. A removed object stays
    /// in the cache until the thread's next look-up in this table, and an evaluation that holds
    /// it keeps it until it returns.
    pub fn cached(
        &self,
        cache: &'static LocalKey<RefCell<Cache<T>>>,
        handle: u64,
    ) -> Option<Found<T>> {
        let (_, index) = parts(handle)?;
        let live = self.live_entry(index, false)?.load(Ordering::Acquire) == handle;
        cache.with_borrow_mut(|cache| {
            self.prune_in(cache);
            if !live {
                return None;
            }
            if let Some((_, value)) = cache.entries.iter().find(|&&(kept, _)| kept == handle) {
                return Some(value.clone());
            }
            let value = Found(Rc::new(self.get(handle)?));
            if cache.entries.len() == CACHED {
                cache.entries.remove(0);
            }
            cache.entries.push((handle, value.clone()));
            Some(value)
        })
    }

    /// Drops the objects of this thread's `cache` that were removed from the table since it last
    /// looked: one atomic load when none was.
    pub fn prune(&self, cache: &'static LocalKey<RefCell<Cache<T>>>) {
        cache.with_borrow_mut(|cache| self.prune_in(cache));
    }

    fn prune_in(&self, cache: &mut Cache<T>) {
        let removed = self.removed.load(Ordering::Acquire);
        if cache.removed != removed {
            cache.removed = removed;
            cache.entries.retain(|&(kept, _)| self.is_live(kept));
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
        if let Some(live) = self.live_entry(index, false) {
            live.store(0, Ordering::Release);
        }
        self.removed.fetch_add(1, Ordering::Release);
        Some(value)
    }

    /// Whether `handle` names the object in its entry.
    fn is_live(&self, handle: u64) -> bool {
        parts(handle)
            .and_then(|(_, index)| self.live_entry(index, false))
            .is_some_and(|live| live.load(Ordering::Acquire) == handle)
    }

    /// Where entry `index`'s live handle is kept, its chunk made when `make` is set.
    fn live_entry(&self, index: u32, make: bool) -> Option<&AtomicU64> {
        let (chunk, offset) = chunk(index);
        let entries = self.live.get(chunk)?;
        let entries = match make {
            true => {
                entries.get_or_init(|| (0..FIRST << chunk).map(|_| AtomicU64::new(0)).collect())
            }
            false => entries.get()?,
        };
        entries.get(offset)
    }
}

impl<T> Default for Table<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// The chunk of `live` that holds entry `index`, and its offset there: chunk `k` holds the
/// entries from `FIRST * (2^k - 1)` on.
fn chunk(index: u32) -> (usize, usize) {
    let index = index as usize;
    let chunk = (index / FIRST + 1).ilog2() as usize;
    (chunk, index - FIRST * ((1 << chunk) - 1))
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

    #[test]
    fn the_chunks_hold_every_index_once() {
        let mut next = 0;
        for index in (0..10_000).chain([u32::MAX - 2, u32::MAX - 1]) {
            let (chunk, offset) = chunk(index);
            assert!(chunk < CHUNKS && offset < FIRST << chunk, "{index}");
            if index < 10_000 {
                assert_eq!(FIRST * ((1 << chunk) - 1) + offset, next);
                next += 1;
            }
        }
    }

    static TABLE: Table<&str> = Table::new();
    thread_local! {
        static CACHE: RefCell<Cache<&'static str>> = const { RefCell::new(Cache::new()) };
    }

    #[test]
    fn a_cached_object_is_unknown_once_removed_on_any_thread() {
        let handles: Vec<u64> = (0..100).map(|_| TABLE.insert("x").unwrap()).collect();
        for &handle in &handles {
            assert_eq!(TABLE.cached(&CACHE, handle).as_deref().copied(), Some("x"));
            assert_eq!(TABLE.cached(&CACHE, handle).as_deref().copied(), Some("x"));
        }
        assert!(CACHE.with_borrow(|cache| cache.entries.len()) <= CACHED);
        let kept = *handles.last().unwrap();
        let held = TABLE.cached(&CACHE, kept).unwrap();
        std::thread::spawn(move || TABLE.remove(kept).unwrap())
            .join()
            .unwrap();
        assert!(TABLE.cached(&CACHE, kept).is_none());
        assert_eq!(*held, "x", "an object in use stays until dropped");
        assert!(CACHE.with_borrow(|cache| cache.entries.iter().all(|&(h, _)| h != kept)));
        let reused = TABLE.insert("y").unwrap();
        assert_eq!(reused as u32, kept as u32, "the entry is reused");
        assert!(TABLE.cached(&CACHE, kept).is_none());
        assert_eq!(TABLE.cached(&CACHE, reused).as_deref().copied(), Some("y"));
        assert!(TABLE.cached(&CACHE, 0).is_none());
        assert!(
            TABLE.cached(&CACHE, u64::from(u32::MAX)).is_none(),
            "an entry never made"
        );
        for handle in handles.into_iter().filter(|&h| h != kept).chain([reused]) {
            TABLE.remove(handle).unwrap();
        }
    }
}

//! Buffers owned by Rust. The host gets a handle and the buffer's address, writes into it or
//! reads from it, and Rust finds the buffer again by its handle.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, PoisonError};

use crate::handles::{Cache, Table};

/// The live buffers, each with a lock of its own: the table's lock is held only to find one.
static BUFFERS: Table<Mutex<Vec<u8>>> = Table::new();

thread_local! {
    /// The buffers this thread found with [`with_cached`].
    static FOUND: RefCell<Cache<Mutex<Vec<u8>>>> = const { RefCell::new(Cache::new()) };
}

/// Creates a buffer of `len` zero bytes and returns its handle, or 0 when it cannot be
/// allocated or the table is full.
pub fn create(len: usize) -> u64 {
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(len).is_err() {
        return 0;
    }
    bytes.resize(len, 0);
    insert(bytes)
}

/// Stores `bytes` as a new buffer and returns its handle, or 0 when the table is full.
pub fn insert(bytes: Vec<u8>) -> u64 {
    BUFFERS.insert(Mutex::new(bytes)).unwrap_or(0)
}

/// The address of a buffer's bytes, for the host to write and read through (0 for an unknown
/// handle). A buffer is never resized, so the address stays valid until the buffer is freed.
pub fn address(handle: u64) -> usize {
    with(handle, |bytes| bytes.as_mut_ptr().expose_provenance()).unwrap_or(0)
}

/// The length of a buffer (0 for an unknown handle).
pub fn len(handle: u64) -> usize {
    with(handle, |bytes| bytes.len()).unwrap_or(0)
}

/// Frees a buffer: 1 when it was live, 0 for an unknown or freed handle.
pub fn free(handle: u64) -> u32 {
    u32::from(take(handle).is_some())
}

/// Removes a buffer and returns its bytes.
pub fn take(handle: u64) -> Option<Vec<u8>> {
    let buffer = BUFFERS.remove(handle)?;
    let mut bytes = buffer.lock().unwrap_or_else(PoisonError::into_inner);
    Some(std::mem::take(&mut *bytes))
}

/// Runs `f` on a buffer's bytes, holding that buffer's lock only.
pub fn with<R>(handle: u64, f: impl FnOnce(&mut Vec<u8>) -> R) -> Option<R> {
    let buffer = BUFFERS.get(handle)?;
    let mut bytes = buffer.lock().unwrap_or_else(PoisonError::into_inner);
    Some(f(&mut bytes))
}

/// Runs `f` on a buffer's bytes as [`with`] does, for a buffer this thread uses again and
/// again, such as the scratch buffer an evaluation replies into: found without the table's lock
/// after the first time ([`Table::cached`]).
pub fn with_cached<R>(handle: u64, f: impl FnOnce(&mut Vec<u8>) -> R) -> Option<R> {
    let buffer = BUFFERS.cached(&FOUND, handle)?;
    let mut bytes = buffer.lock().unwrap_or_else(PoisonError::into_inner);
    Some(f(&mut bytes))
}

/// A buffer, for a caller that uses it many times: finding it again by its handle takes the
/// table's lock. Once freed, it holds no bytes.
pub fn get(handle: u64) -> Option<Buffer> {
    BUFFERS.get(handle).map(Buffer)
}

/// A buffer found by [`get`].
pub struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    /// Runs `f` on the buffer's bytes, holding its lock.
    pub fn with<R>(&self, f: impl FnOnce(&mut Vec<u8>) -> R) -> R {
        f(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

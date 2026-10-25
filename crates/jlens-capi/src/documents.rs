//! Documents, as ADR 0014 keeps them: the JSON text behind an `Arc`, shared by every thread,
//! and converted once per thread into a thread-local cache, since a `jlens_core::Document` is
//! not `Send`.
//!
//! A thread keeps its conversion while the document lives. Once it is freed, each thread drops
//! its conversion the next time it calls any export ([`guarded`](crate::guarded)): a thread
//! that never calls one again keeps it until it exits.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use jlens_core::Document;
use jlens_core::reads::Read;

use crate::handles::Table;

/// What a document is made of, which each thread converts.
pub(crate) struct Text {
    text: String,
    /// The fields the conversion keeps; `None` keeps them all.
    read: Option<Read>,
}

impl Text {
    pub(crate) fn new(text: String, read: Option<Read>) -> Self {
        Text { text, read }
    }

    fn convert(&self) -> serde_json::Result<Document> {
        match &self.read {
            None => Document::parse(&self.text),
            Some(read) => Document::parse_cut(&self.text, read),
        }
    }
}

impl Drop for Text {
    /// Runs once the document is freed and no evaluation holds it any longer, so that a thread
    /// that sees the count change finds its weak reference dead.
    fn drop(&mut self) {
        DROPPED.fetch_add(1, Ordering::Release);
    }
}

/// The live documents.
pub(crate) static DOCUMENTS: Table<Text> = Table::new();

/// The number of documents dropped so far, which tells a thread whether its conversions need
/// sweeping: one atomic load per call when none was.
static DROPPED: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// The documents this thread has converted, each with a weak reference to its text.
    static CONVERTED: RefCell<Vec<(Weak<Text>, Rc<Document>)>> = const { RefCell::new(Vec::new()) };
    /// `DROPPED` as this thread last swept.
    static SWEPT: Cell<u64> = const { Cell::new(0) };
}

/// This thread's conversion of `text`, made on its first use.
pub(crate) fn converted(text: &Arc<Text>) -> serde_json::Result<Rc<Document>> {
    CONVERTED.with_borrow_mut(|cache| {
        // A weak reference keeps its allocation, so a dead one never matches a live document.
        if let Some((_, document)) = cache
            .iter()
            .find(|(weak, _)| std::ptr::eq(weak.as_ptr(), Arc::as_ptr(text)))
        {
            return Ok(document.clone());
        }
        let document = Rc::new(text.convert()?);
        cache.push((Arc::downgrade(text), document.clone()));
        Ok(document)
    })
}

/// Drops this thread's conversions of the documents that were freed, if any was since its last
/// sweep. Every export calls it first ([`crate::guarded`]).
pub(crate) fn sweep_if_freed() {
    let dropped = DROPPED.load(Ordering::Acquire);
    if SWEPT.get() != dropped {
        SWEPT.set(dropped);
        sweep();
    }
}

/// Drops this thread's conversions of the documents that were freed.
fn sweep() {
    CONVERTED.with_borrow_mut(|cache| cache.retain(|(weak, _)| weak.strong_count() > 0));
}

/// The number of conversions this thread holds, for the tests.
pub fn cached() -> usize {
    CONVERTED.with_borrow(Vec::len)
}

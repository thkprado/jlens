//! Documents, as ADR 0014 keeps them: the JSON text behind an `Arc`, shared by every thread,
//! and converted once per thread into a thread-local cache, since a `jlens_core::Document` is
//! not `Send`.
//!
//! A thread keeps its conversion while the document lives. Once it is freed, each thread drops
//! its conversion the next time it calls any export ([`guarded`](crate::guarded)): a thread
//! that never calls one again keeps it until it exits.
//!
//! An evaluation takes documents as variables too ([`crate::api::evaluate_document_with`]), in
//! a buffer that lists them, its numbers little-endian:
//!
//! ```text
//! count: u32, then for each document: handle: u64, length: u32, its name: `length` bytes
//! ```
//!
//! A name is UTF-8, without the leading `$`. The bytes after the last document are not read, so
//! that the host may write the list at the start of the scratch buffer it gives the same call.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use jlens_core::Document;
use jlens_core::reads::Read;

use crate::handles::{Cache, Found, Table};

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
    /// The documents this thread's evaluations found, found again without the table's lock.
    static FOUND: RefCell<Cache<Text>> = const { RefCell::new(Cache::new()) };
    /// The documents this thread has converted, each with a weak reference to its text.
    static CONVERTED: RefCell<Vec<(Weak<Text>, Rc<Document>)>> = const { RefCell::new(Vec::new()) };
    /// `DROPPED` as this thread last swept.
    static SWEPT: Cell<u64> = const { Cell::new(0) };
}

/// The document of `handle`, if it is live, for an evaluation ([`Table::cached`]).
pub(crate) fn find(handle: u64) -> Option<Found<Text>> {
    DOCUMENTS.cached(&FOUND, handle)
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

/// The documents an evaluation binds to variables: their names, one after another, and this
/// thread's conversion of each, with where its name ends.
#[derive(Default)]
pub(crate) struct Named {
    names: String,
    documents: Vec<(usize, Rc<Document>)>,
}

impl Named {
    /// The variables, as `Expression::evaluate_document_with` takes them.
    pub(crate) fn variables(&self) -> Vec<(&str, &Document)> {
        let mut start = 0;
        self.documents
            .iter()
            .map(|(end, document)| {
                let name = &self.names[start..*end];
                start = *end;
                (name, &**document)
            })
            .collect()
    }
}

/// Why a list of documents names none.
pub(crate) enum NotNamed {
    /// The list is not in its form.
    Form(String),
    /// It lists a handle that names no live document.
    Unknown(u64),
    /// A document's text does not convert: it did when the document was made.
    NotJson(String),
}

/// The documents an evaluation binds to variables, as `bytes` lists them (see the
/// [module](self) documentation), each converted for this thread. The list's form is checked
/// whole before its handles, and what it holds is copied: in two allocations, however many
/// documents it lists, as an evaluation makes it at each call.
pub(crate) fn named(bytes: &[u8]) -> Result<Named, NotNamed> {
    const SHORT: &str = "the list ends before its last document";
    /// The next document of the list: its handle and its name.
    fn next<'b>(rest: &mut &'b [u8]) -> Option<(u64, &'b [u8])> {
        let (handle, tail) = rest.split_first_chunk::<8>()?;
        let (length, tail) = tail.split_first_chunk::<4>()?;
        let (name, tail) = tail.split_at_checked(u32::from_le_bytes(*length) as usize)?;
        *rest = tail;
        Some((u64::from_le_bytes(*handle), name))
    }
    let (count, listed) = bytes
        .split_first_chunk::<4>()
        .ok_or_else(|| NotNamed::Form(SHORT.to_owned()))?;
    let count = u32::from_le_bytes(*count);
    let mut rest = listed;
    let mut length = 0;
    for _ in 0..count {
        let (_, name) = next(&mut rest).ok_or_else(|| NotNamed::Form(SHORT.to_owned()))?;
        std::str::from_utf8(name)
            .map_err(|e| NotNamed::Form(format!("a name is not UTF-8: {e}")))?;
        length += name.len();
    }
    // The count is that of the documents the buffer holds, now.
    let mut named = Named {
        names: String::with_capacity(length),
        documents: Vec::with_capacity(count as usize),
    };
    let mut rest = listed;
    for _ in 0..count {
        let Some((handle, name)) = next(&mut rest) else {
            break;
        };
        let text = find(handle).ok_or(NotNamed::Unknown(handle))?;
        let document = converted(text.arc()).map_err(|e| NotNamed::NotJson(e.to_string()))?;
        // Checked above.
        named
            .names
            .push_str(std::str::from_utf8(name).unwrap_or_default());
        named.documents.push((named.names.len(), document));
    }
    Ok(named)
}

/// Drops this thread's conversions of the documents that were freed, if any was since its last
/// sweep. Every export calls it first ([`crate::guarded`]). First this thread's cache lets go of
/// the documents freed, so that a document is dropped once no thread holds it, which the
/// other threads see at their next call.
pub(crate) fn sweep_if_freed() {
    DOCUMENTS.prune(&FOUND);
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

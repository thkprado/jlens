//! Documents, as ADR 0014 keeps them: the JSON text behind an `Arc`, shared by every thread,
//! and converted once per thread into a thread-local cache, since a `jlens_core::Document` is
//! not `Send`.
//!
//! A thread keeps its conversion, by the document's handle, while the document lives. Once it
//! is freed, each thread drops its conversion the next time it calls any export
//! ([`guarded`](crate::guarded)): a thread that never calls one again keeps it until it exits,
//! but keeps no other thread from dropping theirs, nor the text from being freed (0.37 #4).
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

/// The live documents.
pub(crate) static DOCUMENTS: Table<Text> = Table::new();

thread_local! {
    /// The documents this thread has converted, by handle. A thread holds no document's text
    /// between its calls: a freed document's text is dropped once no conversion of it runs,
    /// whichever thread found it last.
    static CONVERTED: RefCell<Vec<(u64, Rc<Document>)>> = const { RefCell::new(Vec::new()) };
    /// [`Table::removed`] of [`DOCUMENTS`] as this thread last swept.
    static SWEPT: Cell<u64> = const { Cell::new(0) };
}

/// Converts `text`, to fail on text that is not JSON, adds it to the documents and keeps the
/// conversion for this thread; `None` when the table is full.
pub(crate) fn insert(text: Text) -> serde_json::Result<Option<u64>> {
    let document = Rc::new(text.convert()?);
    let Some(handle) = DOCUMENTS.insert(text) else {
        return Ok(None);
    };
    CONVERTED.with_borrow_mut(|cache| cache.push((handle, document)));
    Ok(Some(handle))
}

/// This thread's conversion of the document of `handle`, made on its first use: one atomic
/// load and no lock once made.
pub(crate) fn converted(handle: u64) -> Result<Rc<Document>, NotNamed> {
    if !DOCUMENTS.is_live(handle) {
        return Err(NotNamed::Unknown(handle));
    }
    let kept = CONVERTED.with_borrow(|cache| {
        cache
            .iter()
            .find(|&&(kept, _)| kept == handle)
            .map(|(_, document)| document.clone())
    });
    if let Some(document) = kept {
        return Ok(document);
    }
    // The text is held only while it converts.
    let text = DOCUMENTS.get(handle).ok_or(NotNamed::Unknown(handle))?;
    let document = Rc::new(
        text.convert()
            .map_err(|e| NotNamed::NotJson(e.to_string()))?,
    );
    drop(text);
    CONVERTED.with_borrow_mut(|cache| cache.push((handle, document.clone())));
    Ok(document)
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
        let document = converted(handle)?;
        // Checked above.
        named
            .names
            .push_str(std::str::from_utf8(name).unwrap_or_default());
        named.documents.push((named.names.len(), document));
    }
    Ok(named)
}

/// Drops this thread's conversions of the documents that were freed, if any was since its last
/// sweep: one atomic load when none was. Every export calls it first ([`crate::guarded`]).
pub(crate) fn sweep_if_freed() {
    // Read before the handles are, so that a document freed during the sweep is swept next time.
    let removed = DOCUMENTS.removed();
    if SWEPT.get() != removed {
        SWEPT.set(removed);
        CONVERTED.with_borrow_mut(|cache| cache.retain(|&(handle, _)| DOCUMENTS.is_live(handle)));
    }
}

/// The number of conversions this thread holds, for the tests.
pub fn cached() -> usize {
    CONVERTED.with_borrow(Vec::len)
}

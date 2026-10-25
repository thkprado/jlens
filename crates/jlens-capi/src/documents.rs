//! Documents, as ADR 0014 keeps them: the JSON text behind an `Arc`, shared by every thread,
//! and converted once per thread into a thread-local cache, since a `jlens_core::Document` is
//! not `Send`.
//!
//! A thread keeps its conversion while the document lives. Once it is freed, each thread drops
//! its conversion the next time it makes, frees or evaluates against a document: a thread that
//! never does so again keeps it until it exits.

use std::cell::RefCell;
use std::rc::Rc;
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

/// The live documents.
pub(crate) static DOCUMENTS: Table<Text> = Table::new();

thread_local! {
    /// The documents this thread has converted, each with a weak reference to its text.
    static CONVERTED: RefCell<Vec<(Weak<Text>, Rc<Document>)>> = const { RefCell::new(Vec::new()) };
}

/// This thread's conversion of `text`, made on its first use. The conversions of documents that
/// were freed are dropped first.
pub(crate) fn converted(text: &Arc<Text>) -> serde_json::Result<Rc<Document>> {
    CONVERTED.with_borrow_mut(|cache| {
        cache.retain(|(weak, _)| weak.strong_count() > 0);
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

/// Drops this thread's conversions of the documents that were freed.
pub(crate) fn sweep() {
    CONVERTED.with_borrow_mut(|cache| cache.retain(|(weak, _)| weak.strong_count() > 0));
}

/// The number of conversions this thread holds, for the tests.
pub fn cached() -> usize {
    CONVERTED.with_borrow(Vec::len)
}

//! # jlens-schema
//!
//! Structural metadata for JSON documents, and the JSONata language service built on it.
//!
//! * **Inference.** [`Schema::infer`] derives a [`Schema`] from sample documents. It records
//!   types, required and optional fields, array items, number ranges, string formats, small
//!   enumerations and examples.
//! * **Configuration.** [`Schema::apply_overrides`] adds descriptions, labels, allowed values and
//!   hidden fields. Schemas convert to and from JSON Schema 2020-12
//!   ([`Schema::to_json_schema`], [`Schema::from_json_schema`]), so they can be versioned and
//!   edited by hand.
//! * **Language service.** [`LanguageService`] offers completion, diagnostics (including
//!   references to fields that the schema does not have), hover and signature help for JSONata
//!   expressions. It types each part of an expression against the schema, following the
//!   evaluator's rules for paths, filters and variables.
//!
//! ```
//! use jlens_schema::{LanguageService, Schema};
//! use serde_json::json;
//!
//! let schema = Schema::infer(&[json!({
//!     "customer": {"login": "123", "name": "XYZ"},
//!     "items": [{"code": "1", "type": "apparel", "unitPrice": 10.0, "quantity": 2}]
//! })]);
//! let service = LanguageService::new(Some(schema));
//!
//! // Fields of the items, inside a filter:
//! let text = "items[ty";
//! let labels: Vec<_> = service.complete(text, text.len()).items.into_iter()
//!     .map(|i| i.label).collect();
//! assert_eq!(labels, ["type", "$type"]); // fields first, then matching functions
//!
//! // A misspelt field is reported:
//! let warnings = service.diagnostics("customer.Login");
//! assert_eq!(warnings[0].code, "W0201");
//! assert!(warnings[0].message.contains("did you mean `login`"));
//! ```

mod jsonschema;
mod service;
mod shape;
mod typing;

pub use service::{
    CompletionItem, CompletionKind, Completions, Hover, LanguageService, SignatureHelp,
};
pub use shape::{Field, JsonType, Override, Schema, Shape};

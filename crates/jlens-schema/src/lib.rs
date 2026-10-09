//! # jlens-schema
//!
//! Structural metadata for JSON documents and the JSONata language service built on it.
//!
//! * **Inference** — derive a [`Schema`] from one or more sample documents, then refine it with
//!   an overrides file or import/export it as JSON Schema 2020-12.
//! * **Language service** — completion, diagnostics, hover and signature help for JSONata
//!   expressions, driven by the schema.
//!
//! This crate is scaffolded; inference and the language service arrive in phase 5 of the
//! roadmap (`docs/src/architecture/roadmap.md`).

use serde::{Deserialize, Serialize};

/// Structural metadata describing the documents an expression is written against.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Schema {}

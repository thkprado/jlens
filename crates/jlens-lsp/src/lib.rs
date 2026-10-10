//! # jlens-lsp
//!
//! A language server ([LSP](https://microsoft.github.io/language-server-protocol/)) for JSONata
//! expressions, built on the [`jlens_schema::LanguageService`]: completion of fields, variables
//! and functions, diagnostics (syntax errors, annotation problems, fields the schema does not
//! have), hover and signature help.
//!
//! Which schema, variables and host functions apply to which files is configured in a
//! `jlens.json` at the root of each workspace folder, mapping path patterns to settings:
//!
//! ```json
//! {
//!   "rules/**/*.jsonata": {
//!     "schema": "schemas/order.schema.json",
//!     "variables": {"minimum": {"type": "number"}},
//!     "functions": [{"name": "shipping", "signature": "<ns:n>", "summary": "Shipping cost"}]
//!   }
//! }
//! ```
//!
//! The `jlens lsp` command of `jlens-cli` runs [`run_stdio`]. Embedders can run [`run`] on
//! another [`lsp_server::Connection`], such as an in-memory one.

mod config;
mod server;
mod text;
mod uri;

pub use server::{Error, run};

/// Runs the server over standard input and output, until the client asks it to exit.
pub fn run_stdio() -> Result<(), Error> {
    let (connection, io_threads) = lsp_server::Connection::stdio();
    run(&connection)?;
    drop(connection);
    io_threads.join()?;
    Ok(())
}

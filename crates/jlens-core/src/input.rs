//! Inputs read one level at a time (ADR 0010).
//!
//! [`Expression::evaluate`](crate::Expression::evaluate) reads a `serde_json::Value` lazily: an
//! evaluation converts only the arrays and objects its expression reaches. Other
//! representations of a document implement [`Source`] and are evaluated with
//! [`Expression::evaluate_source`](crate::Expression::evaluate_source), so that they are read the
//! same way, without being converted to `serde_json` first. The WebAssembly package reads
//! JavaScript objects this way. A source that looks up one field for less than it reads its
//! object whole also implements [`Source::read_field`], which a path step uses; the Python
//! package reads its dictionaries this way. One that reads an item of an array for less than
//! the whole array implements [`Source::length`] and [`Source::read_item`], which an index such
//! as `items[0]` uses. A source can also write the arrays and objects of a
//! level as members of one [`Sources`], so that each costs one allocation instead of two; the
//! Python package does.
//!
//! ```
//! use jlens_core::input::{Sink, Source};
//! use jlens_core::{Bindings, Expression, Options};
//!
//! /// A table of rows, read as an array of objects.
//! struct Table<'t>(&'t [(&'t str, f64)]);
//! struct Row<'t>(&'t (&'t str, f64));
//! /// The input itself: the only value of its level.
//! struct Input<'t>(Table<'t>);
//!
//! impl<'t> Source<'t> for Input<'t> {
//!     fn read(&self, sink: &mut Sink<'_, 't>) -> Result<(), String> {
//!         sink.array(Table(self.0 .0));
//!         Ok(())
//!     }
//! }
//!
//! impl<'t> Source<'t> for Table<'t> {
//!     fn read(&self, sink: &mut Sink<'_, 't>) -> Result<(), String> {
//!         for row in self.0 {
//!             sink.object(Row(row));
//!         }
//!         Ok(())
//!     }
//! }
//!
//! impl<'t> Source<'t> for Row<'t> {
//!     fn read(&self, sink: &mut Sink<'_, 't>) -> Result<(), String> {
//!         sink.key("name");
//!         sink.string(self.0 .0);
//!         sink.key("price");
//!         sink.number(self.0 .1);
//!         Ok(())
//!     }
//! }
//!
//! let rows = [("pen", 1.5), ("ink", 4.0)];
//! let expr = Expression::compile("$sum(price)").unwrap();
//! let bindings = Bindings::new();
//! let result = expr.evaluate_source(&Input(Table(&rows)), &bindings, &Options::default());
//! assert_eq!(result.unwrap(), Some(serde_json::json!(5.5)));
//! ```

pub use crate::value::{Sink, Source, Sources};

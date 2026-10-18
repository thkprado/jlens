# 0019 — Documents serialized straight from Rust values

**Status:** Accepted.

## Context

An application that embeds `jlens-core` and keeps its state in Rust types (structs of strings,
numbers and small lists) has one way to evaluate against that state: build a
`serde_json::Value` (`serde_json::to_value`, or `json!`) and convert it with `Document::new`.
When the document is rebuilt often, from a state that changes between a few evaluations, that
double conversion costs more than the evaluations. One application measured it, rebuilding a
document of about 10 objects of 20 fields and evaluating it 1 to 10 times: building the
`serde_json::Value` took 35–40% of its time, `Document::new` 15–18%, the evaluations much less
than both.

`crates/jlens-core/benches/host_state.rs` reproduces the case: ten records of twenty fields,
rebuilt at each iteration, then the first 1, 4 or 10 of ten small expressions (filters, sorts,
`[0]`). Windows, release, jlens 0.20.0 with the prototype of this ADR:

| Building the document | µs |
|---|---|
| `serde_json::to_value` alone | 22.3 |
| `to_value` + `Document::new` (today) | 43.7 |
| `serde_json::to_string` + `Document::parse` | 35.8 |
| `Document::from_serialize` (this ADR) | 17.9 |

| Building and evaluating, µs | 1 | 4 | 10 |
|---|---|---|---|
| `to_value` + `Document::new` + `evaluate_document` (today) | 43.0 | 51.6 | 72.1 |
| `Document::from_serialize` + `evaluate_document` | 19.3 | 28.2 | 48.7 |
| `to_value` + lazy `evaluate` | 25.1 | 41.0 | 70.4 |
| `evaluate_source`, a `Source` written over the state | 3.8 | 20.5 | 56.0 |

Every path gives the same document and the same results (the benchmark checks it before it
measures).

The `serde_json::Value` costs a `String` for every key of every object, an `IndexMap` that hashes
each key, and a `String` for every string; `Document::new` then allocates each string again,
interns the keys and builds its own maps. Most of that is copying from one tree into another.

## Decision

1. `Document::from_serialize<T: ?Sized + Serialize>(value: &T) -> serde_json::Result<Document>`
   serializes a Rust value straight into the internal representation, through a
   `serde::Serializer` private to `jlens-core`.
2. **Its contract is `Document::new(&serde_json::to_value(value)?)`**: the same document, and
   an error wherever `to_value` fails. Property tests check it on arbitrary `serde_json::Value`s
   (which implement `Serialize`), and a unit test on the shapes serde derives: enums of each kind
   (`"Unit"`, `{"Variant": …}`), newtype and unit structs, `Option`, tuples, `char`, `f32`,
   maps keyed by numbers and booleans.
3. **Key order** is the order of serialization: a struct's fields in declaration order, a map in
   its iteration order (sorted for a `BTreeMap`, unspecified for a `HashMap`, as with
   `to_value`). A repeated key keeps its first position and takes the last value, as a
   `serde_json::Map` with `preserve_order`. The order no longer depends on that feature: the
   serializer writes the internal map itself. Keys are shared through the document's table of
   keys, as `Document::parse` shares them, so struct field names are not allocated per object.
4. **Numbers** become `f64`, as everywhere in jlens: `i64` and `u64` with `as` (beyond 2⁵³ they
   round, as `Document::new` rounds them today), `f32` widened exactly (`0.1f32` is
   `0.10000000149011612`, as in `to_value`). NaN and infinities become `null`, as `serde_json`
   writes them. `float_roundtrip` plays no part: it governs parsing text, and no text is parsed.
   The rare paths go through `serde_json` itself so that they follow its features: 128-bit
   integers (out of the `i64`/`u64` range: `serde_json`'s error, or a number with
   `arbitrary_precision`), keys that are not strings (numbers and booleans become their text,
   finite floats as `serde_json` formats them), and `serde_json`'s private tokens for
   `arbitrary_precision` numbers and `RawValue`.
5. **What JSON cannot represent fails**, with the `serde_json::Error` that `to_value` gives:
   a map key that is a unit, an option, a sequence, a map or bytes ("key must be a string"), a
   NaN or infinite float key, a 128-bit integer out of range, and any error a `Serialize`
   implementation raises (`S::Error::custom`). Nothing is built on an error. Bytes as a value
   become an array of numbers, as in `serde_json`.
6. **One difference: nesting.** A value nested more than 1,000 levels deep (`MAX_NESTING`) fails,
   where `to_value` would recurse until the stack overflows. The serializer grows the stack
   (`stack::grow`) on the way, as every recursive walk over values does.
7. The error type is `serde_json::Error`, the one `Document::parse` already returns. No new public
   type: the API gains one function.

## Alternatives not taken

- **An incremental builder** (`DocumentBuilder::object()`, `.field(key)`, `.array()`, …). It
  builds the same values with the same operations, so it would not be faster than a serializer
  that serde monomorphizes for each type, and it needs much more API to keep until 1.0: a builder
  type, its states, what a field without a value or an unclosed object means, its errors.
  The host also writes one call per field, where `#[derive(Serialize)]` writes none. A host that
  wants to write field by field already can, with `input::Source`.
- **`Document::from_source(&dyn Source)`**: reading a `Source` eagerly into a document. It reuses
  a public trait, but needs a hand-written source per type, and a `Source` writes its arrays and
  objects lazily (`Sink::array`), so the document would force every level. It would cost what
  `from_serialize` costs, for more code in the host.
- **`evaluate_source` alone, without a document.** Already public, and the fastest for one to
  four evaluations (3.8 and 20.5 µs), since it reads only the levels and fields an expression
  reaches. But each evaluation reads again what it needs, so it falls behind a document by ten
  evaluations (56.0 against 48.7 µs), and it needs a hand-written `Source` (about 120 lines in
  the benchmark). It stays the answer for an application that evaluates once per state; the
  guide should say so.
- **`serde_json::to_string` + `Document::parse`.** Needs nothing new and is 18% faster than
  today (35.8 µs against 43.7), but still twice as slow as `from_serialize`: it writes and
  parses text. It is the workaround for 0.20.

## Consequences

- The serializer is about 600 lines, most of them the methods `serde::Serializer` requires.
  serde 1 is stable, and `jlens-core` already exposes `serde_json` types, so a `Serialize` bound
  adds no dependency to the API that 1.0 will keep (ADR 0015 lists it among the stable items).
- The benchmark `host_state` joins the workload, with `host build` and `host rebuild` on the
  performance page.
- A later step could serialize only the fields some expressions read, as `Document::parse_cut`
  parses them (ADR 0016): a struct field the `Read` leaves out would not be serialized at all.
  Not part of this decision; measured if an application asks for it.

## Variables of `evaluate_document`

`evaluate_document` converts each variable of its `Bindings` whole, at every evaluation
(`Value::from_json`), where `evaluate_with` and `evaluate_traced` read them lazily
(`Value::lazy`, ADR 0010). It cannot do the same: a `Document` holds a `Value<'static>`, and
`Value<'a>` is invariant in `'a` (its objects are `RefCell`s, its sources `dyn Sources<'a>`), so
an evaluation against a document runs at `'static`, and a lazy value cannot borrow the
`Bindings`. A safe conversion of the document to a shorter lifetime would copy it whole.

The benchmark measures what that costs: a record of twenty fields read through a variable
(`$self.score > 50 and $self.active`) takes 3.5 µs, the same record read from the document
(`items[3]…`) 1.0 µs. The 2.5 µs are the conversion of the record, at each evaluation.

What would remove it is a variable whose value is already a document, bound without converting
(an `Rc` clone), with a way to take a part of a document as one (`$self` = the fourth record).
`Bindings` cannot hold a `Document` (it is `Send + Sync`, a document is not), so that is new API
of its own, decided separately: roadmap 0.21 #2.

# 0010 — Input documents converted lazily, one level at a time

**Status:** Accepted. Amends [ADR 0003](0003-value-model.md), whose consequences left room for a
borrowed view of the input.

## Context

[ADR 0003](0003-value-model.md) converts the input from `serde_json::Value` into jlens's own
values before an evaluation starts. The conversion visits the whole document, however little of
it the expression reads. On the 24 KB document of the benchmarks (performance page, 0.8.0), the
path `customer.name` takes:

| | `evaluate` | `evaluate_document` |
|---|---|---|
| Native | 115 µs: 91 µs to convert, 17 µs to drop, 0.9 µs to evaluate | 0.9 µs |
| WebAssembly | 308 µs: 41 µs for `JSON.stringify`, 256 µs to parse and free | 2.5 µs |

jsonata-js takes 2.1 µs, because it reads the JavaScript object it was given. Roadmap 0.8,
item 3, asks for `evaluate` within 10× of `evaluate_document` on that case: 9 µs natively and
25 µs in WebAssembly, with no other benchmark case more than 3% slower.

Two facts bound the design:

- **Natively, the input is borrowed.** `evaluate` takes `Option<&serde_json::Value>`. Owning it
  means cloning it, which costs about as much as converting it. A lazy value must be able to
  point into the caller's document.
- **In WebAssembly, `JSON.stringify` alone takes 41 µs**, more than the whole budget. The input
  must be read from the JavaScript object itself, not from its text.

The value model's rules stay: `Undefined` never crosses the API, evaluations never mutate their
input, a compiled `Expression` is `Send + Sync`, and a `Document` is converted once for many
evaluations.

## Decision

1. **Objects and arrays expand one level at a time.** `Value` keeps its variants. An
   `Array` or an object's map gains an optional *pending source*: the part of the input it
   stands for, not yet read. The first access expands it: its scalars are converted, and each
   of its arrays and objects becomes a new value with its own pending source. The level is then
   kept, so it is read at most once, and later accesses cost a branch.
   - The items of an array and the map of an object become private and are reached through
     accessors that expand first (`items()`, `map()`, `map_mut()`). The compiler then points at
     every place that read them directly, so none is missed.
   - A lookup expands the whole level of its object, not just one key. That costs the width of
     the object once, and keeps one value per field, so a field read twice is the same value.
   - Keys go through the shared table of `value::Keys`, kept for the whole evaluation.
   - A value whose source was never expanded, returned as a result, is converted to JSON
     straight from its source (`customer` returns a clone of that `serde_json` subtree) without
     building jlens values.
   - The transform operator modifies only the copy that `$clone` makes, and `$clone` serializes
     what it copies, which expands it. A `$clone` that does not copy (the documented exception) modifies the expanded
     level, never the caller's document.
2. **Values carry the lifetime of the input: `Value<'a>`.** A pending source is an enum:
   `Json(&'a serde_json::Value)`, which costs no allocation, or `Source(Box<dyn Source<'a> + 'a>)`
   for other inputs (item 4). The lifetime goes through the evaluator, environments and
   functions. Each evaluation picks one `'a`:
   - `evaluate` and the other borrowed entry points use the lifetime of their input.
   - `evaluate_document` evaluates with `'a = 'static`: a `Document` is fully expanded when it
     is built, as today, so it never holds a pending source.
   - `Value<'a>` is invariant (it holds `RefCell`s), so a value of one evaluation never mixes
     with another's. Nothing changes for threads: values were never `Send`, and an `Expression`
     holds no values.
   - The public `audit::ValueRef<'r>` keeps its single lifetime: it holds the value behind a
     private trait object, so tracers written today still compile.
3. **Bindings convert the same way.** A variable given as `serde_json::Value` becomes a lazy
   value over it. The results of host functions and the copies `$clone` makes are
   owned, not borrowed; they stay eager.
4. **Other inputs implement a public trait, `jlens_core::input::Source`.** One call expands one
   level into an opaque sink: null, booleans, numbers and strings are written as they are,
   arrays and objects as new sources, object fields after their key. The trait mentions no
   internal type, so the value model stays private. The WebAssembly package implements it over
   `JsValue`; an embedder can implement it over another JSON representation. The exact names
   are settled with the implementation, and `cargo-semver-checks` reviews them.
5. **The WebAssembly package reads the JavaScript object.**
   - One call to a JavaScript helper per level returns its keys, its scalars and its nested
     objects, so the cost does not grow with the number of crossings between Wasm and
     JavaScript.
   - The helper follows `JSON.stringify`: `toJSON` is called, as for a `Date`; boxed
     primitives are unwrapped; `undefined`, functions and symbols are left out of objects and
     become `null` in arrays; `NaN` and the infinities become `null`.
   - The cases where `JSON.stringify` throws (a cycle, a `BigInt`, a `toJSON` that throws)
     throw the same `TypeError`, when the part of the input that holds them is read.
   - `Document.fromJson` and documents built from JavaScript objects stay eager, for repeated
     evaluations.
6. **Reading the input never returns an error to the code that reads it.** Only a `Source` can
   fail (a JavaScript input), and making every accessor fallible would reach code that cannot
   fail today, such as equality and sorting. A failed expansion leaves the level empty and
   records the error in the evaluation; the evaluator fails with it at the next check of its
   limits (`guardrails`), before calling a host function, and before returning a result. Its
   code is a new `U1004`, whose message carries the source's message; the WebAssembly package
   throws the original `TypeError` instead.

## Implementation notes

- Step 2 (lazy levels with the `Json` source) measured how the pending source is stored, because
  every array and object of an evaluation pays for the test, lazy or not:
  - An array keeps its items in a plain field. A lazy array keeps its source and its read items
    in a separate boxed part, so the other arrays test one plain field. A `OnceCell` around the
    items made `evaluate_document` up to 10% slower on group-by, since every access read
    through the cell.
  - An object keeps its `RefCell` map, and a lazy one fills it in place on first access; its
    source is a plain field and a flag. The accessors are named `borrow()` and `borrow_mut()`,
    like the `RefCell` they replace.
  - Scalars are converted directly when a level is read.
- On the benchmark document, step 2 measured: `evaluate` path 1.9 µs (115 µs before);
  `evaluate_document` within ±3% of step 1; `evaluate` cases that read every item 0–4% slower
  than step 1, interleaving conversion with evaluation. Same-build runs on the maintainer's
  machine sometimes differ by 25–55% on a few `evaluate_document` cases, so a case that moves
  must be measured again before it is trusted.
- Step 3 (the `Source` trait, `U1004` and the WebAssembly source) settled the names:
  - `jlens_core::input::Source<'a>` has one method, `read(&self, sink: &mut Sink<'_, 'a>) ->
    Result<(), String>`. A source stands for an array, an object, or the input itself, which
    writes the input as the only value of its level (nothing for undefined), so the input
    needs no type of its own. `Expression::evaluate_source` takes it, with bindings that live as
    long as `'a`, since values of one evaluation share one lifetime.
  - `Sink` writes scalars (`null`, `bool`, `number`, `string`), nested sources (`array`,
    `object`), and the key of the next field (`key`). Two methods serve speed: `reserve`, a hint
    of the number of values, and `json_members`, which writes a level from JSON text, converted
    at once.
  - A failed read is recorded with the key table, in the state the lazy values of an evaluation
    share. The evaluator tests a flag at each step, after converting the arguments of a host
    function, and when the evaluation ends, whatever its outcome, so that an error caused by the
    empty level never hides `U1004`.
- In WebAssembly, writing a level value by value costs more per value than `JSON.stringify` and
  a parse in Rust: reading the whole benchmark document so took 560 µs, against 320 µs for
  the conversion it replaces. The JavaScript helper therefore returns a level of more than 16
  values as its `JSON.stringify` text, which the source writes with `json_members`; a text
  that `serde_json` cannot read (a lone surrogate, more than 128 levels) is read again, value
  by value. A smaller level returns its tags and numbers in a `Float64Array`, its strings and
  keys in one string, and its nested arrays and objects as nodes that keep their parent's node,
  so that a cycle is detected as `JSON.stringify` detects it.
- On the benchmark document, step 3 measured, against step 2: natively, every case within ±2%;
  in WebAssembly, `evaluate` path 7.6 µs (313 µs before), and the other cases within ±3.5% of
  the conversion through `JSON.stringify`, in one Node.js process.

## Alternatives not taken

- **A borrowed variant, `Value::Json(&'a serde_json::Value)`.** Every place that matches on
  arrays or objects (about 90) would handle two representations, and sequences need owned
  arrays anyway. Lazy levels inside the existing variants keep one representation.
- **Owning the input** (`Rc<serde_json::Value>`, with pointers into it). `evaluate` borrows its
  input, so owning it means a clone, which costs about as much as the conversion. Pointing into
  an `Rc` from a value without a lifetime needs `unsafe` or a self-referential type.
- **Lookups that read one key without expanding the level.** Cheaper on wide objects, but a
  field read twice would be converted twice, into two different values. It can come later, for
  the `Json` source, if a benchmark asks for it.
- **Reading the JSON text lazily in WebAssembly.** `JSON.stringify` alone is over the budget.
- **A `js` feature of `jlens-core`, depending on `js-sys`.** It ties the core library to one
  host; a trait serves the Wasm package and any other.
- **Fallible accessors.** See decision 6.

## Consequences

- An evaluation reads only the levels its expression reaches. A path costs the width of the
  objects it crosses, not the size of the document.
- Expressions that read everything (`**`, `$string($)`, a filter over every item) expand every
  level, as the conversion did. The per-level work must stay within 3% of today's conversion
  natively; in WebAssembly, per-level calls replace one `JSON.stringify` and one parse, and the
  implementation measures both before it is merged.
- `evaluate_document` gains one branch per array or object it reads.
- In WebAssembly, an input that `JSON.stringify` rejects fails only when the expression reads
  the part that `JSON.stringify` rejects, and the input is read during the evaluation rather
  than before it: a host function that changes the input object can change what is read later.
  The WebAssembly guide documents both.
- The lifetime touches most signatures of the evaluator and the function library, with no
  change of behaviour; it lands on its own, before the lazy levels.
- Implementation, in order: the `'a` lifetime alone; the lazy levels with the `Json` source,
  for `evaluate` and bindings, with property tests that `evaluate` and `evaluate_document`
  agree; the `Source` trait, `U1004` and the WebAssembly source, with the benchmarks of
  roadmap 0.8, item 3.

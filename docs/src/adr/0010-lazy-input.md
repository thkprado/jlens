# 0010 — Input documents converted lazily, one level at a time

**Status:** Accepted. Amends [ADR 0003](0003-value-model.md), whose consequences left room for a
borrowed view of the input. Amended in 0.17: a path step reads one field of an object that has
not been read ([below](#amendment-one-field-at-a-time-017)). Amended in 0.19: the arrays and
objects of a level can share one source ([below](#amendment-one-allocation-per-member-019)).
Amended in 0.24: an index reads the items of an array that it picks
([below](#amendment-one-item-at-a-time-024)).

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
     *Amended in 0.17: a lookup reads its one field, and keeps it.*
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
  the `Json` source, if a benchmark asks for it. *Taken in 0.17, keeping the fields read.*
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

## Amendment: one field at a time (0.17)

### Context

Roadmap 0.17, item 9, measured reading a Python `dict` for `evaluate`: to look up `quantity` in
an item of six fields, jlens read the six into a map, 534 ns, where `jsonatapy` looks up the one
field in the `dict`, 66 ns. Over the 200 items of the case names over items, that was most of
the 138 µs jlens took against 25 µs. The `Json` source pays the same way: a lookup converts
every field of its object, strings included, to use one.

### Decision

1. **`Source` gains an optional method, `read_field(key, sink)`.** It writes the value of one
   field of the object the source stands for, as the only value of the sink (nothing when the
   object has no such field), and returns `true`. Its default returns `false`: the source reads
   its object whole only, as before, and the evaluation reads the whole level. The Python source
   implements it with one lookup in the `dict`. The WebAssembly source does not, for now: it
   reads a level with one call into JavaScript, and a call per field has not been measured
   against it.
2. **The path step reads one field.** `functions::lookup`, the step's single site, asks the
   object for its field with `Object::get`, which reads it alone when the object has not been
   read whole. The `Json` source finds the key in its `serde_json` map, comparing the keys of an
   object of up to 16 fields rather than hashing the key (`serde_json`'s map hashes with
   SipHash). Every other access (`borrow()`, `*`, `$keys`, `$string`, comparisons, the result)
   reads the whole level as before.
3. **An array or object is read once; a scalar is read again.** An object keeps the arrays and
   objects read alone in its map, with a state that says the map holds only some fields.
   Reading such a field again finds it there; reading the whole level later reads every field
   from the source, then puts back the values kept, so an array or object read twice is the
   same value. Scalars, the only values whose identity does not show, are not kept: keeping
   one costs a map, its allocations and a key, more than reading it again. A missing field is
   not kept either.
4. **An object that keeps a field is no longer *unread*.** It is converted to JSON, and walked
   by the transform's check for cycles, through its whole level, not as the input it stands
   for: an object it keeps may be one that a `$clone` that does not copy has changed. An object
   of which only scalars were read stays unread.
5. **What does not change.** A level read whole is read once; cycles and deep input fail with
   `U1004` and `U1001`; a source that fails stops the evaluation as decision 6 says. Errors come
   from the part that is read, now at the grain of a field: a Python `dict` holding a `set`
   fails when that field, or the whole `dict`, is read, and a key that is not a `str` only when
   the whole `dict` is.

### Implementation notes

- A first version kept every field read alone. It made the shipping rule, which reads five of
  the six fields of each item, 20% slower with `cargo xtask ab --mode evaluate`: each item paid
  for a map grown field by field. Making room for the whole object at the first field cured the
  shipping rule but took names over items from 52 to 90 µs, and at the second field it cured
  neither. Keeping only arrays and objects removes the map from objects whose scalars are read.
- With `evaluate` on a `serde_json::Value`, against 2bf7c84: names over items 152 → 18.5 µs,
  filter + aggregate 192 → 64 µs, group-by 208 → 101 µs, the shipping rule 216 → 151 µs, the
  path 1.22 µs → 0.60 µs; no case slower (transform and recursion within ±1.5% over 20
  rounds, as when a revision is compared with itself). `evaluate_document` is unchanged.

### Consequences

- A path through wide objects costs the fields it reads, not their width.
- A scalar read several times is converted each time: a string allocates again. An expression
  that reads a few fields of an object, then needs it whole, reads the arrays and objects among
  them twice, for the cost of a lookup each.
- A Python key's `__eq__` can run during a lookup, when a `dict` has a key that is not a `str`
  and whose hash collides with the key looked up. Such a `dict` fails when it is read whole.
- `Object` gains no field: its `read` flag becomes a state of three values.

## Amendment: one allocation per member (0.19)

### Context

Roadmap 0.19, item 3. Each `dict` of a `list` that an evaluation does not read cost jlens 98 ns,
against 30 ns in `jsonatapy`, and about 40% of it was `malloc` and `free`: `Sink::array` and
`Sink::object` box the source they are given, besides allocating the array or object itself
(an array also allocates the part that holds its lazy items). `Sink` takes any type of source,
so it cannot keep the sources of a level together without boxing each, or erasing their type
with `unsafe`, which jlens forbids.

### Decision

1. **A new trait, `input::Sources`.** A value of it holds the sources of several arrays and
   objects, its *members*, and reads the member at an index as a `Source` reads itself:
   `read(index, sink)`, and optionally `read_field(index, key, sink)`.
2. **`Sink::array_of(&sources, index)` and `Sink::object_of(&sources, index)`** write a member,
   taking a reference to an `Rc` of the sources: the array or object holds that `Rc` and the
   index (a `u32`), and allocates nothing more. `Sink::array` and `Sink::object` stay, for
   sources that prefer them: they put their source in an `Rc` of its own, the only member, one
   allocation as the box was.
3. **A lazy value stays as large as before.** The origin of a lazy array or object is either
   JSON or an `Rc` of sources, 16 bytes as the boxed source was. An object keeps its index in
   room its layout left free; an array, in the part that holds its lazy items, which only lazy
   arrays allocate.
4. **The Python source writes a level's arrays and objects as members.** One value per level,
   made at its first array or object, holds the level's Python value and its ancestors (the
   chain that detects cycles, which it replaces) and the members' Python values, the first one
   inline, so that a level holding a single array or object allocates no more than before.
5. **What does not change.** Cycles and deep input fail with `U1004` and `U1001`; a member keeps
   its Python value, so a host function that changes the input between two reads does not
   change which value a member stands for; free-threaded Python is supported as before (no Python
   code runs while a level is read, and a `dict` or `list` is locked while it is).

### Consequences

- With Python values in, `$count(items)` takes 15.4 against 19.0 µs and names over items 35.1
  against 37.4 µs ([performance](../architecture/performance.md)).
- A first version added the index to the origin, which made every lazy object 16 bytes larger,
  those over `serde_json` too: names over items took 17% longer with `cargo xtask ab --mode
  evaluate`. With decision 3, no case is slower but group-by, by 2%.
- The WebAssembly source still boxes each source: it has not been measured with members.

## Amendment: one item at a time (0.24)

### Context

Roadmap 0.24, item 4. An index into an array of the input read the array's whole level:
`items[0].code` over 10,000 items took 0.6 to 0.7 ms through a source or with `evaluate`,
writing a lazy value for each item, against 0.67 µs against a `Document`, and 6.25 ms in the
WebAssembly package's `evaluate`, against 2.2 µs in jsonata-js. The 0.17 amendment let a path
step read one field of an object; an index had no such way.

### Decision

1. **`Source` gains two optional methods, `length()` and `read_item(index, sink)`.** `length`
   tells how many items the array the source stands for has, without reading them; `read_item`
   writes the item at an index below that length as the only value of the sink, and returns
   `true`. Their defaults, `None` and `false`, keep the source reading its array whole.
   `Sources` gains the same, given the member's index. An item read alone needs the length: a
   negative index counts from the end, and an index out of range picks nothing.
2. **An index reads the items it picks.** A predicate that is a number (`items[0]`,
   `items[-1]`), and one built from literals alone, which gives the same value for each item
   (0.19 #2: `items[[0..2]]`), ask the array for its length and for each item they pick, which
   it reads alone when it has not been read whole. Over JSON, the item is taken from the
   `serde_json` array. `$count` reads the length too. A predicate that tests each item, an
   index held in a variable (`items[$i]`) among them, and every other access read the whole
   level as before.
3. **An array or object is read once, as a field is.** The array keeps the arrays and objects
   among the items read alone, with their indexes; reading the whole level later puts them back
   in their places, so an item read twice is the same value. Past 16 of them, finding one would
   cost a comparison with each, and the array is read whole instead; a predicate that picks
   more than 16 items reads the level at once. Scalars are not kept. The length is asked of a
   source once.
4. **An array that keeps an item is no longer *unread*,** for the reason of the 0.17
   amendment's decision 4: it is converted to JSON, and walked by the transform's check for
   cycles, through its whole level.
5. **What does not change.** A level read whole is read once; cycles and deep input fail with
   `U1004` and `U1001`; a source that fails to read an item stops the evaluation as decision 6
   says. Errors come from the part that is read, now at the grain of an item.

### Consequences

- An index into a long array costs the items it picks, not the array's length, when the input
  can read one item: lazy JSON can, and so can a source that implements both methods. With
  `cargo xtask ab`, built aligned against the revision before, `items[0].code` over 10,000
  items takes 1.31 µs instead of 618 µs with `evaluate`, and 1.95 µs instead of 609 µs through
  the program's source; against a `Document` it is unchanged, 0.67 µs. Every other case is
  within 2% in every mode, checked over 20 rounds where a run of 10 left it above.
- Two first versions were slower elsewhere. Reading the 16 first items of `items[[0..19]]`
  alone, then the level, took 7% longer through a source than reading the level: hence the
  rule above. And `filter`, grown by the reads alone, was inlined into `eval`, which made the
  path `customer.name` 3% slower; it is kept out of it.
- The part of a lazy array that holds its pending items is 48 bytes larger, for the length and
  the items kept. Only lazy arrays allocate it; an array that an evaluation builds, or one of a
  `Document`, is unchanged.
- The WebAssembly and Python packages' sources do not implement the methods yet: roadmap 0.24
  #9 and #10.

### Follow-up: an index held in a variable (0.24 #5)

Decision 2 left out an index held in a variable. Roadmap 0.24 #5 counts a variable as a
literal there: a predicate built from literals and variables gives the same value for each
item, so `items[$i]` reads the items it picks, unless the predicate reads `$`, the item, or its
input is a tuple stream, whose variables (`#$i`, `@$o`) change from item to item. Such a
predicate is evaluated without an item, so none is read for it. `($i := 0; items[$i].code)`
over 10,000 items takes 1.97 µs instead of 2.29 ms with `evaluate`.

### Follow-up: the WebAssembly package (0.24 #9)

The WebAssembly package's source implements both methods: `length` asks the JavaScript array
its length, and `read_item` writes one item through the helper that reads a level, which
checks cycles against the same ancestors. Its arrays and objects of a level are now the
members of one `Sources`, the list of nodes the helper returns, one allocation each instead
of two. With `evaluate`, `items[0].code` over 10,000 items takes 6.7 µs instead of 6.19 ms,
and `($i := 0; items[$i].code)` 7.3 µs instead of 6.19 ms; the twelve other cases of the
comparison with jsonata-js are between 3.6% faster and 0.4% slower (the median of four runs
each).

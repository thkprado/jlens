# 0003 — An explicit value model with `Undefined` and sequence flags

**Status:** Accepted

## Context

JSONata semantics depend on distinctions that plain JSON values cannot express:

- **undefined vs null.** A missing path yields *undefined*, which disappears from arrays and
  objects. JSON `null` is a real value.
- **Sequences vs arrays.** Path results are *sequences*. An empty sequence is undefined, and a
  singleton sequence collapses to its only item. Arrays that come from the input, or that an
  explicit `[...]` constructor builds, are never collapsed. The `[]` modifier (`keepArray`) and
  the wrapper that makes an input array count as a single context value add more cases.
- **Callables.** Lambdas, native functions and regexes are first-class values.

The Java port encodes these as a `null` for undefined, a `NULL_VALUE` sentinel for JSON null, and
an `ArrayList` subclass that carries flags. Its documentation names this the most error-prone
part of the port.

## Decision

The evaluator works on its own `Value` type:

```rust
enum Value {
    Undefined, Null, Bool(bool), Number(f64), String(Rc<str>),
    Array(Rc<Array>),            // Array carries: sequence | keep_singleton | outer_wrapper | cons | tuple_stream
    Object(Rc<IndexMap<..>>),    // insertion order is observable in JSONata
    Lambda(..), Native(..), Regex(..), Range(..),
}
```

- Numbers are `f64`, as in JavaScript. Number-to-text conversion uses `ryu-js` so it matches
  JavaScript exactly.
- `Undefined` never crosses the public API: results are `Option<serde_json::Value>`.
- Values are reference-counted (`Rc`) for the lifetime of a single evaluation. The compiled AST
  is immutable and `Send + Sync`, so a compiled `Expression` can be shared between threads, and
  each thread evaluates with its own values.

## Consequences

- Each JSONata rule about undefined and sequences maps to a variant or a flag, and each is
  covered by a dedicated test.
- Input is converted once from `serde_json::Value` at the start of an evaluation. If profiling
  shows that this conversion dominates, a borrowed view of the input can be introduced later.

## Amendment: a string can be two strings joined (0.27)

A string is a `Text`: a text, `Rc<str>`, or two strings that `&` joined and nothing has read yet,
`Rc<Joined>`. `&` copied both sides at each call, so a loop that grows a string with it, such as
`$reduce(strs, function($a, $b){ $a & $b }, '')`, took time growing with the square of the
string: 64,000 short strings took 1.06 s, where jsonata-js, whose strings are V8's ropes, took
99 ms. Appending in place was the other candidate. It needs the left side to be the only
reference to its string, which a loop never has, since the lambda's frame holds `$a` while its
body reads it.

- A result of `&` of 256 bytes or more holds both sides. Its text is made the first time
  something reads it, walking the nodes with a stack of its own, and the sides are then
  dropped, so that a loop that reads each string it grows does not keep every one of them.
- A side shorter than 256 bytes goes into the short text next to it, `(a & b) & c` being
  `a & (b & c)`, so that a loop of short pieces holds a node for each 256 bytes or so, not one
  for each piece. Without that, adding single bytes up to the limit of a string's length would
  have held about 88 bytes of nodes for each byte.
- Dropping the nodes keeps a stack of its own too, as a loop makes them as deep as its count.
- `Text` is two words, like `Rc<str>`: its second kind sits where `Rc<str>`'s pointer cannot be
  null. `Value` stays three words. What reads a string pays one test of that kind; what takes
  it as a key of an object or as the text of a regex makes its text.

The compiler's drop of a `Value` grew with the second kind, and stopped being inlined into the
drop of a list of values, which made each item of a dropped array a call: `$sum(items.quantity)`
against a `Document` took 10% longer under Linux. An array of scalars is now freed without a
drop for each item, which took that away. An array of objects or strings still makes the call
for each item: dropping them by hand cost the index into 10,000 items as much as it gained
`$distinct`, so it was left (performance page, 0.27 #16).

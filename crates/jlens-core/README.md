# jlens-core

A [JSONata](https://jsonata.org/) engine in Rust, with evaluation audits. It is the language
core of [jlens](https://github.com/thkprado/jlens). Try expressions in the
[web editor](https://thkprado.github.io/jlens/editor/), which runs jlens in your browser.

- **Complete JSONata 2.2.** Every operator and built-in function. It passes 1,688 of the 1,690
  cases of the official test suite; the other two are documented.
- **Explainable.** `evaluate_audited` records which comparisons passed or failed, which items
  filters rejected and which values were bound. Labels in ordinary comments
  (`/** @label "Minimum total" */`) name the checks.
- **Tooling-ready.** Recovering parsing, a public syntax tree with spans, a function catalog,
  and stable error codes.
- **Safe to embed.** Timeouts and depth limits. Deep recursion fails with an error instead of
  overflowing the stack. Compiled expressions are `Send + Sync`.

```rust
use jlens_core::{Bindings, Expression};
use serde_json::json;

let expr = Expression::compile("$sum(items[type = $kind].(price * qty))")?;
let order = json!({"items": [
    {"type": "book", "price": 12.5, "qty": 2},
    {"type": "pen", "price": 1.5, "qty": 10}
]});
let total = expr.evaluate(Some(&order), &Bindings::new().var("kind", "book"))?;
assert_eq!(total, Some(json!(25)));
```

## Documentation

- The [API reference](https://docs.rs/jlens-core).
- The [jlens book](https://thkprado.github.io/jlens/): a language reference, and guides to the
  [crate](https://thkprado.github.io/jlens/guides/crate.html), auditing and performance.
- `jlens-schema` adds schema inference and editor support (completion, diagnostics, hover).
- `jlens-cli` is the `jlens` command.

## License

MIT. Parts are ported from [jsonata-js](https://github.com/jsonata-js/jsonata), also MIT; see
`LICENSE`.

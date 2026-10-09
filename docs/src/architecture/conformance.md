# Conformance testing

jlens vendors the language-neutral test suite of the reference implementation
([jsonata-js](https://github.com/jsonata-js/jsonata) **2.2.2**, MIT) in `tests/jsonata-suite/`.
That covers 102 groups and about 1,690 cases. The harness lives in
`crates/jlens-core/tests/conformance.rs` and mirrors `test/run-test-suite.js`:

- Each case is one test, named `<group>/<file>`, or `<group>/<file>#<index>` for files that hold
  an array of cases.
- The input comes from `data`. If there is no `data`, the input is the named `dataset`;
  `"dataset": null` means *undefined* input.
- `timelimit` and `depth` map to `Options::timeout` and `Options::max_depth`.
- Each case states exactly one expectation:
  - `result`, compared with deep equality where numbers compare as JavaScript numbers, so `1`
    equals `1.0`;
  - `undefinedResult`;
  - `code`, optionally with `token`;
  - `error`, an object whose fields must all match.

## The baseline ratchet

`tests/conformance/baseline.txt` lists the cases that must pass, and `cargo test` fails only if
one of them regresses. Every other case is reported as *ignored*. To work on a group, run it
with:

```sh
cargo test -p jlens-core --test conformance -- --include-ignored array-constructor/
```

To measure progress and lock in new passes:

```sh
cargo xtask conformance                    # writes tests/conformance/REPORT.md
cargo xtask conformance --update-baseline  # also rewrites baseline.txt
```

## Overrides

`tests/conformance/overrides.json` records every intentional deviation. Each entry needs a
`reason` and may `skip` the case, or replace its expectation with `alternateResult` or
`alternateCode`. The current overrides are:

| Case | Reason |
|---|---|
| `function-encodeUrl/case002`, `function-encodeUrlComponent/case002` | The data contains a lone UTF-16 surrogate (U+D800), which Rust's UTF-8 strings cannot represent. |

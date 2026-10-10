# Vendored JSONata test suite

Pinned copy of `test/test-suite` from [jsonata-js](https://github.com/jsonata-js/jsonata)
**v2.2.2**, distributed under the MIT license (see `LICENSE` in this directory). Do not edit
these files by hand; record deviations in `../conformance/overrides.json` instead. The only
edit is the one `update-suite` applies (`SUITE_EDITS` in `xtask/src/main.rs`), which renames a
company in the sample data. Update with `cargo xtask update-suite <path>`.

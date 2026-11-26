# Vendored JSONata test suite

Pinned copy of `test/test-suite` from [jsonata-js](https://github.com/jsonata-js/jsonata)
at tag **v2.3.0** (commit `d19d8d7`; the suite is unchanged since `1ab3eec` of its main
branch), distributed under the MIT license (see `LICENSE` in this directory). Do not edit
these files by hand; record deviations in `../conformance/overrides.json` instead. The only
edit is the one `update-suite` applies (`SUITE_EDITS` in `xtask/src/main.rs`), which renames a
company in the sample data. Update with `cargo xtask update-suite <path>`.

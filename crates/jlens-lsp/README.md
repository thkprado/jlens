# jlens-lsp

A language server ([LSP](https://microsoft.github.io/language-server-protocol/)) for
[JSONata](https://jsonata.org/) expressions. Part of [jlens](https://github.com/thkprado/jlens).

- **Completion** of fields from a JSON Schema of the input documents, of variables, and of
  built-in and host functions.
- **Diagnostics**: syntax errors, annotation problems, and fields the schema does not have
  ("did you mean").
- **Hover** and **signature help**.

Editors run it as `jlens lsp` (`cargo install jlens-cli`). A `jlens.json` at the root of each
workspace folder says which schema, variables and host functions apply to which files:

```json
{
  "rules/**/*.jsonata": {
    "schema": "schemas/order.schema.json",
    "variables": {"minimum": {"type": "number"}},
    "functions": [{"name": "shipping", "signature": "<ns:n>", "summary": "Shipping cost"}]
  }
}
```

To embed the server, run `jlens_lsp::run` on an `lsp_server::Connection`. The
[editors guide](https://github.com/thkprado/jlens/blob/main/docs/src/guides/editors.md) covers
the configuration and the setup of Neovim, Helix and Emacs.

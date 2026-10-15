# Editors (language server)

`jlens lsp` is a language server: editors that speak the
[Language Server Protocol](https://microsoft.github.io/language-server-protocol/) get, in
JSONata files:
- completion of fields, variables, and built-in and host functions;
- diagnostics: syntax errors, annotation problems, and fields that the schema does not have,
  with a "did you mean";
- hover, with the type and description of fields and the documentation of functions;
- signature help in function calls.

It is the [language service](schema.md) of the web editor, behind a standard protocol. Install
it with the CLI:

```sh
cargo install jlens-cli
jlens lsp          # what editors run; it speaks LSP on standard input and output
```

## Configuration: `jlens.json`

A `jlens.json` at the root of a workspace folder says which schema, variables and
[host functions](crate.md#host-functions) apply to which expression files:

```json
{
  "rules/**/*.jsonata": {
    "schema": "schemas/order.schema.json",
    "variables": {"minimum": {"type": "number"}, "region": null},
    "functions": [
      {"name": "shipping", "signature": "<ns:n>", "params": ["weight", "region"],
       "summary": "Shipping cost of a weight to a region"}
    ]
  },
  "**/*.jsonata": {}
}
```

- **Keys** are glob patterns, matched against the file's path relative to the folder of
  `jlens.json`. `*` stays within a directory and `**` crosses directories. The first matching
  pattern applies; files that no pattern matches get completion and diagnostics without a
  schema.
- **`schema`** is a JSON Schema of the input documents, relative to the folder of `jlens.json`.
  `jlens schema infer` creates one from sample documents; see the [schema guide](schema.md).
- **`variables`** declares the variables the expressions are evaluated with, with a JSON Schema
  of each value, or `null` when the shape is unknown. Names can start with `$`.
- **`functions`** declares the host functions: `name`, and optionally `signature`, `params` and
  `summary`.

All settings are optional. When `jlens.json` or a JSON file changes, the server reloads the
configuration and the schemas, and checks the open files again. It relies on the editor's file
watching, or on the file being saved in the editor. A configuration that cannot be read is
reported as a message, and its files are checked without settings.

The client can also pass the same object as `initializationOptions`, relative to the first
workspace folder. Its patterns come before those of `jlens.json`.

[`jlens check`](cli.md#configuration-jlensjson) reads the same file, so that CI reports what the
editor shows.

## Neovim

With Neovim 0.11 or later, in `init.lua`:

```lua
vim.filetype.add({ extension = { jsonata = "jsonata" } })
vim.lsp.config("jlens", {
  cmd = { "jlens", "lsp" },
  filetypes = { "jsonata" },
  root_markers = { "jlens.json", ".git" },
})
vim.lsp.enable("jlens")
```

## Helix

In `languages.toml`:

```toml
[[language]]
name = "jsonata"
scope = "source.jsonata"
file-types = ["jsonata"]
roots = ["jlens.json"]
language-servers = ["jlens"]

[language-server.jlens]
command = "jlens"
args = ["lsp"]
```

## Emacs

With Eglot (built into Emacs 29):

```elisp
(define-derived-mode jsonata-mode prog-mode "JSONata")
(add-to-list 'auto-mode-alist '("\.jsonata\'" . jsonata-mode))
(with-eval-after-load 'eglot
  (add-to-list 'eglot-server-programs '(jsonata-mode "jlens" "lsp")))
```

## Visual Studio Code

The jlens extension (`thkprado.jlens`, from 0.15) highlights JSONata and starts `jlens lsp` for
the `jsonata` language, in files ending in `.jsonata`. It looks for the server in this order:

1. the setting `jlens.server.path`, the path of a `jlens` program;
2. the `jlens` binary bundled with the extension, in its packages for Linux, macOS and Windows on
   x86-64, and Linux and macOS on ARM64;
3. `jlens` on the `PATH`, on other platforms.

When it finds none, it says so, and highlighting keeps working. `jlens.server.path` is read
from user settings only, never from a workspace's: a repository cannot choose which program
runs. The command **jlens: Restart Language Server** starts the server again, after an update
of `jlens` for example, and changing the setting does too.

The **jlens** output channel shows the server's version and path, and its messages. The setting
`jlens.trace.server` (`messages` or `verbose`) adds the protocol's messages to it.

The extension also checks `jlens.json` as it is edited, with a JSON Schema: unknown settings,
misspelled ones and wrong types are underlined, and hovers describe each setting.

If another JSONata extension is installed, VS Code uses both: disable the other's language
features, or you get the diagnostics of both. In the browser, the
[WebAssembly package](wasm.md#the-language-service) offers the same service to CodeMirror and
Monaco.

## Positions

The server counts positions in UTF-16 code units, the protocol's default, or in bytes when the
client offers UTF-8 (`general.positionEncodings`). Documents are synchronized incrementally.
Positions past the end of a line or of the document are clamped to it, as the protocol requires.

## Errors

A bug in jlens that panics while the server handles a request does not stop the server: the
request is answered with an `InternalError`, and a panic while it handles a notification (such
as checking a document after a change) is logged with `window/logMessage`. The panic itself is
printed on the standard error, which editors keep in the server's log. Please report it, with
the text of the document.

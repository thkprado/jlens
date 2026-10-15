# jlens — JSONata

[JSONata](https://jsonata.org) in Visual Studio Code, with the language server of
[jlens](https://thkprado.github.io/jlens/), a JSONata 2.2 engine:

- highlighting of `.jsonata` files;
- diagnostics: syntax errors, unknown functions, wrong argument counts, and fields that the
  input's schema does not have, with a "did you mean";
- completion of fields, variables and functions, hovers, and signature help;
- a `jlens.json` at the root of the workspace says which JSON Schema, variables and host
  functions apply to which files; the extension checks it as it is edited.

The packages for Linux, macOS and Windows on x86-64, and Linux and macOS on ARM64, bundle the
`jlens` binary. On other platforms, install the [jlens CLI](https://thkprado.github.io/jlens/getting-started/installation.html)
and the extension uses the `jlens` on the `PATH`.

## Settings

- `jlens.server.path`: another `jlens` program to run, read from user settings only.
- `jlens.trace.server`: logs the protocol's messages in the **jlens** output channel.

The command **jlens: Restart Language Server** starts the server again.

If another JSONata extension is installed, disable its language features, or you get the
diagnostics of both.

See the [editors guide](https://thkprado.github.io/jlens/guides/editors.html) for `jlens.json`
and the other editors.

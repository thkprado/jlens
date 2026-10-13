# jlens-cli

`jlens`: evaluate, check and explain [JSONata](https://jsonata.org/) expressions from the
command line. Part of [jlens](https://github.com/thkprado/jlens).

```sh
cargo binstall jlens-cli     # a prebuilt binary, from the GitHub release
cargo install jlens-cli      # or build it from source
npx jlens-cli --version      # or from npm, in JavaScript projects (from 0.14.0 on)
```

```text
$ jlens eval -i order.json '$sum(items.(price * qty))'
25

$ jlens eval --ndjson --filter -i orders.ndjson 'total > 1000'     # grep for JSON
$ jlens eval -f rule.jsonata -i order.json --explain               # why this result?
$ jlens schema infer samples/*.json -o order.schema.json           # learn the structure
$ jlens check --schema order.schema.json -f rule.jsonata           # catch misspelt fields
$ jlens test tests/*.json                                          # run the rules' cases
$ jlens functions formatNumber                                     # built-in reference
```

| Command | Purpose |
|---|---|
| `jlens eval` | Evaluate against a document, a stream of documents (`--ndjson`), or nothing; filter, explain or audit |
| `jlens check` | Report syntax errors and unknown fields, for editors and CI (`--json`) |
| `jlens test` | Run files of cases (input, bindings, expected result or error code) |
| `jlens schema infer` / `complete` | Infer a JSON Schema from samples; list completions |
| `jlens functions` | Describe the built-in functions |
| `jlens ast` | Show the tokens or the syntax tree |

The [CLI guide](https://thkprado.github.io/jlens/guides/cli.html) of the
[jlens book](https://thkprado.github.io/jlens/) documents every option and exit code. The
[web editor](https://thkprado.github.io/jlens/editor/) runs the same engine in your browser.

## License

MIT.

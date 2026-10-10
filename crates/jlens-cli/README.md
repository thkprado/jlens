# jlens-cli

`jlens`: evaluate, check and explain [JSONata](https://jsonata.org/) expressions from the
command line. Part of [jlens](https://github.com/thkprado/jlens).

```sh
cargo install jlens-cli
```

```text
$ jlens eval -i order.json '$sum(items.(price * qty))'
25

$ jlens eval --ndjson --filter -i orders.ndjson 'total > 1000'     # grep for JSON
$ jlens eval -f rule.jsonata -i order.json --explain               # why this result?
$ jlens schema infer samples/*.json -o order.schema.json           # learn the structure
$ jlens check --schema order.schema.json -f rule.jsonata           # catch misspelt fields
$ jlens functions formatNumber                                     # built-in reference
```

| Command | Purpose |
|---|---|
| `jlens eval` | Evaluate against a document, a stream of documents (`--ndjson`), or nothing; filter, explain or audit |
| `jlens check` | Report syntax errors and unknown fields, for editors and CI (`--json`) |
| `jlens schema infer` / `complete` | Infer a JSON Schema from samples; list completions |
| `jlens functions` | Describe the built-in functions |
| `jlens ast` | Show the tokens or the syntax tree |

The CLI guide of the jlens book documents every option and exit code.

## License

MIT.

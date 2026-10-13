# jlens-cli

`jlens`: evaluate, check and explain [JSONata](https://jsonata.org/) expressions from the
command line. Part of [jlens](https://github.com/thkprado/jlens).

```sh
npx jlens-cli --version          # run it once
npm install --save-dev jlens-cli # or add it to a project, then run `jlens` in npm scripts
```

The package runs the native `jlens` binary of the GitHub release, which npm installs from one
of the optional dependencies `jlens-cli-<os>-<cpu>`: Linux (x64 and arm64, static), macOS (x64
and arm64) and Windows (x64). It has no install scripts.

```text
$ jlens eval -i order.json '$sum(items.(price * qty))'
25

$ jlens eval --ndjson --filter -i orders.ndjson 'total > 1000'     # grep for JSON
$ jlens eval -f rule.jsonata -i order.json --explain               # why this result?
$ jlens schema infer samples/*.json -o order.schema.json           # learn the structure
$ jlens check --schema order.schema.json -f rule.jsonata           # catch misspelt fields
$ jlens test tests/*.json                                          # run the rules' cases
```

The [CLI guide](https://thkprado.github.io/jlens/guides/cli.html) of the
[jlens book](https://thkprado.github.io/jlens/) documents every command, option and exit code.
To use JSONata from JavaScript code instead, install the library
[`jlens-wasm`](https://www.npmjs.com/package/jlens-wasm).

## License

MIT.

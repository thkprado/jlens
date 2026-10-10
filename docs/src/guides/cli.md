# Command-line interface

The `jlens` binary evaluates, checks, explains and inspects JSONata expressions.

```text
jlens eval      <EXPR> | -f FILE  [-i INPUT | --no-input] [--ndjson] [--filter]
                [-b BINDINGS] [-c] [-r] [--timeout MS] [--max-depth N]
                [--explain | --audit json|text] [--audit-level decisions|full] [--trace-log FILE]
jlens check     <EXPR> | -f FILE  [--json] [--schema SCHEMA]
jlens functions [NAME] [--json]
jlens schema infer    SAMPLES...  [--each] [--overrides FILE] [-o SCHEMA]
jlens schema complete <EXPR> | -f FILE  [--schema SCHEMA] [--cursor N] [--json]
jlens ast       <EXPR> | -f FILE  [--tokens | --raw] [--recover]
```

Every command takes the expression either as an argument or from a file with `-f`. Prefer `-f`
for anything longer than a one-liner. Shells interpret `$`, `"` and `*` differently, and Windows
shells (`cmd.exe`, PowerShell) in particular make quoting expressions awkward. Glob patterns in
`jlens schema infer` are expanded by `jlens` itself, so they work in every shell.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. `check` may still have printed warnings |
| `1` | The expression has errors: a syntax error, an evaluation error, or (with `--ndjson`) at least one line that failed |
| `2` | Usage or I/O problem: unreadable file, invalid JSON input, bad arguments |

Writing to a closed pipe, as in `jlens functions | head`, ends quietly with code 0.

## `jlens eval`

Evaluates the expression against a JSON document and prints the result. The document comes from
`-i FILE`, or from standard input. An undefined result prints nothing.

```text
$ jlens eval -f fixtures/shipping-rule.jsonata -i fixtures/online-store-order.json
{
  "action": "FREE_SHIPPING",
  "code": "OK_200"
}
```

| Option | Effect |
|---|---|
| `-i, --input FILE` | The input document (default: standard input) |
| `--no-input` | Evaluate without an input document (`$` is undefined) |
| `-b, --bindings FILE` | A JSON object whose fields are bound as variables: `{"limit": 10}` makes `$limit` available |
| `-c, --compact` | One line per result |
| `-r, --raw-output` | Print strings without quotes, like `jq -r` |
| `--timeout MS` | Stop an evaluation that runs longer (error `D1012`) |
| `--max-depth N` | Stop an evaluation that nests deeper (error `D1011`); the default is 50,000 |

Results are printed the way JavaScript prints JSON, so they match jsonata-js output: `2^64` prints
as `18446744073709552000` and `1e21` as `1e+21`.

### Many documents: `--ndjson`

With `--ndjson`, the input has one JSON document per line, and the expression is evaluated for
each one. Results are printed one per line, as they are produced, so arbitrarily large inputs
stream through in constant memory.
- Blank lines are skipped.
- Lines that are not valid JSON, and evaluations that fail, are reported on standard error with
  their line number. Processing continues, and the exit code becomes 1.

```text
$ jlens eval --ndjson -i fixtures/orders.ndjson '$sum(items.(unitPrice * quantity))'
50000
300
60000
line 5: invalid JSON: expected ident at line 1 column 2
```

### Selecting documents: `--filter`

With `--filter`, `jlens` prints the input **documents** for which the expression is truthy,
instead of the results. Truthiness follows `$boolean`: `false`, `0`, `""`, `null`, empty arrays,
empty objects and undefined are false. Combined with `--ndjson`, this works like a `grep` for
JSON:

```text
$ jlens eval --ndjson --filter -i fixtures/orders.ndjson \
    "customer.login in ['alice.walker99', 'bob.harrison42'] and \$sum(items.(unitPrice * quantity)) > 50000"
{"number":"SO-3","customer":{"login":"bob.harrison42"},"items":[{"type":"toy","unitPrice":60000,"quantity":1}]}
```

### Explaining results

| Option | Effect |
|---|---|
| `--explain` | Prints an [explanation](audit.md) of how the result was reached, instead of the result |
| `--audit json` | Prints the full audit tree as JSON (`--audit text` is the same as `--explain`) |
| `--audit-level full` | Records every evaluated node, not only decisions |
| `--trace-log FILE` | Writes every evaluation event to `FILE` as JSON Lines; combines with the options above |

```text
$ jlens eval --no-input --explain '/** @label "Positive" */ 3 > 0 and /** @label "Small" */ 3 < 2'
result: false
✘ Small — 3 < 2 → false (3 < 2)
```

These options apply to a single document; they cannot be combined with `--ndjson` or
`--filter`.

## `jlens check`

Parses the expression with error recovery and reports every problem it finds. With `--schema`,
it also reports field names that the schema does not have (see the
[schema guide](schema.md)). Each problem is shown in context:

```text
$ jlens check 'Account.Order[0;].Product'
error[S0202]: Expected "]", got ";"
 --> <expr>:1:16
  |
1 | Account.Order[0;].Product
  |                ^
<expr>: 1 error(s), 0 warning(s)
```

Errors make the exit code 1; warnings alone do not. With `--json`, the diagnostics are printed as
an array, which suits editors and CI scripts. Spans are byte offsets into the expression:

```json
[
  {
    "severity": "error",
    "code": "S0207",
    "message": "Unexpected end of expression",
    "span": { "start": 3, "end": 3 }
  }
]
```

To validate a directory of rules in CI:

```sh
for f in rules/*.jsonata; do jlens check --schema order.schema.json -f "$f" || exit 1; done
```

## `jlens functions`

Lists the built-in functions by category, with their parameters and a summary. Given a name
(with or without `$`), it describes one function; `--json` prints the catalog as JSON.

```text
$ jlens functions formatNumber
$formatNumber($value, $picture, $options)
  signature: <n-so?:s>
  category:  String
  Formats a number with an XPath picture string
```

The [function reference](../language/functions.md) is generated from the same catalog.

## `jlens schema`

| Command | Purpose |
|---|---|
| `jlens schema infer SAMPLES… [--each] [--overrides FILE] [-o SCHEMA]` | Infer a JSON Schema from sample documents |
| `jlens schema complete <EXPR> --schema SCHEMA [--cursor N] [--json]` | List completions at a cursor position, given in characters (default: the end) |

Both are described in the [schema guide](schema.md).

## `jlens ast`

Prints the processed syntax tree as JSON. This is the tree the evaluator walks, with node ids,
spans, stages and annotations. Flags show earlier stages of the pipeline:

| Flag | Output |
|---|---|
| (none) | Processed tree (`Ast`) |
| `--raw` | Raw parse tree, before post-processing |
| `--tokens` | Token stream, with spans |
| `--recover` | Keep going after syntax errors; print the partial tree, with diagnostics on standard error |

`jlens ast` is mostly useful for understanding how an expression is read, for example which part
an [annotation](../language/annotations.md) covers. The [parsing chapter](../architecture/parser.md)
explains the tree.

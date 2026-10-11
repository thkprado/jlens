# Command-line interface

The `jlens` binary evaluates, checks, explains and inspects JSONata expressions.

```text
jlens eval      <EXPR> | -f FILE  [-i INPUT | --no-input] [--ndjson] [--filter]
                [-b BINDINGS] [-c | -r | --json] [--timeout MS] [--max-depth N]
                [--now TIME] [--seed N]
                [--explain | --audit json|text] [--audit-level decisions|full] [--trace-log FILE]
jlens check     <EXPR> | -f FILE...  [--json] [--deny-warnings] [--schema SCHEMA]
jlens functions [NAME] [--json]
jlens errors    [CODE] [--json]
jlens schema infer    SAMPLES...  [--each] [--overrides FILE] [-o SCHEMA]
jlens schema complete <EXPR> | -f FILE  [--schema SCHEMA] [--cursor N] [--json]
jlens ast       <EXPR> | -f FILE  [--tokens | --raw] [--recover]
```

Every command takes the expression either as an argument or from a file with `-f`. Prefer `-f`
for anything longer than a one-liner. Shells interpret `$`, `"` and `*` differently, and Windows
shells (`cmd.exe`, PowerShell) in particular make quoting expressions awkward. Glob patterns in
`jlens check -f` and `jlens schema infer` are expanded by `jlens` itself, so they work in every
shell.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. `check` may still have printed warnings (unless `--deny-warnings`) |
| `1` | The expression has errors: a syntax error, an evaluation error, or (with `--ndjson`) at least one line that failed |
| `2` | Usage or I/O problem: unreadable file, invalid JSON input, bad arguments, a terminal on standard input |

Writing to a closed pipe, as in `jlens functions | head`, ends quietly with code 0.

## For scripts and agents

`jlens` keeps a contract with the programs that run it, recorded in
[ADR 0008](../adr/0008-cli-machine-contract.md):

- **Standard output is data**: results, JSON from `--json`, catalogs. **Standard error is for
  people**: rendered diagnostics, summaries, usage errors. Never parse standard error.
- **Decide on the exit code** (see the table above), not on whether anything was printed: an
  undefined result prints nothing and still exits with 0.
- **Match errors by code** (`S0202`, `T2001`, `D1012`…), never by message. Codes are listed in
  the [error reference](../language/errors.md).
- **The format never depends on the terminal.** A command prints the same thing when an agent
  runs it and when a person copies it into a shell. Ask for machine output with a flag.
- **Pass the expression with `-f FILE`** and the input with `-i FILE` or `--no-input`. This
  avoids shell quoting. An `eval` that would read its input from a terminal fails with code 2
  instead of waiting.
- **Use `eval --json`** to get every outcome, errors included, as one line of JSON on standard
  output (see [below](#machine-output---json)), and `check --json` for the diagnostics of
  every file as one JSON array.
- **Fix the clock and the seed** with `--now` and `--seed` when a result depends on `$now()`,
  `$millis()`, `$random()` or `$shuffle()`, so that the same command prints the same bytes when
  it is run again.

## `jlens eval`

Evaluates the expression against a JSON document and prints the result. The document comes from
`-i FILE`, or from standard input. An undefined result prints nothing. When standard input is a
terminal and neither `-i` nor `--no-input` is given, `eval` fails with exit code 2 instead of
waiting for a document.

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
| `--json` | Print a JSON envelope instead of the bare result (see below) |
| `--timeout MS` | Stop an evaluation that runs longer (error `D1012`) |
| `--max-depth N` | Stop an evaluation that nests deeper (error `D1011`); the default is 50,000 |
| `--now TIME` | The time seen by `$now()` and `$millis()`: milliseconds since the epoch, or an ISO 8601 timestamp such as `2026-01-01T00:00:00Z` |
| `--seed N` | Seed `$random()` and `$shuffle()`: the same seed gives the same results |

Results are printed the way JavaScript prints JSON, so they match jsonata-js output: `2^64` prints
as `18446744073709552000` and `1e21` as `1e+21`.

### Machine output: `--json`

With `--json`, every outcome is one compact JSON object, the *envelope*, on one line of
standard output. Nothing needs to be read from standard error.

| Outcome | Envelope |
|---|---|
| A result | `{"ok":true,"result":…}` |
| An undefined result | `{"ok":true,"undefined":true}` |
| An error | `{"ok":false,"error":{"code":…,"message":…,"position":…,"line":…,"column":…}}` |

```text
$ jlens eval --json -i fixtures/online-store-order.json 'customer.name + 1'
{"ok":false,"error":{"code":"T2001","message":"The left side of the \"+\" operator must evaluate to a number","position":15,"line":1,"column":1}}
```

- Decide on `ok`, and match errors by `code`; `message` is for people and may change.
- `line` and `column` (1-based, the column in characters) mark the start of the offending text
  in the expression, as the human report does. `position` is the jsonata-js position, a UTF-16
  offset usually just past the offending token. The three are absent when the error has no
  location.
- An input document that is not valid JSON gives an envelope with code `U2001`, and exit
  code 2 as without `--json`.
- With `--filter`, a matching document is the `result`, and a document that does not match
  prints nothing.
- With `--ndjson`, every envelope also carries `line`, the number of its input line, so that
  results and errors can be paired with their input. A line that is not valid JSON gives a
  `U2001` envelope, and processing continues.
- `--json` cannot be combined with `-r`, `--explain` or `--audit`.
- Fields are only ever added to the envelope: ignore the ones you do not know.

### Many documents: `--ndjson`

With `--ndjson`, the input has one JSON document per line, and the expression is evaluated for
each one. Results are printed one per line, as they are produced, so arbitrarily large inputs
stream through in constant memory.
- Blank lines are skipped.
- Lines that are not valid JSON, and evaluations that fail, are reported on standard error with
  their line number. Processing continues, and the exit code becomes 1.
- With `--json`, each result and each error is an envelope that carries its line number, and
  nothing is written to standard error.

```text
$ jlens eval --ndjson -i fixtures/orders.ndjson '$sum(items.(unitPrice * quantity))'
50000
300
60000
line 5: invalid JSON: expected ident at line 1 column 2
```

```text
$ jlens eval --ndjson --json -i fixtures/orders.ndjson '$sum(items.(unitPrice * quantity))'
{"ok":true,"line":1,"result":50000}
{"ok":true,"line":2,"result":300}
{"ok":true,"line":4,"result":60000}
{"ok":false,"line":5,"error":{"code":"U2001","message":"The input is not valid JSON: expected ident at line 1 column 2"}}
{"ok":true,"line":6,"undefined":true}
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

Parses the expression with error recovery and reports every problem it finds. It warns about
calls of functions that are neither built in nor bound in the expression (`W0202`, with a
suggestion such as `$summ` → `$sum`), and about calls with a number of arguments the function's
signature does not accept (`W0203`). With `--schema`, it also reports field names that the
schema does not have (see the [schema guide](schema.md)). Each problem is shown in context:

```text
$ jlens check 'Account.Order[0;].Product'
error[S0202]: Expected "]", got ";"
 --> <expr>:1:16
  |
1 | Account.Order[0;].Product
  |                ^
<expr>: 1 error(s), 0 warning(s)
```

`-f` takes several files, and glob patterns such as `rules/*.jsonata`. Each file is reported in
turn, followed by a total. Errors make the exit code 1; warnings alone do not, unless
`--deny-warnings` is given. An unreadable file, or a pattern that matches nothing, ends with exit
code 2 before anything is checked.

With `--json`, the diagnostics of all the files are printed as one compact JSON array on
standard output, which suits editors, CI scripts and agents. Each diagnostic has `file` (absent
for an expression given as an argument), the 1-based `line` and `column` (counted in
characters), then the severity, code, message and `span`, in byte offsets into the file:

```text
$ jlens check --json 'a +'
[{"line":1,"column":4,"severity":"error","code":"S0207","message":"Unexpected end of expression","span":{"start":3,"end":3}}]
```

The host functions an application provides are not known to `check` yet, so their calls get
`W0202`. The [language server](editors.md) knows the ones declared in `jlens.json`.

To validate a directory of rules in CI, warnings included:

```sh
jlens check --deny-warnings --schema order.schema.json -f 'rules/*.jsonata'
```

## `jlens functions`

Lists the built-in functions by category, with their parameters and a summary. Given a name
(with or without `$`), it describes one function; `--json` prints the catalog as JSON.

```text
$ jlens functions formatNumber
$formatNumber($value, $picture, $options)
  signature: <n-so?:s>
  category:  Numeric
  Formats a number with an XPath picture string
```

The [function reference](../language/functions.md) is generated from the same catalog.

## `jlens errors`

Lists the error and warning codes by category, with their message templates. Given a code (in
any case), it describes one; `--json` prints the catalog as JSON, each entry with its `code`,
`category` (`static`, `type`, `dynamic`, `guardrail` or `warning`) and `template`.

```text
$ jlens errors S0202
S0202
  category: Static errors (S)
  message:  Expected {{value}}, got {{token}}
```

The [error reference](../language/errors.md) is generated from the same catalog.

## `jlens schema`

| Command | Purpose |
|---|---|
| `jlens schema infer SAMPLES… [--each] [--overrides FILE] [-o SCHEMA]` | Infer a JSON Schema from sample documents |
| `jlens schema complete <EXPR> --schema SCHEMA [--cursor N] [--json]` | List completions at a cursor position, given in characters (default: the end) |

Both are described in the [schema guide](schema.md).

## `jlens lsp`

Runs the language server on standard input and output. Editors start it; it is configured by
the `jlens.json` of each workspace folder. See the [editors guide](editors.md).

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

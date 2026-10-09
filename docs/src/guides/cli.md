# Command-line interface

The `jlens` binary evaluates, checks and inspects JSONata expressions.

```text
jlens eval   <EXPR> | -f FILE  [-i input.json | --no-input] [-b bindings.json] [--compact]
jlens check  <EXPR> | -f FILE  [--json]
jlens ast    <EXPR> | -f FILE  [--tokens | --raw] [--recover]
```

Every command takes the expression either as an argument or from a file with `-f`. Prefer `-f`
for anything longer than a one-liner, especially on Windows, where quoting `$` and `"` in
`cmd.exe` and PowerShell is awkward.

> `jlens eval` evaluates expressions. Only a core set of built-in functions (`$sum`, `$count`,
> `$string`, `$exists`, ...) is available until phase 3 of the [roadmap](../architecture/roadmap.md)
> completes the library.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. `check` may still have printed warnings |
| `1` | The expression has errors: a syntax error, or an evaluation error |
| `2` | Usage or I/O problem: unreadable file, invalid JSON input, bad arguments |

## `jlens check`

Parses the expression with error recovery and reports every problem it finds, both errors and
annotation warnings. Each problem is printed with its location:

```text
$ jlens check 'Account.Order[0;].Product'
error[S0202]: Expected "]", got ";"
 --> <expr>:1:16
  |
1 | Account.Order[0;].Product
  |                ^
<expr>: 1 error(s), 0 warning(s)
```

With `--json`, the diagnostics are printed as an array, which suits editors and CI scripts.
Spans are byte offsets into the expression:

```json
[
  {
    "severity": "error",
    "code": "S0202",
    "message": "Expected \"]\", got \";\"",
    "span": { "start": 15, "end": 16 }
  }
]
```

Use `check` in CI to validate a directory of rule files:

```sh
for f in rules/*.jsonata; do jlens check -f "$f" || exit 1; done
```

## `jlens ast`

Prints the processed syntax tree as JSON. This is the tree the evaluator walks, with node ids,
spans, stages and annotations. Two flags show earlier stages of the pipeline:

| Flag | Output |
|---|---|
| (none) | Processed tree (`Ast`) |
| `--raw` | Raw parse tree, before post-processing |
| `--tokens` | Token stream, with spans |
| `--recover` | Keep going after syntax errors and print the partial tree, with diagnostics on stderr |

```text
$ jlens ast --tokens 'a / /x/i'
[
  { "type": "name", "value": "a", "span": { "start": 0, "end": 1 } },
  { "type": "operator", "value": "/", "span": { "start": 2, "end": 3 } },
  { "type": "regex", "value": "/x/i", "span": { "start": 4, "end": 8 } }
]
```

`jlens ast` is mostly a tool for understanding how an expression is read, for example to see
which part an [annotation](../language/annotations.md) covers. The parser chapter in the
[architecture](../architecture/parser.md) section explains the tree.

# Schema and autocomplete

jlens can learn the structure of your documents from samples and use it to help people write
expressions. The language service then provides:
- completion of field names that knows which object you are in;
- warnings for fields that do not exist;
- hover documentation;
- signature help for functions.

The `jlens-schema` crate provides both parts, and the CLI and the WebAssembly package expose
them.

## 1. Infer a schema from samples

```text
$ jlens schema infer samples/*.json -o order.schema.json
inferred from 120 sample(s): order.schema.json
```

Glob patterns are expanded by `jlens` itself, so they also work in Windows shells. With `--each`,
a sample file holding a top-level array counts as one sample per item.

Inference records, for every place in the documents:

| What | How |
|---|---|
| Types | Every JSON type seen. `integer` is used while all numbers are written without a fraction; `1500.0` makes it `number` |
| Fields | In first-seen order, with `required` when every object had the field, and the fraction of objects that had it otherwise |
| Array items | The merged shape of all items of all arrays seen at that place |
| Numbers | Minimum and maximum |
| String formats | `date-time`, `date`, `time`, `email`, `uuid` or `uri`, when every string seen has that format |
| Enumerations | When the values come from a small set (at most 10) and repeat |
| Examples | Up to three distinct values |

The result is a standard **JSON Schema 2020-12** document. Fields that are not required carry
`x-jlens-presence`, the fraction of objects that had them. You can commit the file, review it,
and edit it like any JSON Schema. jlens also reads JSON Schemas written by hand or by other
tools: it supports `$ref` to local definitions, and it merges `anyOf`, `oneOf` and `allOf`.

## 2. Describe the fields

Samples tell jlens what fields exist, but not what they mean. Overrides add that information.
An overrides file maps paths to settings; `[]` steps into array items:

```json
{
  "items[].type": {
    "description": "Product category used for catalog classification",
    "enum": ["apparel", "electronic", "toy"]
  },
  "items[].tax": { "label": "Import tax" },
  "customer.login": { "description": "Login chosen at sign-up" },
  "internalRevision": { "hidden": true }
}
```

| Setting | Effect |
|---|---|
| `description` | Shown in completion and hover |
| `label` | A display name, shown in bold |
| `enum` | The allowed values |
| `examples` | Example values |
| `format` | A format name |
| `type` | Replaces the inferred types, e.g. `["string", "null"]` |
| `hidden` | Never suggested by completion |

```text
$ jlens schema infer samples/*.json --overrides overrides.json -o order.schema.json
```

A path that does not exist yet is created, so overrides can also declare fields that are absent
from the samples.

## 3. Use it

### In the terminal

```text
$ jlens check --schema order.schema.json -f rule.jsonata
warning[W0201]: Field "unitprice" is not in the schema; did you mean `unitPrice`?
 --> rule.jsonata:4:22
  |
4 |   $sum(items[type = 'x'].unitprice)
  |                      ^^^^^^^^^

$ jlens schema complete --schema order.schema.json "items[type = 'electronic']."
code                     field     string
color                    field     string
type                     field     string
unitPrice                field     number
quantity                 field     integer
tax                      field     number
```

`--cursor N` sets the cursor position in characters, and `--json` prints the full completion
items.

### In Rust

```rust
use jlens_schema::{FunctionDecl, LanguageService, Schema};

let schema = Schema::from_json_schema(&serde_json::from_str(&schema_text)?)?;
let service = LanguageService::new(Some(schema))
    .with_variable("approved", None)         // variables the rule is evaluated with
    .with_function(                          // and its host functions
        FunctionDecl::new("shipping", "Shipping cost of a weight to a region")
            .signature("<ns?:n>")
            .params(["weight", "region"]),
    );

let completions = service.complete(text, cursor_byte_offset);
let diagnostics = service.diagnostics(text);
let hover = service.hover(text, cursor_byte_offset);
let help = service.signature_help(text, cursor_byte_offset);
```

Declare the [host functions](crate.md#host-functions) the expressions are evaluated with:
they are completed first, and hover and signature help show their signature, parameters and
summary. A declared function replaces a built-in function of the same name, as it does during
evaluation.

All offsets and spans in the Rust API are **byte** offsets into the expression text. The
WebAssembly package converts them to and from the UTF-16 offsets that browsers and editors use.

### In the browser

The WebAssembly package exposes the same service for editors such as CodeMirror and Monaco; see
the [WebAssembly guide](wasm.md).

## What completion understands

Completion knows the *context* at the cursor, that is, which object the next field name will be
looked up in. It follows the evaluator's rules:

| You type | Suggested |
|---|---|
| `items.` or `items[...].` | Fields of an item (arrays are flattened, as during evaluation) |
| `items[` … `]` | Inside the predicate: fields of an item |
| `items{` … `}` / `items^(` … `)` | Fields of an item, in group-by keys and values and in sort keys |
| `$sum(items.(` … `))` | Fields of an item, inside a block applied to each item |
| `($i := items[0]; $i.` | Variables keep the type of what was assigned to them |
| `items@$it.` … `$it.` | Focus variables bind the items of their step |
| `$map(items, function($i){ $i.` | Callback parameters of `$map`, `$filter`, `$single`, `$sort` and `$reduce` get the item type |
| `$sort(items, …)[0].` | Functions that return their input (`$sort`, `$filter`, `$reverse`, …) keep its type |
| `` items.`ty `` | Field names that need backquotes |
| `$su` | Variables in scope, then declared host functions, then built-in functions |
| `customer.login ` (after a space) | Operators: `and`, `or`, `in` |

Each field suggestion carries:
- its type (`string`, `array<object>`, `integer | null`, …);
- its label and description;
- its presence, for optional fields;
- its allowed values or examples.

Field names that are not plain identifiers are inserted with backquotes, and hidden fields are
never suggested.

Completion works on incomplete expressions too, such as `$count(items[` or
`items[type = 'x' and `. That is the normal state of an expression while someone types it.

## Diagnostics

`LanguageService::diagnostics` returns:
- syntax errors, from the recovering parser;
- [annotation](../language/annotations.md) warnings;
- `W0201` warnings for field names the schema does not have, with a "did you mean" suggestion
  when a field name is close;
- `W0202` warnings for calls of unknown functions, with the same kind of suggestion, and `W0203`
  warnings for calls with too few or too many arguments for the function's signature. These need
  no schema. A function is known when it is built in, declared with
  `LanguageService::with_function` (or in `jlens.json`), or bound anywhere in the expression; a
  variable declared with `with_variable` may hold a function, so its calls are not checked.

Missing fields are reported only where the context is fully known. Nothing is reported after a
user-defined function, an object constructor or `$lookup`, so the warnings do not produce false
alarms. Schemas inferred from samples can miss optional fields that the samples did not contain.
Add those fields with overrides, or treat `W0201` as advice.

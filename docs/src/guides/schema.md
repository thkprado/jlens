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
| Arrays never empty | `minItems: 1` when no array seen at that place was empty |
| Numbers | Minimum and maximum |
| String formats | `date-time`, `date`, `time`, `email`, `uuid` or `uri`, when every string seen has that format |
| Enumerations | When the values come from a small set (at most 10) and repeat |
| Examples | Up to three distinct values |

The result is a standard **JSON Schema 2020-12** document. Fields that are not required carry
`x-jlens-presence`, the fraction of objects that had them, and an inferred `enum` carries
`x-jlens-enum-inferred`: it lists the values seen, not every value allowed, so the language
service completes them but never warns about another one. Remove the keyword once the list is
complete, or set the values in the overrides (below). An inferred `minItems` carries
`x-jlens-min-items-inferred`: the language service takes it as it takes an inferred `required`
(the [result's type](#the-type-of-the-result) counts on it), so review both, and remove a
`minItems` where an empty array is allowed. You can commit the file, review it,
and edit it like any JSON Schema. jlens also reads JSON Schemas written by hand or by other
tools: it follows `$ref`, by JSON Pointer, `$id` and `$anchor`, and it merges `anyOf`, `oneOf`
and `allOf`.

A schema may be recursive: a tree whose `children` are `{"$ref": "#"}`, a definition that
refers to itself, definitions that refer to each other. Completion, hover and the diagnostics
follow it as deep as the expression goes, so that `children.children.children.nane` is warned
about as `nane` is. A `$ref` that leads nowhere but to itself says nothing of its place:
nothing is completed or warned about under it. In the Rust crate such a `$ref` is kept as a
reference to a definition of the schema
([ADR 0024](../adr/0024-recursive-schemas.md)): `Schema::shape_at` follows it, and
`Schema::members` gives the shapes that one stands for. A path of the overrides that goes
through it changes the definition, and so every place that refers to it: in that tree,
`children[].label` describes the `label` of every node.

### A schema in several documents

A `$ref` may lead to another document (`"$ref": "common.json#/$defs/person"`). jlens never
fetches one: the application gives the documents, each with its URI, and the load reads only
those ([ADR 0025](../adr/0025-references-to-other-documents.md)).

```rust
use jlens_schema::SchemaLoader;

let (schema, report) = SchemaLoader::new()
    .document("https://example.com/schemas/common.json", &common)
    .load(&order)?;
for unresolved in &report.unresolved {
    eprintln!("{}: {} was not followed", unresolved.place, unresolved.uri);
}
```

A `$ref` is resolved against the `$id` of the schema it is in, or the URI its document is given
with (`load_at`), and a document is known by that URI and by each `$id` in it. With neither, a
reference is taken as written: `common.json#/$defs/person` is a place of the document given as
`common.json`.

A `$ref` the load cannot follow does not fail it. Its place is left unknown, so that nothing
is completed or warned about under it, and the rest of the schema is checked as usual. The
report beside the schema lists each one: where it is written, what it resolves to, and the
document that was not given (`report.documents()` lists them, each once). What an expression
uses marks such a place `unknown`, so a document given later shows as a change. Only a
document that is not a schema, or one nested more than 64 levels deep, fails to load.

`Schema::from_json_schema` is that load with no other document and without the report.

Every package that reads a schema takes its documents and gives the report, in the same JSON
form (`{"unresolved": [{"place", "reference", "uri", "document"}]}`, `{}` when every `$ref`
was followed):

| Where | The documents | The report |
|---|---|---|
| Rust | `SchemaLoader::document`, or `load_asking` with a function that gives them | `LoadReport` |
| [WebAssembly](wasm.md#a-schema-in-several-documents) | `new LanguageService(schema, variables, functions, { uri, documents })` | `schemaReport(schema, { uri, documents })` |
| [Java](java.md#a-schema-in-several-documents) | `Schema.loader().document(uri, text).load(text)` | `schema.unresolved()`, `schema.missingDocuments()` |
| CLI and language server | the files beside the schema's | a warning for each document not found |

The CLI and the language server read a document from a file, and never from the network. The
file is the one at the same relative path from the schema's file as between the two URIs:
beside `schemas/order.json`, `"$ref": "common.json"` is `schemas/common.json` and
`"$ref": "../shared/money.json"` is `shared/money.json`, whether the schema has an `$id` (the
path between `https://example.com/schemas/order.json` and
`https://example.com/schemas/common.json` is `common.json`) or not. A document on another
host, or one with no file there, is not read. A variable's schema in `jlens.json` refers to
files from the folder of `jlens.json`. For each document that was not found, and each `$ref`
to a place its document does not have, `jlens check`, `jlens schema uses` and
`jlens schema complete` print a `warning:` line on standard error, and the language server
shows a message; the exit code does not change, and `--deny-warnings` counts the warnings of
expressions only. A file that is there and is not JSON fails the command, as a schema that
is not JSON does.

In the Rust crate, `SchemaLoader::load_asking` is that loop: it asks a function for each
document the load misses, with its URI and its relative path from the schema, then for the
documents those refer to, each URI once and no more than 256.

What JSON Schema has no keyword for travels in jlens's own `x-jlens-*` keywords, which other
tools ignore. jlens writes them and reads them back, so they can also be written by hand:

| Keyword | Meaning |
|---|---|
| `x-jlens-presence` | On a field that is not required: the fraction of objects that had it, from 0 to 1 |
| `x-jlens-enum-inferred` | The `enum` lists the values seen in the samples, not every value allowed: completion offers them, but a value outside them is not warned about |
| `x-jlens-min-items-inferred` | The `minItems` comes from the samples: no array seen there was empty |
| `x-jlens-domain` | An [open domain](#open-domains): an identifier naming a list of values only the application can fetch |
| `x-jlens-hidden` | `true` hides the field from completion, like `hidden` in the overrides (below); it is still known, so it is not reported as unknown, and hover shows it |
| `x-jlens-samples` | At the root: the number of sample documents the schema was inferred from. For information only: nothing depends on it |

A field's `label` is written as the standard `title`, and its `description` as `description`.

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
| `domain` | An open domain: values the application provides (see [Open domains](#open-domains)) |
| `examples` | Example values |
| `format` | A format name |
| `type` | Replaces the inferred types: one name such as `"integer"`, or a list such as `["string", "null"]`, as in JSON Schema. The names are JSON Schema's: `null`, `boolean`, `integer` (a number without a fraction), `number`, `string`, `array` and `object` |
| `hidden` | Never suggested by completion |

```text
$ jlens schema infer samples/*.json --overrides overrides.json -o order.schema.json
```

A path that does not exist yet is created, so overrides can also declare fields that are absent
from the samples. A misspelt path does the same, so `jlens schema infer` warns about each such
path, naming the existing path it most likely meant:

```text
warning: override path items.type is not in the samples; declared (did you mean items[].type?)
```

In Rust, `Schema::missing_override_paths` lists them before `apply_overrides` creates them
(`missingOverridePaths` in the Wasm package).

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
WebAssembly package converts them to and from the UTF-16 offsets that browsers and editors use;
an application that builds the service into a module of its own converts them with
[`jlens_core::Offsets`](crate.md#tools-for-editors).

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
| `items.%.`, `items[%.` … `]`, `items.code.%.` | The parent: fields of the object the step that `%` refers to is evaluated against, the order for an item of `items` |
| `` items.`ty `` | Field names that need backquotes |
| `$su` | Variables in scope, then declared host functions, then built-in functions |
| `customer.login ` (after a space) | Operators: `and`, `or`, `in` |
| `type = `, `type != `, `type in [`, `type in ["toy", ` | The values of `type`'s enumeration first, then fields, variables and functions |
| `type = "ap` | The string values of `type`'s enumeration that start with `ap` |

Each field suggestion carries:
- its type (`string`, `array<object>`, `integer | null`, …);
- its label and description;
- its presence, for optional fields;
- its allowed values or examples.

Field names that are not plain identifiers are inserted with backquotes, and hidden fields are
never suggested.

Values (kind `value`) come from the enumeration of the left-hand side of `=`, `!=` or `in`: a
schema's `enum` or `const`, one set in the overrides, or one inferred from samples. `examples`
are not offered, as they are not every value. A value is inserted as a JSONata literal, a string
quoted and escaped. Inside quotes the whole literal is replaced, and the string keeps single
quotes when it can (JSONata has no `\'` escape). In an array after `in`, the values already
listed are left out.

### Open domains

An `enum` is a closed list, written with the schema. Some fields take their values from a list
only the application can fetch, often a large one, such as customers or products. Such a field
names its *domain*, an identifier only the application understands: `x-jlens-domain` in the
JSON Schema, or `domain` in the overrides:

```json
{ "customer.login": { "domain": "customers" } }
```

At the positions where values are completed, the completions then carry, beside their items, a
value request (`Completions::value_request`): the domain, the field's path in the schema
(`customer.login`; none for a variable), the prefix typed so far, the range a value replaces and
the quote it is written with. The service stays synchronous: the application fetches the values
that match the prefix, caches them as it sees fit, and writes each as a JSONata literal with
`ValueRequest::literal` (`valueLiteral` in the Wasm package). Values the field also declares in an
`enum` are still completed as items. A value outside an open domain is never warned about, and
hover names the domain.

The values of one domain often depend on another value: the subgroups of a group, the states
of a country. The request carries the fields already fixed beside the value, its *conditions*:
each field compared with a literal by `=`, or with an array of literals by `in`, that is joined
to the value's comparison by `and`, in the same predicate or, outside any, in the whole
expression. In `items[categories[group = 'Electronics' and subgroup = '|`, the request for
`subgroup` has the condition `items[].categories[].group` = `["Electronics"]`, read from the
expression as it is typed, unfinished. A comparison on one side of `or`, by `!=`, with something
that is not a literal, inside a call such as `$not(...)` or in another predicate is not a
condition. The request also says whether the value is an element of the array of `in [...]`
(`in_array`), where the application may insert several values, separated by commas, and which
literals that array already holds (`listed`), so that the application does not offer them again,
as the service does not complete a declared value twice.

Completion works on incomplete expressions too, such as `$count(items[` or
`items[type = 'x' and `. That is the normal state of an expression while someone types it.

## Diagnostics

`LanguageService::diagnostics` returns:
- syntax errors, from the recovering parser;
- [annotation](../language/annotations.md) warnings;
- `W0201` warnings for field names the schema does not have, with a "did you mean" suggestion
  when a field name is close. Under the parent operator, `items.%.custmer`, the field is looked
  up in the parent; where the typing does not tell the parent, as for a step inside parentheses
  (`customer.(login).%`), nothing is reported;
- `W0202` warnings for calls of unknown functions, with the same kind of suggestion, and `W0203`
  warnings for calls with too few or too many arguments for the function's signature. These need
  no schema. A function is known when it is built in, declared with
  `LanguageService::with_function` (or in `jlens.json`), or bound anywhere in the expression; a
  variable declared with `with_variable` may hold a function, so its calls are not checked;
- `W0205` warnings for arguments whose type the function's signature cannot accept, such as
  `$sum(items.name)` when `name` is a string. Only types that are known count: literals, the
  results of operators, and fields of the schema. A field that is absent passes any signature;
  the warning means that the call fails whenever the field is present;
- `W0204` warnings for variables that are not declared, with a suggestion, when at least one
  variable is declared with `with_variable` (or in `jlens.json`): the declarations are then
  taken to be complete. Names bound in the expression, and functions, are known;
- `W0206` warnings for a literal compared with a field, by `=`, `!=` or `in`, that is not one of
  the field's declared values, such as `items[type = "aparel"]`: the comparison is always false.
  A string gets the nearest value as a suggestion (`did you mean "apparel"?`). Only an `enum` or
  `const` of the JSON Schema, or an `enum` set in the overrides, counts: one inferred from samples
  (`x-jlens-enum-inferred`) is a guess, and is never warned about, and neither is a field with an
  [open domain](#open-domains). Values that are not literals, such as `type = color`, are not
  checked;
- warnings for the operands of an operator whose types it cannot take, so that the expression
  fails whenever they are present, or a comparison is always false. With `total` a string,
  `total > 1000` gets `W0208`: strings and numbers are not compared with each other (`T2009`
  when it runs). A side of `<`, `<=`, `>` or `>=` that is a boolean, null, an object or an array
  of them has no order, `W0207` (`T2010`), and a side of `+`, `-`, `*`, `/` or `%` that is not a
  number gets `W0209` (`T2001`, `T2002`), as does the operand of a negation, `-x`, with `W0211`
  (`D1002`). With `active` a string, `active = true` gets `W0210`:
  a field compared by `=`, `!=` or `in` with a literal of another type is never equal to it. A
  field with a declared enumeration gets `W0206` there instead, and two literals compared are
  left alone. As for `W0205`, only types that are known count, an absent field passes, and an
  array stands for its items: `scores > 1` gets none of these when `scores` is an array of
  numbers, but `W0215`, below;
- `W0215` for an operand that may be several values. A path over several items gives a
  [sequence](#the-type-of-the-result): `items.qty > 1` is true for an order of one item and fails
  for two (`T2010`), `items.qty + 1` fails too (`T2001`), and `items.qty = 2` is false for two,
  as `=` compares the array. A field that holds an array is an array even of one item: `tags >
  'a'` fails, and `tags = 'a'` is false. The warning names the step that makes the sequence (the
  step over an array, a filter that may keep several items, `*`, `**`, `$keys`) and says what to
  write instead: an aggregate, `$max(items.qty) > 1` or `$sum(items.qty) + 1`; a filter that tests
  each item, `$count(items[qty > 1]) > 0`; `in` to find a value among them, `2 in items.qty`. A
  number filter keeps one item, so `items[0].qty > 1` gets nothing, nor does a filter of an object,
  `order[qty > 1]`. `=` and `!=` are warned about only where the other side is one value;
- with the type the result must have declared, `W0212`, `W0213` and `W0214` for a result that
  never has it, may have another, or may be undefined ([below](#the-type-of-the-result)).

The typing does not read the guards of an expression: in `$type(x) = 'number' and x > 10`, with
`x` a string in the schema, `x > 10` is warned about, though the left side, always false, keeps it
from running.

Missing fields are reported only where the context is fully known. Nothing is reported after a
user-defined function, an object constructor or `$lookup`, so the warnings do not produce false
alarms. Schemas inferred from samples can miss optional fields that the samples did not contain.
Add those fields with overrides, or treat `W0201` as advice.

## The type of the result

An application often knows the type an expression must give: the conditions of a rules engine
must give a boolean. Declare it, and the diagnostics compare the result with it:

```rust
use jlens_schema::{JsonType, LanguageService};

let service = LanguageService::new(Some(schema)).with_result_type([JsonType::Boolean]);
let diagnostics = service.diagnostics("$contains(notes, 'urgent')");
// W0214: The result may be undefined, not a boolean, because of notes
```

There are three answers. The result is always one of the types, and nothing is reported. It
is never one of them, `W0212`, as `total` is a number. Or it may not be: `W0213` when it may
have another type, with the types it may have, and `W0214` when it may be undefined, pointing
at the operand that makes it so. `Number` accepts an integer.

Most conditions are proved without a document: `=`, `!=`, `in`, `and`, `or` and `$exists`
always give a boolean. An order comparison, arithmetic and most functions are undefined
where an operand is, and these may be undefined:

- a field that is not required, and what is under it: `$contains(notes, 'urgent')`;
- a filter, which may keep nothing: `items[type = 'gift'].price > 10`;
- a step over an array that may be empty, as `$sum(items.price) > 100` is when `items` is `[]`
  (`$sum` of nothing is undefined). An array with `minItems` of at least 1 is never empty, and
  inference gives `minItems: 1` to an array no sample had empty;
- a function that may give nothing, such as `$lookup`, `$max` of an empty array or `$keys` of
  an object without fields, and a condition without `else`.

A path over several items gives a **sequence**: one value alone, or several in an array.
`lines.price` is a number for an order of one line and an array for an order of two, so a
result expected to be a number gets `W0213`, pointing at `lines`; `$sum(lines.price)` is one
number. An array constructor, `[price]`, and functions such as `$sort` always give an array.

Where the typing does not know the type, nothing is reported: a function the expression
defines, an object constructor, a path that binds `@$v` or `#$i`, a variable declared without a
shape, a field without a schema, the parent of a step inside parentheses. `LanguageService::result_type` gives what it knows as a value,
with no expected type: the types the result may have (`None` where they are not known) and the
spans of the operands that may make it undefined or a sequence. `ResultType::check` compares
it with a list of types, and answers `Always`, `Never`, `Maybe` or `Unknown`.

## What an expression uses of its schema

An application may store expressions written against a schema that changes. When a new version
of the schema arrives, the diagnostics tell whether each expression still passes. They do not
tell whether something it uses changed: a field that went from a boolean to a string leaves
`active = true` valid, and always false.

`LanguageService::uses` answers that. It takes [the fields an expression
reads](crate.md#the-fields-an-expression-reads) and gives what the schema says of each:

```rust
use jlens_core::Expression;
use jlens_schema::{LanguageService, Schema};

let service = LanguageService::new(Some(schema));
let expression = Expression::compile(r#"$sum(items[type = "book"].price)"#)?;
let uses = service.uses(&expression.reads());
let json = serde_json::to_string(&uses)?;   // kept beside the expression
```

```json
{
  "input": {
    "$": {"types": ["object"]},
    "items": {"types": ["array"], "required": true},
    "items[]": {"types": ["object"]},
    "items[].price": {"whole": true, "types": ["number"], "required": false},
    "items[].type": {"whole": true, "types": ["string"], "enum": ["book", "toy"],
                     "closed": true, "required": true}
  },
  "bindings": {}
}
```

`input` holds the paths read of the input, and `bindings` those of each variable, by name;
`input` is left out when the expression does not read its input. A path is written as the
overrides write it: `$` is the value itself, `[]` steps into the items of an array, and a name
that is not a plain one is written between backquotes, as in an expression (``customer.`odd
name` ``). `Schema::shape_at` reads every path of an answer, but a name that itself holds a
backquote. Every value on the way to a field has its own path, and so do the items of an array.

What the schema says of a path:

| Member | Meaning |
|---|---|
| `types` | The types the schema gives the value, as JSON Schema names them; left out when it gives none |
| `enum` | The values it lists |
| `closed` | `true` when the enumeration is declared and has no open domain, so that a value outside it is warned about (`W0206`); left out for one inferred from samples |
| `domain` | Its [open domain](#open-domains) |
| `format` | Its string format |
| `required` | Whether the object that holds the field requires it. Left out for `$` and for the items of an array. A field is always there only when the fields on its way are required too |
| `whole` | `true` when the expression reads the value whole, not only some of its fields |
| `unknown` | `true` when the schema says nothing there: the value has no schema, or the field is not one it declares |

The paths are those of the analysis of the reads, which is an upper bound. Where it follows a
value, they are the fields the expression names, those of a predicate included. Where it does
not (`*`, `**`, `$eval`, a value given to a host function), the value is read whole, and every
path the schema has under it is listed. `$eval` can also read any variable: the answer then has
`"everyBinding": true`, and every variable declared with a schema, whole. The host functions
declared to the service are not values, and are left out of `bindings`.

The call of a function that is not built in reads the context of its call whole, as a host
function written in Rust can read it: `$describe(items[0])` at the top of an expression uses the
whole input, and every change of the schema is then a change for the expression. The host
functions of the WebAssembly, Python and Java packages receive their arguments and nothing
else, and so do those marked `HostFunction::without_input` in Rust. `LanguageService::uses_of`
takes the expression itself, and counts a call to a function declared to the service
(`with_function`) as reading its arguments alone, and the context that a `-` of its signature
may take: the call above uses `items`, whole, and nothing else.

```rust
let uses = service.uses_of(&expression);
```

Give `uses` the reads when a declared function reads its context, or when the application
knows better which functions do (`Expression::reads_with`).

Under a value read whole, a recursive schema is listed down to where it repeats: what is
under a definition is listed once, at the place nearest to the value read whole, and another
value of that definition is the last path on its way. Read whole, a tree whose `children` are
the document itself gives `$`, its fields, `children` and `children[]`. A change inside the
definition shows in the paths listed.

The answer depends on nothing else in the schema, and its paths and names are written in order:
two schemas that give an expression the same text differ in nothing it uses.

### What changed between two schemas

When the answers differ, `Uses::changes` tells where. It compares the answer kept beside the
expression with that of the new schema:

```rust
let old: Uses = serde_json::from_str(&kept)?;
let new = LanguageService::new(Some(new_schema)).uses(&expression.reads());
let changes = old.changes(&new);
if !changes.is_empty() {
    let json = serde_json::to_string(&changes)?;   // shown to who reviews the expression
}
```

With `price` now an integer or a string, `type` no longer required and with one more value, and
a field `weight` added to the items, an expression that reads the items whole
(`items[type = "book"]`) gets:

```json
{
  "input": {
    "added": {
      "items[].weight": {"types": ["number"], "required": false}
    },
    "changed": {
      "items[].price": {"types": {"old": ["number"], "new": ["integer", "string"]}},
      "items[].type": {
        "enum": {"old": ["book", "toy"], "new": ["book", "game", "toy"], "added": ["game"]},
        "required": {"old": true, "new": false}
      }
    }
  }
}
```

`input` and `bindings`, by variable, hold the changes as the answers hold the paths, and each is
left out when nothing changed there: `{}` means that nothing the expression uses changed.

| Member | Meaning |
|---|---|
| `added` | The paths that only the new answer has, with what the new schema says of each: a field added under a value the expression reads whole |
| `removed` | The paths that only the old answer has, with what the old schema said: a field removed under a value read whole |
| `changed` | The paths of both answers of which the schemas do not say the same. Each has the members that changed, under their names in the answer, as `{"old": …, "new": …}`; a side on which the member was left out is left out here too |

A field that the expression names stays in the answer when the new schema drops it. It is then
under `changed`, with `"unknown": {"old": false, "new": true}` and what the old schema said of
it. A path is compared with the same path, so a field that was renamed is one path that became
unknown, or was removed, and another that was added.

The types and the values of an enumeration are compared as sets: their order is not a change.
An enumeration that changed also has `added` and `removed`, the values that only one side
lists, so that an application can tell a value that was added, which leaves the expression's
comparisons as they were, from one that was removed.

### In the CLI and the other packages

Every place that has the schema gives the two answers, as the same JSON:

| Where | What an expression uses | What changed |
|---|---|---|
| Rust | `LanguageService::uses(&reads)`, `uses_of(&expression)` | `Uses::changes(&new)` |
| CLI | `jlens schema uses <EXPR>` | `jlens schema changes OLD NEW` |
| [WebAssembly](wasm.md#what-an-expression-uses-of-its-schema) | `service.uses(expression.reads(bindings))` | `usesChanges(old, current)` |
| [Java](java.md#what-an-expression-uses-of-its-schema) | `service.uses(expression)` | `uses.changes(current)` |

The comparison takes the two answers and nothing else, so the answer kept beside an expression
is compared when its schema is gone. In Rust, `Uses::from_json` reads an answer kept as JSON; it
refuses what is not in the form above, where the derived `Deserialize` would take an array for
an object.

`jlens schema uses` prints the answer of an expression, given as an argument or with `-f`. It
takes its schema, variables and host functions as [`jlens check`](cli.md#jlens-check) does:
from `--schema`, and from the `jlens.json` of the current directory or of `--config`, which
`--path` applies to an expression given as an argument. A host function the configuration
declares reads its arguments alone, as `uses_of` takes it. `jlens schema changes` compares two
files it wrote:

```sh
jlens schema uses -f rules/books.jsonata --schema order-v1.schema.json > books.uses.json
jlens schema uses -f rules/books.jsonata --schema order-v2.schema.json > books.new.json
jlens schema changes books.uses.json books.new.json
```

```text
input
  added    items[].weight: number, optional
  changed  items[].price: types number -> integer | string
  changed  items[].type: enum added "game"; required true -> false
```

It prints `no change` when nothing the expression uses changed, and with `--json` the JSON form,
`{}` then. Both commands exit with 0 whether something changed or not; a file that is not an
answer is a usage problem, 2.

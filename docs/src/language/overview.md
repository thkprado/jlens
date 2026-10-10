# Language overview

JSONata is a query and transformation language for JSON. An expression selects values from a
document with location paths (`Account.Order.Product`), combines them with operators and
functions (`$sum(Product.(Price * Quantity))`), and shapes the output with ordinary JSON syntax
(`{"total": ...}`). It also has variables, functions and conditionals, so rules of any
complexity can be written in it.

jlens implements JSONata 2.2 and is checked against the official test suite. Where it differs
from the reference implementation, jsonata-js, [Compatibility](compatibility.md) says so.

> This reference is adapted from the [JSONata documentation](https://docs.jsonata.org)
> (© IBM Corp. and JSONata contributors, MIT license), reorganized and extended with
> jlens-specific notes. Every example in it is run against jlens by `cargo xtask doc-check`.

## Reading the examples

Examples show an expression followed by its result after `=>`:

```jsonata-example
1 + 2
=> 3
```

When an example needs an input document, its first line names it. Most examples use one of
three sample documents, listed [at the end of this page](#sample-documents):

```jsonata-example
input: address.json
Address.City
=> "Winchester"
```

A result of `undefined` means the expression produced nothing: JSONata's empty sequence. The
command line prints nothing for it, and the APIs return `None` or `undefined`.

## Types

JSONata values have the JSON types plus one more:

| Type | Examples |
|---|---|
| string | `"hello"`, `'hello'` |
| number | `42`, `-3.5`, `1e6` (always a 64-bit float) |
| Boolean | `true`, `false` |
| null | `null` |
| array | `[1, "two", null]` |
| object | `{"name": "Fred", "age": 28}` |
| function | `$uppercase`, `function($x) { $x * 2 }` |

**JSONata is a superset of JSON.** Every JSON document is a valid expression that evaluates to
itself. A JSON template is therefore a good start for a transformation: replace its values with
expressions.

```jsonata-example
input: address.json
{"name": FirstName & " " & Surname, "adult": Age >= 18, "phones": $count(Phone)}
=> {"name": "Fred Smith", "adult": true, "phones": 4}
```

## Sequences

A path can match zero, one or many values, so JSONata expressions produce *sequences*. The
rules for sequences shape every result:

1. **An empty sequence is nothing.** It is not `null`, and it does not appear in the output: an
   object field whose value is an empty sequence is left out.
2. **A sequence of one value is that value.** No array surrounds it.
3. **A sequence of several values is output as an array.**
4. **Sequences are flattened.** Values of nested sequences are pulled up into the outer one.
   Arrays from the input, or built with `[...]`, keep their structure.

```jsonata-example
input: address.json
Phone[type = 'mobile'].number
=> "077 7700 1234"
```

```jsonata-example
input: address.json
Phone[type = 'office'].number
=> ["01962 001234", "01962 001235"]
```

```jsonata-example
input: address.json
{"fax": Phone[type = 'fax'].number, "name": FirstName}
=> {"name": "Fred"}
```

The second rule makes a single value and a one-element array interchangeable, which keeps
expressions composable. When a consistent array is needed, the
[`[]` modifier](paths.md#always-getting-an-array) keeps singleton results as arrays.

## Comments

`/* ... */` comments can appear anywhere whitespace can:

```jsonata-example
input: address.json
/* the person's full name */
FirstName & " " & Surname
=> "Fred Smith"
```

Comments that start with `/**` are documentation comments. jlens reads `@label`, `@message`
and `@id` tags in them to describe checks in [audits](../guides/audit.md); see
[Audit annotations](annotations.md). Other JSONata implementations treat them as ordinary
comments.

## The rest of this reference

- [Paths](paths.md): navigating objects and arrays, predicates, wildcards, and the path
  stages (`#`, `@`, `%`).
- [Building results](construction.md): array and object constructors.
- [Operators](operators.md): arithmetic, comparison, Boolean, string, conditional and chaining
  operators, and their precedence.
- [Sorting, grouping and aggregation](grouping.md).
- [Programming](programming.md): variables, blocks, functions, recursion, closures, partial
  application, function composition and the transform operator.
- [Regular expressions](regex.md) and [Dates and times](date-time.md).
- [The function library](functions.md), with a page per category.
- [Errors](errors.md): every error code.

## Sample documents

`address.json`:

```json
{{#include ../../examples/address.json}}
```

`invoice.json`:

```json
{{#include ../../examples/invoice.json}}
```

`library.json`:

```json
{{#include ../../examples/library.json}}
```

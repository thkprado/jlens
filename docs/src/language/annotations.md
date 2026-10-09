# Audit annotations

An annotation attaches business metadata to part of an expression: a label, a message and an
identifier. jlens uses annotations to make audit trees and explanations readable. Annotations are
written as JSONata comments, so an annotated expression is still valid JSONata and evaluates
identically in any other implementation.

```jsonata
/** @label "Authorized customer" @message "The customer login must be on the approved list." */
customer.login in $approved
and
/** @label "Minimum total" @id "RULE-042" */
$sum(items.(unitPrice * quantity)) > 50000
```

## Syntax

An annotation is a block comment that starts with `/**`. The empty comment `/**/` and ordinary
`/* ... */` comments are not annotations.

| Element | Meaning |
|---|---|
| `@label "…"` | Short name shown in audit trees and explanations |
| `@message "…"` | Longer explanation of what the expression checks |
| `@id "…"` | Stable identifier, for referring to this check from other systems |
| text before the first tag | Free-form description |

Tag values are JSON string literals, so `\"`, `\n` and `é` escapes work. Leading `*`
characters on continuation lines are ignored, so multi-line comments can be written JSDoc-style:

```jsonata
/**
 * Customers must be registered partners.
 * @label "Authorized customer"
 * @id "IMP-001"
 */
customer.login in $approved
```

When several annotations precede the same expression, they are merged and later tags win.

## What an annotation covers

An annotation covers the **operand that follows it**. It extends to the right as far as the
surrounding operators allow, but never past `and` or `or`, nor past operators that bind more
loosely: `?` (the conditional), `:=` and `..`. In practice:

| Expression | Annotated part |
|---|---|
| `/** A */ x.y in $list and z` | `x.y in $list` |
| `a and /** A */ b > 1 or c` | `b > 1` |
| `/** A */ $total > 100 ? "big" : "small"` | `$total > 100` (the condition) |
| `a - /** A */ b + c` | `b` (the operand of `-`) |
| `$f(/** A */ items.price, 2)` | `items.price` (the argument) |
| `/** A */ (x or y)` | the whole parenthesised block |
| `$x := /** A */ $sum(values)` | `$sum(values)` (the bound value) |

Annotations never change how an expression is parsed. The syntax tree of an annotated
expression is the same as the tree of the expression with its comments removed; only the
annotation metadata is added to one node.

To label a whole `and`/`or` chain, wrap it in parentheses, as the rule in
`fixtures/shipping-rule.jsonata` does:

```jsonata
/** @label "Order category check" */
(
  /** @label "Electronics group" */ ('electronic' in items.type and $electronics > $total * 0.3)
  or
  /** @label "Blue items group" */ ('blue' in items.color and $blueTax < $averageTax)
)
```

## Warnings

Annotation problems are warnings: they are reported, but they never stop compilation or
evaluation.

| Code | When |
|---|---|
| `W0101` | The annotation precedes something that is not an expression, such as an operator, a closing bracket, or the end of the input |
| `W0102` | Unknown tag, such as `@severity` |
| `W0103` | A tag value is not a quoted JSON string; the rest of the line is used as the value |

Run `jlens check` to see them:

```text
$ jlens check '/** @label "Total" @severity "high" */ $sum(items.price) > 100'
warning[W0102]: Unknown annotation tag @severity; expected @label, @message or @id
 --> <expr>:1:20
  |
1 | /** @label "Total" @severity "high" */ $sum(items.price) > 100
  |                    ^^^^^^^^^
<expr>: 0 error(s), 1 warning(s)
```

## In the API

After parsing, the annotation is available on the annotated node as `Node::annotation`. It also
appears in the JSON form of the syntax tree (`jlens ast`):

```json
{ "type": "binary", "op": "in", "annotation": { "label": "Authorized customer", "span": { … } } }
```

The [audit guide](../guides/audit.md) explains how annotations appear in audit trees.

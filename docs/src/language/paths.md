# Paths

A *location path* selects values from the input document. Field names separated by `.`
navigate into nested objects, and every array along the way is searched item by item.

## Navigating objects

```jsonata-example
input: address.json
Surname
=> "Smith"
```

```jsonata-example
input: address.json
Address.City
=> "Winchester"
```

A path that matches `null` returns it. A path that matches nothing returns nothing (an empty
sequence), never an error: missing data is normal.

```jsonata-example
input: address.json
Other.Misc
=> null
```

```jsonata-example
input: address.json
Other.Nothing
=> undefined
```

Field names that are not plain identifiers go between backquotes:

```jsonata-example
input: address.json
Other.`Over 18 ?`
=> true
```

## Navigating arrays

When a step selects an array, the following steps apply to each of its items, and the results
are collected:

```jsonata-example
input: address.json
Phone.number
=> ["0203 544 1234", "01962 001234", "01962 001235", "077 7700 1234"]
```

A number in square brackets selects an item by position, from 0. Negative positions count from
the end, and non-integer positions are rounded down. A position past the end selects nothing.

```jsonata-example
input: address.json
Phone[0]
=> {"type": "home", "number": "0203 544 1234"}
```

```jsonata-example
input: address.json
Phone[-1].number
=> "077 7700 1234"
```

```jsonata-example
input: address.json
Phone[8]
=> undefined
```

An array of positions selects several items:

```jsonata-example
input: address.json
Phone[[0, 2]].number
=> ["0203 544 1234", "01962 001235"]
```

```jsonata-example
input: address.json
Phone[[0..1]].number
=> ["0203 544 1234", "01962 001234"]
```

The index applies to the step it follows, not to the whole path so far. `Phone.number[0]`
takes the first number of *each* phone. Parentheses apply the index to the whole result:

```jsonata-example
input: address.json
Phone.number[0]
=> ["0203 544 1234", "01962 001234", "01962 001235", "077 7700 1234"]
```

```jsonata-example
input: address.json
(Phone.number)[0]
=> "0203 544 1234"
```

## Predicates

Square brackets around any other expression *filter* the items: an item is kept when the
expression, evaluated with the item as context, is true. Values that are not Booleans are
converted as by [`$boolean`](functions/boolean.md#boolean).

```jsonata-example
input: address.json
Phone[type = 'mobile'].number
=> "077 7700 1234"
```

```jsonata-example
input: invoice.json
Account.Order.Product[Price > 30 and Quantity > 1].`Product Name`
=> ["Bowler Hat", "Bowler Hat"]
```

```jsonata-example
input: library.json
library.books["Aho" in authors].title
=> ["The AWK Programming Language", "Compilers: Principles, Techniques, and Tools"]
```

## Always getting an array

Because a single value and a one-item sequence are the same (see
[Sequences](overview.md#sequences)), a path can return a value or an array depending on the
data. Empty square brackets `[]` on any step keep the result an array:

```jsonata-example
input: address.json
Address[].City
=> ["Winchester"]
```

```jsonata-example
input: address.json
Phone[type = 'home'].number[]
=> ["0203 544 1234"]
```

## The context: `$` and `$$`

`$` is the *context value*: the item a step is evaluated against. At the start of an expression
it is the whole input document, which is how a top-level array is indexed. `$$` is always the
root of the input, wherever the evaluation is.

```jsonata-example
input: [{"ref": [1, 2]}, {"ref": [3, 4]}]
$[0].ref
=> [1, 2]
```

```jsonata-example
input: [{"ref": [1, 2]}, {"ref": [3, 4]}]
$.ref
=> [1, 2, 3, 4]
```

The nested arrays of the last example were flattened into one sequence. To keep the structure,
use an [array constructor](construction.md#arrays): `$.[ref]`.

```jsonata-example
input: address.json
Phone[type = 'mobile'].($$.FirstName & ": " & number)
=> "Fred: 077 7700 1234"
```

## Wildcards

`*` selects the values of all the fields of an object:

```jsonata-example
input: address.json
Address.*
=> ["Hursley Park", "Winchester", "SO21 2JN"]
```

```jsonata-example
input: address.json
*.Postcode
=> "SO21 2JN"
```

`**` selects descendants at any depth:

```jsonata-example
input: address.json
**.Postcode
=> ["SO21 2JN", "E1 6RF"]
```

## Path stages

A path is a pipeline of stages. Each stage takes the sequence from the previous one:

| Stage | Syntax | Action |
|---|---|---|
| Map | *seq* `.` *expr* | Evaluates *expr* for each item, and flattens the results |
| Filter | *seq* `[` *expr* `]` | Keeps the items for which *expr* is true (or the positions *expr* gives) |
| Sort | *seq* `^(` *keys* `)` | Sorts the items; see [Sorting](grouping.md#sorting) |
| Index | *seq* `#` *$var* | Binds the position of each item to *$var* |
| Join | *seq* `@` *$var* | Binds each item to *$var*, keeping the previous context |
| Group | *seq* `{` *key*`:` *value*, … `}` | Groups the items into one object; see [Grouping](grouping.md#grouping) |

A filter binds more tightly than a map, so `books.authors[0]` is the first author of *each* book.
Parentheses change the grouping, as `(books.authors)[0]` does. Parentheses also end the scope of
variables bound inside them.

### Mapping with expressions

Any expression can be a step, not just a field name. Its context is each item in turn:

```jsonata-example
input: invoice.json
Account.Order.Product.(Price * Quantity)
=> [68.9, 21.67, 137.8, 107.99]
```

```jsonata-example
input: invoice.json
Account.Order.OrderID.$uppercase()
=> ["ORDER103", "ORDER104"]
```

### Positions: `#`

`#$i` binds the position of each item, counted before later filters, to `$i`:

```jsonata-example
input: library.json
library.books#$i["Kernighan" in authors].{"title": title, "index": $i}
=> [
  {"title": "The C Programming Language", "index": 1},
  {"title": "The AWK Programming Language", "index": 2}
]
```

### Joins: `@`

`@$v` binds each item to `$v` and keeps the context where it was, so the next step navigates
from the same place. That cross-references arrays like a join in a database:

```jsonata-example
input: library.json
library.loans@$l.books@$b[$l.isbn = $b.isbn].{"title": $b.title, "customer": $l.customer}
=> [
  {"title": "Structure and Interpretation of Computer Programs", "customer": "10001"},
  {"title": "Compilers: Principles, Techniques, and Tools", "customer": "10003"},
  {"title": "Structure and Interpretation of Computer Programs", "customer": "10003"}
]
```

### Parents: `%`

`%` selects the parent of the context value: the object whose field holds it. `%.%` is the
grandparent. The parent is worked out from the path when the expression is compiled, so `%`
only works in paths that reach the value through its parent (otherwise: error `S0217`).

```jsonata-example
input: invoice.json
Account.Order.Product.{"product": `Product Name`, "order": %.OrderID, "account": %.%.`Account Name`}
=> [
  {"product": "Bowler Hat", "order": "order103", "account": "Firefly"},
  {"product": "Trilby hat", "order": "order103", "account": "Firefly"},
  {"product": "Bowler Hat", "order": "order104", "account": "Firefly"},
  {"product": "Cloak", "order": "order104", "account": "Firefly"}
]
```

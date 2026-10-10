# Sorting, grouping and aggregation

## Sorting

The order-by stage `^(...)` sorts the items of a path. Sort keys are expressions evaluated
against each item, and they must give numbers or strings. Strings sort by Unicode code point. A
key prefixed with `>` sorts in descending order; `<`, the default, sorts in ascending order.
Further keys, separated by commas, break ties. The sort is stable.

```jsonata-example
input: invoice.json
Account.Order.Product^(Price).`Product Name`
=> ["Trilby hat", "Bowler Hat", "Bowler Hat", "Cloak"]
```

```jsonata-example
input: invoice.json
Account.Order.Product^(>Price, <Quantity).{"name": `Product Name`, "qty": Quantity}
=> [
  {"name": "Cloak", "qty": 1},
  {"name": "Bowler Hat", "qty": 2},
  {"name": "Bowler Hat", "qty": 4},
  {"name": "Trilby hat", "qty": 1}
]
```

```jsonata-example
input: invoice.json
Account.Order.Product^(Price * Quantity).(Price * Quantity)
=> [21.67, 68.9, 107.99, 137.8]
```

[`$sort`](functions/array.md#sort) sorts an array with a comparison function, for orders that
keys cannot express.

## Grouping

An object constructor directly after a path (without `.`) groups the items: it evaluates the
key for each item, and items with the same key fall into the same group. Then it evaluates the
value once per group, with the group's items as context. Aggregations therefore work per group:

```jsonata-example
input: invoice.json
Account.Order.Product{`Product Name`: Price}
=> {"Bowler Hat": [34.45, 34.45], "Trilby hat": 21.67, "Cloak": 107.99}
```

```jsonata-example
input: invoice.json
Account.Order.Product{`Product Name`: $round($sum($.(Price * Quantity)), 2)}
=> {"Bowler Hat": 206.7, "Trilby hat": 21.67, "Cloak": 107.99}
```

The value expression runs against the whole group. A path in it therefore collects values
from all the group's items. To build one object per item, map over the group with `$.`:

```jsonata-example
input: invoice.json
Account.Order.Product{`Product Name`: {"price": Price, "qty": Quantity}}
=> {
  "Bowler Hat": {"price": [34.45, 34.45], "qty": [2, 4]},
  "Trilby hat": {"price": 21.67, "qty": 1},
  "Cloak": {"price": 107.99, "qty": 1}
}
```

```jsonata-example
input: invoice.json
Account.Order.Product{`Product Name`: $.{"price": Price, "qty": Quantity}}
=> {
  "Bowler Hat": [{"price": 34.45, "qty": 2}, {"price": 34.45, "qty": 4}],
  "Trilby hat": {"price": 21.67, "qty": 1},
  "Cloak": {"price": 107.99, "qty": 1}
}
```

Keys must be strings (error `T1003`). Several key/value pairs can group the same items in
different ways, and a group that evaluates to nothing is left out.

```jsonata-example
input: library.json
library.books{
  "cheap": $count($[price < 30]),
  "expensive": $count($[price >= 30]),
  "unpriced": $count($[$not($exists(price))])
}
=> {"cheap": 1, "expensive": 2, "unpriced": 1}
```

## Aggregation

Aggregation functions reduce an array to one value:
- [`$sum`](functions/aggregation.md#sum);
- [`$count`](functions/aggregation.md#count);
- [`$max`](functions/aggregation.md#max), [`$min`](functions/aggregation.md#min);
- [`$average`](functions/aggregation.md#average);
- [`$join`](functions/string.md#join) for strings;
- [`$reduce`](functions/higher-order.md#reduce) for anything else.

```jsonata-example
input: invoice.json
$sum(Account.Order.Product.(Price * Quantity))
=> 336.36
```

```jsonata-example
input: invoice.json
{
  "orders": $count(Account.Order),
  "dearest": $max(Account.Order.Product.Price),
  "average quantity": $average(Account.Order.Product.Quantity)
}
=> {"orders": 2, "dearest": 107.99, "average quantity": 2}
```

```jsonata-example
input: invoice.json
$join(Account.Order.Product.`Product Name`, ", ")
=> "Bowler Hat, Trilby hat, Bowler Hat, Cloak"
```

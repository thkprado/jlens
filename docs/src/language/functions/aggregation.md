# Aggregation functions

Aggregation functions reduce an array to a single value. A single value counts as an array of
one item, and an empty array gives `0` for `$sum` and `$count`, and nothing for the others. The
items of `$sum`, `$max`, `$min` and `$average` must be numbers (error `T0412`).

## `$sum`

`$sum(array)` adds the numbers of `array`.

```jsonata-example
$sum([5, 1, 3, 7, 4])
=> 20
```

```jsonata-example
input: invoice.json
$sum(Account.Order.Product.Quantity)
=> 8
```

## `$count`

`$count(array)` returns the number of items. It accepts any values.

```jsonata-example
[$count([1, 2, 3, 1]), $count("hello"), $count([])]
=> [4, 1, 0]
```

```jsonata-example
input: address.json
$count(Phone[type = 'office'])
=> 2
```

## `$max`

`$max(array)` returns the largest number.

```jsonata-example
$max([5, 1, 3, 7, 4])
=> 7
```

## `$min`

`$min(array)` returns the smallest number.

```jsonata-example
$min([5, 1, 3, 7, 4])
=> 1
```

```jsonata-example
$min([])
=> undefined
```

## `$average`

`$average(array)` returns the mean of the numbers.

```jsonata-example
$average([5, 1, 3, 7, 4])
=> 4
```

---

Adapted from the JSONata documentation (© IBM Corp. and JSONata contributors, MIT license).

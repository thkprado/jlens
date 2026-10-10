# Array functions

A single value passed where an array is expected counts as an array of one item. Results that
are arrays are proper arrays: they keep their structure even with one item.

## `$append`

`$append(array1, array2)` returns the items of both arrays, in order. A value that is not an
array is appended as one item.

```jsonata-example
$append([1, 2, 3], [4, 5, 6])
=> [1, 2, 3, 4, 5, 6]
```

```jsonata-example
$append("Hello", "World")
=> ["Hello", "World"]
```

## `$reverse`

`$reverse(array)` returns the items in reverse order.

```jsonata-example
[1..5] ~> $reverse()
=> [5, 4, 3, 2, 1]
```

## `$sort`

`$sort(array, comparator?)` returns the items in order. Without a comparator, the items must be
all numbers or all strings (error `D3070`), and they sort in ascending order.

`comparator($a, $b)` returns `true` when `$a` must come *after* `$b`. The sort is stable: items
the comparator considers equal keep their order.

```jsonata-example
$sort([5, 1, 3, 7, 4])
=> [1, 3, 4, 5, 7]
```

```jsonata-example
input: invoice.json
$sort(Account.Order.Product, function($l, $r) {
  $l.Description.Weight > $r.Description.Weight
}).`Product Name`
=> ["Trilby hat", "Bowler Hat", "Bowler Hat", "Cloak"]
```

For sorting by keys, the [order-by](../grouping.md#sorting) stage `^(...)` is shorter.

## `$distinct`

`$distinct(array)` removes duplicates, keeping the first occurrence of each value. Values are
compared as by `=`, deeply for arrays and objects.

```jsonata-example
$distinct([1, 2, 3, 3, 4, 3, 5])
=> [1, 2, 3, 4, 5]
```

```jsonata-example
input: invoice.json
$distinct(Account.Order.Product.Description.Colour)
=> ["Purple", "Orange", "Black"]
```

## `$shuffle`

`$shuffle(array)` returns the items in random order.

```jsonata-example
$count($shuffle([1..9]))
=> 9
```

## `$zip`

`$zip(array1, array2, ...)` groups the items at each position of its arguments into arrays. The
result is as long as the shortest argument.

```jsonata-example
$zip([1, 2, 3], [4, 5, 6])
=> [[1, 4], [2, 5], [3, 6]]
```

```jsonata-example
$zip([1, 2, 3], [4, 5], [7, 8, 9])
=> [[1, 4, 7], [2, 5, 8]]
```

`$count` is described with the [aggregation functions](aggregation.md#count).

---

Adapted from the JSONata documentation (MIT license).

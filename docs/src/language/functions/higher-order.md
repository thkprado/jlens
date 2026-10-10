# Higher-order functions

These functions take a function as an argument. The function receives an item and, if it
declares more parameters, the item's position and the whole array (or, for `$sift`, the field
name and the whole object).

## `$map`

`$map(array, function)` returns the results of `function($value, $index, $array)` for each item.
Results follow the [sequence](../overview.md#sequences) rules: one result is returned as
itself, and none as nothing.

```jsonata-example
$map([1, 2, 3], function($v) { $v * 2 })
=> [2, 4, 6]
```

```jsonata-example
$map([1..5], $string)
=> ["1", "2", "3", "4", "5"]
```

```jsonata-example
input: address.json
$map(Email.address, function($v, $i, $a) { "Item " & ($i + 1) & " of " & $count($a) & ": " & $v })
=> [
  "Item 1 of 4: fred.smith@my-work.com",
  "Item 2 of 4: fsmith@my-work.com",
  "Item 3 of 4: freddy@my-social.com",
  "Item 4 of 4: frederic.smith@very-serious.com"
]
```

## `$filter`

`$filter(array, function)` keeps the items for which `function($value, $index, $array)` is true.

```jsonata-example
input: invoice.json
$filter(Account.Order.Product, function($v, $i, $a) { $v.Price > $average($a.Price) }).`Product Name`
=> "Cloak"
```

## `$single`

`$single(array, function?)` returns the only item for which `function` is true. More than one
match is an error (`D3138`), and so is none (`D3139`). Without a function, the array must have
exactly one item.

```jsonata-example
input: invoice.json
$single(Account.Order.Product, function($v) { $v.SKU = "0406654608" }).`Product Name`
=> "Bowler Hat"
```

```jsonata-example
input: invoice.json
$single(Account.Order.Product, function($v) { $v.Price > 30 })
=> error D3138
```

## `$reduce`

`$reduce(array, function, init?)` folds the array into one value. It calls
`function($accumulator, $value, $index, $array)` for each item, carrying the result to the next
call. Without `init`, the first item is the initial accumulator.

```jsonata-example
$reduce([1..5], function($acc, $v) { $acc * $v })
=> 120
```

```jsonata-example
input: invoice.json
$reduce(Account.Order.Product, function($acc, $p) { $acc + $p.Quantity }, 0)
=> 8
```

## `$sift`

`$sift(object, function)` keeps the fields of `object` for which `function($value, $key,
$object)` is true. With one argument it applies to the context value.

```jsonata-example
input: invoice.json
(Account.Order.Product)[0].$sift(function($v, $k) { $k ~> /^Product/ })
=> {"Product Name": "Bowler Hat", "ProductID": 858383}
```

```jsonata-example
$sift({"a": 1, "b": null, "c": 0}, function($v) { $v != null })
=> {"a": 1, "c": 0}
```

`$sort` takes a comparison function too; see [array functions](array.md#sort).

---

Adapted from the JSONata documentation (© IBM Corp. and JSONata contributors, MIT license).

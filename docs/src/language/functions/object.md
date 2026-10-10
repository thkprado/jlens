# Object functions

Object fields keep the order in which they were inserted.

## `$keys`

`$keys(object)` returns the field names of `object`. For an array of objects, it returns the
names of all their fields, without duplicates.

```jsonata-example
input: address.json
$keys(Address)
=> ["Street", "City", "Postcode"]
```

```jsonata-example
input: address.json
$keys(Phone)
=> ["type", "number"]
```

## `$lookup`

`$lookup(object, key)` returns the value of field `key`, for names that are not known in
advance. For an array of objects, it returns the values from all of them.

```jsonata-example
input: address.json
$lookup(Address, "City")
=> "Winchester"
```

```jsonata-example
input: address.json
(
  $field := "number";
  $lookup(Phone[type = 'mobile'], $field)
)
=> "077 7700 1234"
```

## `$spread`

`$spread(object)` splits an object into an array of objects with one field each. For an array of
objects, it splits each of them.

```jsonata-example
input: address.json
$spread(Address)
=> [{"Street": "Hursley Park"}, {"City": "Winchester"}, {"Postcode": "SO21 2JN"}]
```

## `$merge`

`$merge(array)` merges an array of objects into one. When several objects have the same field,
the last one wins.

```jsonata-example
$merge([{"a": 1, "b": 2}, {"b": 3, "c": 4}])
=> {"a": 1, "b": 3, "c": 4}
```

```jsonata-example
input: address.json
$merge($spread(Address)[$keys($) != "Postcode"])
=> {"Street": "Hursley Park", "City": "Winchester"}
```

## `$each`

`$each(object, function)` calls `function($value, $key)` for each field and returns the results.

```jsonata-example
input: address.json
$each(Address, function($v, $k) { $k & ": " & $v })
=> ["Street: Hursley Park", "City: Winchester", "Postcode: SO21 2JN"]
```

`$sift`, which keeps the fields that satisfy a function, is described with the
[higher-order functions](higher-order.md#sift).

---

Adapted from the JSONata documentation (MIT license).

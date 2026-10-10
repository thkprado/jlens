# Building results

The output of an expression can have any shape. Array and object constructors use JSON syntax,
with expressions wherever values (and object keys) go.

## Arrays

Square brackets build an array of the values of the expressions inside them:

```jsonata-example
input: address.json
[Address.City, Other.`Alternative.Address`.City]
=> ["Winchester", "London"]
```

Unlike a sequence, an array built with `[...]` keeps its structure: it is neither flattened nor
unwrapped when it has a single item. As a path step, it builds one array per context item:

```jsonata-example
input: address.json
Email.address
=> ["fred.smith@my-work.com", "fsmith@my-work.com", "freddy@my-social.com", "frederic.smith@very-serious.com"]
```

```jsonata-example
input: address.json
Email.[address]
=> [
  ["fred.smith@my-work.com", "fsmith@my-work.com"],
  ["freddy@my-social.com", "frederic.smith@very-serious.com"]
]
```

An array constructor can also select several paths at once, as a more precise alternative to
wildcards:

```jsonata-example
input: address.json
[Address, Other.`Alternative.Address`].City
=> ["Winchester", "London"]
```

### Ranges

Inside an array constructor, `from..to` produces the integers from `from` to `to`, both
included. Both ends must be integers (error `T2003`, `T2004`), and a range can have at most
10 million items (`D2014`).

```jsonata-example
[1..5]
=> [1, 2, 3, 4, 5]
```

```jsonata-example
[1..3, 7..9]
=> [1, 2, 3, 7, 8, 9]
```

```jsonata-example
[1..5].($ * $)
=> [1, 4, 9, 16, 25]
```

```jsonata-example
input: address.json
[1..$count(Phone)].("Phone " & $)
=> ["Phone 1", "Phone 2", "Phone 3", "Phone 4"]
```

## Objects

Braces build an object from `key: value` pairs. Keys are expressions that must evaluate to
strings (error `T1003`); values are any expressions. Fields whose value is nothing are left
out.

```jsonata-example
input: address.json
{"name": FirstName, "city": Address.City, "fax": Phone[type = 'fax'].number}
=> {"name": "Fred", "city": "Winchester"}
```

Where the constructor appears changes what it does:

- **After `.`** it is a map step and builds one object per item:

  ```jsonata-example
  input: address.json
  Phone.{type: number}
  => [
    {"home": "0203 544 1234"},
    {"office": "01962 001234"},
    {"office": "01962 001235"},
    {"mobile": "077 7700 1234"}
  ]
  ```

- **Directly after a path** (no `.`) it groups the items into a single object. Values for
  the same key are collected together; see [Grouping](grouping.md#grouping).

  ```jsonata-example
  input: address.json
  Phone{type: number}
  => {
    "home": "0203 544 1234",
    "office": ["01962 001234", "01962 001235"],
    "mobile": "077 7700 1234"
  }
  ```

  With `[]`, every group is an array, whatever its size:

  ```jsonata-example
  input: address.json
  Phone{type: number[]}
  => {
    "home": ["0203 544 1234"],
    "office": ["01962 001234", "01962 001235"],
    "mobile": ["077 7700 1234"]
  }
  ```

## JSON literals

Values of every JSON type can be written as they are in JSON: strings (with JSON escapes, and
either quote character), numbers, `true`, `false`, `null`, arrays and objects. Because JSONata is
a superset of JSON, a JSON document makes a good template for the output.

```jsonata-example
input: invoice.json
{
  "account": Account.`Account Name`,
  "orders": Account.Order.{
    "id": OrderID,
    "total": $round($sum(Product.(Price * Quantity)), 2),
    "items": $count(Product)
  },
  "currency": "EUR"
}
=> {
  "account": "Firefly",
  "orders": [
    {"id": "order103", "total": 90.57, "items": 2},
    {"id": "order104", "total": 245.79, "items": 2}
  ],
  "currency": "EUR"
}
```

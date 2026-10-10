# First steps

This tour uses the `jlens` command line and the files in `fixtures/`. They describe a store
order: a customer and a list of items, each with a type, a color, a unit price, a
quantity and a tax. Every command runs from the repository root.

```json
{
  "number": "SO-12345",
  "customer": { "login": "alice.walker99", "name": "Acme Stores" },
  "items": [
    { "code": "1", "color": "blue",  "type": "apparel",    "unitPrice": 10000.0, "quantity": 2, "tax": 1500.0 },
    { "code": "2", "color": "black", "type": "electronic", "unitPrice": 25000.0, "quantity": 1, "tax": 4000.0 },
    { "code": "3", "color": "white", "type": "electronic", "unitPrice": 12000.0, "quantity": 1, "tax": 3000.0 }
  ]
}
```

## Query

A path selects values. Predicates in square brackets filter them:

```text
$ jlens eval -i fixtures/online-store-order.json 'customer.name'
"Acme Stores"

$ jlens eval -i fixtures/online-store-order.json -c 'items[type = "electronic"].code'
["2","3"]

$ jlens eval -i fixtures/online-store-order.json '$sum(items.(unitPrice * quantity))'
57000
```

The [language reference](../language/overview.md) explains paths, operators and functions.

## Write a rule

`fixtures/shipping-rule.jsonata` is a business rule. Variables hold intermediate totals, then a
condition chooses the outcome:

```jsonata
(
  $total       := $sum(items.(unitPrice * quantity));
  $electronics := $sum(items[type = 'electronic'].(unitPrice * quantity));

  $approved := (
    /** @label "Authorized customer" @message "The customer login must be on the approved partners list." */
    customer.login in ["alice.walker99", "bob.harrison42"]
    and
    /** @label "Minimum order grand total" @message "The order grand total must exceed 50,000.00." */
    $total > 50000
    and
    /** @label "Electronics group" */
    ($electronics > $total * 0.30 + 1000)
  );

  $approved
    ? { "action": "FREE_SHIPPING", "code": "OK_200" }
    : { "action": "REGULAR_FEE", "code": "SHIP_FEE_402", "delivery": "SLOW" }
)
```

(The fixture has a few more checks; this is the gist.)

```text
$ jlens eval -f fixtures/shipping-rule.jsonata -i fixtures/online-store-order.json
{
  "action": "FREE_SHIPPING",
  "code": "OK_200"
}
```

## Explain the result

The `/** @label ... */` comments are *annotations*. They are ordinary comments to other JSONata
implementations, but jlens uses them to explain results:

```text
$ jlens eval -f fixtures/shipping-rule.jsonata -i fixtures/online-store-order.json --explain
result: {"action":"FREE_SHIPPING","code":"OK_200"}
• $total := $sum(items.(unitPrice * quantity)) = 57000
• $electronics := $sum(items[type = 'electronic'].(unitPrice * quantity)) = 37000
  • [type = 'electronic']: 2 of 3 items passed; rejected: {"code":"1","color":"blue",...}
✔ $approved := ( customer.login in [...] and ... ) = true
  ✔ Authorized customer — customer.login in [...] → true ("alice.walker99" in [...])
  ✔ Minimum order grand total — $total > 50000 → true (57000 > 50000)
  ✔ Electronics group — ... → true
      ✔ $electronics > $total * 0.30 + 1000 → true (37000 > 18100)
• $approved ? { ... } : { ... } → {"action":"FREE_SHIPPING","code":"OK_200"} [then]
```

(Output shortened.) `--audit json` prints the full audit tree for programs to read. The
[audit guide](../guides/audit.md) explains audits, levels and annotations.

## Get help while writing

jlens can infer a schema from sample documents. Using it, `jlens check` reports fields that do
not exist, and the language service completes field names:

```text
$ jlens schema infer fixtures/online-store-order.json -o order.schema.json
inferred from 1 sample(s): order.schema.json

$ jlens check --schema order.schema.json 'items[type = "electronic"].unitprice'
warning[W0201]: Field "unitprice" is not in the schema; did you mean `unitPrice`?
 --> <expr>:1:28
  |
1 | items[type = "electronic"].unitprice
  |                            ^^^^^^^^^

$ jlens schema complete --schema order.schema.json "items[type = 'electronic']."
code                     field     string
color                    field     string
type                     field     string
unitPrice                field     number
quantity                 field     integer
tax                      field     number
```

The same service drives editors in the browser; try the `examples/web-editor` demo
([WebAssembly guide](../guides/wasm.md#the-web-editor-example)).

## Next

- Embed the engine in a Rust program: [Rust crate](../guides/crate.md).
- Use it in a browser: [WebAssembly](../guides/wasm.md).
- Process files and streams: [Command-line interface](../guides/cli.md).

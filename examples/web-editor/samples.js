// The examples offered by the web editor. Each has an input document, bindings (a JSON object of
// variables) and an expression.
//
// The Invoice, Address, Schema, Library and Bindings examples are the samples of the JSONata
// Exerciser (https://try.jsonata.org, https://github.com/jsonata-js/jsonata-exerciser,
// src/sample.js), MIT license. The Bindings example is adapted: the
// Exerciser binds a JavaScript function (Math.cos), and jlens bindings are JSON values, so $cosine
// is written in JSONata.

export const samples = [
  {
    id: "store-order",
    label: "Free shipping",
    input: {
      number: "SO-12345",
      customer: { login: "alice.walker99", name: "Acme Stores" },
      items: [
        { code: "1", color: "blue", type: "apparel", unitPrice: 10000.0, quantity: 2, tax: 1500.0 },
        { code: "2", color: "black", type: "electronic", unitPrice: 25000.0, quantity: 1, tax: 4000.0 },
        { code: "3", color: "white", type: "electronic", unitPrice: 12000.0, quantity: 1, tax: 3000.0 },
      ],
    },
    bindings: { approvedCustomers: ["alice.walker99", "bob.harrison42"] },
    expression: `(
  $total       := $sum(items.(unitPrice * quantity));
  $electronics := $sum(items[type = 'electronic'].(unitPrice * quantity));

  $approved :=
    /** @label "Authorized customer" @message "The customer must be an approved partner." */
    customer.login in $approvedCustomers
    and
    /** @label "Minimum order grand total" @message "The total must exceed 50,000.00." */
    $total > 50000
    and
    /** @label "Electronics share" */
    $electronics > $total * 0.30 + 1000;

  $approved
    ? { "action": "FREE_SHIPPING", "code": "OK_200" }
    : { "action": "REGULAR_FEE", "code": "SHIP_FEE_402" }
)`,
  },
  {
    id: "host-functions",
    label: "Host functions",
    input: {
      lines: [
        { sku: "A1", price: "12.50", quantity: 2 },
        { sku: "B2", price: "n/a", quantity: 1 },
        { sku: "C3", price: "7", quantity: 3 },
        { sku: "D4", price: "1,5", quantity: 4 },
      ],
    },
    bindings: {},
    expression: `/* $try is a host function: JavaScript that the editor registers (host-functions.js),
   not part of JSONata. It calls the function it receives and, when that fails, calls the
   fallback with the error. The functions read $line: the context of their call is null. */
(
  $lines := lines.(
    $line := $;
    {
      'sku': sku,
      'amount': $try(
        function() { $number($line.price) * $line.quantity },
        function($error) { {'error': $error.code, 'message': $error.message} }
      )
    }
  );
  {
    'total': $sum($lines[$type(amount) = 'number'].amount),
    'rejected': $lines[$type(amount) = 'object']
  }
)`,
  },
  {
    id: "invoice",
    label: "Invoice",
    input: {
      "Account": {
        "Account Name": "Firefly",
        "Order": [
          {
            "OrderID": "order103",
            "Product": [
              {
                "Product Name": "Bowler Hat",
                "ProductID": 858383,
                "SKU": "0406654608",
                "Description": {
                  "Colour": "Purple",
                  "Width": 300,
                  "Height": 200,
                  "Depth": 210,
                  "Weight": 0.75
                },
                "Price": 34.45,
                "Quantity": 2
              },
              {
                "Product Name": "Trilby hat",
                "ProductID": 858236,
                "SKU": "0406634348",
                "Description": {
                  "Colour": "Orange",
                  "Width": 300,
                  "Height": 200,
                  "Depth": 210,
                  "Weight": 0.6
                },
                "Price": 21.67,
                "Quantity": 1
              }
            ]
          },
          {
            "OrderID": "order104",
            "Product": [
              {
                "Product Name": "Bowler Hat",
                "ProductID": 858383,
                "SKU": "040657863",
                "Description": {
                  "Colour": "Purple",
                  "Width": 300,
                  "Height": 200,
                  "Depth": 210,
                  "Weight": 0.75
                },
                "Price": 34.45,
                "Quantity": 4
              },
              {
                "ProductID": 345664,
                "SKU": "0406654603",
                "Product Name": "Cloak",
                "Description": {
                  "Colour": "Black",
                  "Width": 30,
                  "Height": 20,
                  "Depth": 210,
                  "Weight": 2
                },
                "Price": 107.99,
                "Quantity": 1
              }
            ]
          }
        ]
      }
    },
    bindings: {},
    expression: `$sum(Account.Order.Product.(Price * Quantity))`,
  },
  {
    id: "address",
    label: "Address",
    input: {
      "FirstName": "Fred",
      "Surname": "Smith",
      "Age": 28,
      "Address": {
        "Street": "Hursley Park",
        "City": "Winchester",
        "Postcode": "SO21 2JN"
      },
      "Phone": [
        {
          "type": "home",
          "number": "0203 544 1234"
        },
        {
          "type": "office",
          "number": "01962 001234"
        },
        {
          "type": "office",
          "number": "01962 001235"
        },
        {
          "type": "mobile",
          "number": "077 7700 1234"
        }
      ],
      "Email": [
        {
          "type": "office",
          "address": [
            "fred.smith@my-work.com",
            "fsmith@my-work.com"
          ]
        },
        {
          "type": "home",
          "address": [
            "freddy@my-social.com",
            "frederic.smith@very-serious.com"
          ]
        }
      ],
      "Other": {
        "Over 18 ?": true,
        "Misc": null,
        "Alternative.Address": {
          "Street": "Brick Lane",
          "City": "London",
          "Postcode": "E1 6RF"
        }
      }
    },
    bindings: {},
    expression: `{
  "name": FirstName & " " & Surname,
  "mobile": Phone[type = "mobile"].number
}`,
  },
  {
    id: "schema",
    label: "Schema",
    input: {
      "$schema": "http://json-schema.org/draft-04/schema#",
      "required": [
        "Account"
      ],
      "type": "object",
      "id": "file://input-schema.json",
      "properties": {
        "Account": {
          "required": [
            "Order"
          ],
          "type": "object",
          "properties": {
            "Customer": {
              "required": [
                "First Name",
                "Surname"
              ],
              "type": "object",
              "properties": {
                "First Name": {
                  "type": "string"
                },
                "Surname": {
                  "type": "string"
                }
              }
            },
            "AccID": {
              "type": "string"
            },
            "Order": {
              "items": {
                "required": [
                  "OrderID",
                  "Product"
                ],
                "type": "object",
                "properties": {
                  "OrderID": {
                    "type": "string"
                  },
                  "Product": {
                    "items": {
                      "required": [
                        "ProductID",
                        "Product Name",
                        "Price",
                        "Quantity"
                      ],
                      "type": "object",
                      "properties": {
                        "SKU": {
                          "type": "string"
                        },
                        "Description": {
                          "type": "object",
                          "properties": {
                            "Width": {
                              "type": "integer"
                            },
                            "Depth": {
                              "type": "integer"
                            },
                            "Weight": {
                              "type": "number"
                            },
                            "Colour": {
                              "type": "string"
                            },
                            "Material": {
                              "type": "string"
                            },
                            "Height": {
                              "type": "integer"
                            }
                          }
                        },
                        "Product Name": {
                          "type": "string"
                        },
                        "Price": {
                          "type": "number"
                        },
                        "Quantity": {
                          "type": "integer"
                        },
                        "ProductID": {
                          "type": "integer"
                        }
                      }
                    },
                    "type": "array"
                  }
                }
              },
              "type": "array"
            },
            "Account Name": {
              "type": "string"
            },
            "Address": {
              "required": [
                "Address Line 1",
                "City",
                "Postcode"
              ],
              "type": "object",
              "properties": {
                "Address Line 1": {
                  "type": "string"
                },
                "Address Line 2": {
                  "type": "string"
                },
                "Postcode": {
                  "type": "string"
                },
                "City": {
                  "type": "string"
                }
              }
            }
          }
        }
      }
    },
    bindings: {},
    expression: `**.properties ~> $keys()`,
  },
  {
    id: "library",
    label: "Library",
    input: {
      "library": {
        "books": [
          {
            "title": "Structure and Interpretation of Computer Programs",
            "authors": [
              "Abelson",
              "Sussman"
            ],
            "isbn": "9780262510875",
            "price": 38.9,
            "copies": 2
          },
          {
            "title": "The C Programming Language",
            "authors": [
              "Kernighan",
              "Richie"
            ],
            "isbn": "9780131103627",
            "price": 33.59,
            "copies": 3
          },
          {
            "title": "The AWK Programming Language",
            "authors": [
              "Aho",
              "Kernighan",
              "Weinberger"
            ],
            "isbn": "9780201079814",
            "copies": 1
          },
          {
            "title": "Compilers: Principles, Techniques, and Tools",
            "authors": [
              "Aho",
              "Lam",
              "Sethi",
              "Ullman"
            ],
            "isbn": "9780201100884",
            "price": 23.38,
            "copies": 1
          }
        ],
        "loans": [
          {
            "customer": "10001",
            "isbn": "9780262510875",
            "return": "2016-12-05"
          },
          {
            "customer": "10003",
            "isbn": "9780201100884",
            "return": "2016-10-22"
          },
          {
            "customer": "10003",
            "isbn": "9780262510875",
            "return": "2016-12-22"
          }
        ],
        "customers": [
          {
            "id": "10001",
            "name": "Joe Doe",
            "address": {
              "street": "2 Long Road",
              "city": "Winchester",
              "postcode": "SO22 5PU"
            }
          },
          {
            "id": "10002",
            "name": "Fred Bloggs",
            "address": {
              "street": "56 Letsby Avenue",
              "city": "Winchester",
              "postcode": "SO22 4WD"
            }
          },
          {
            "id": "10003",
            "name": "Jason Arthur",
            "address": {
              "street": "1 Preddy Gate",
              "city": "Southampton",
              "postcode": "SO14 0MG"
            }
          }
        ]
      }
    },
    bindings: {},
    expression: `library.loans@$L.books@$B[$L.isbn=$B.isbn].customers[$L.customer=id].{
  'customer': name,
  'book': $B.title,
  'due': $L.return
}`,
  },
  {
    id: "bindings",
    label: "Bindings",
    input: {
      "angle": 60
    },
    bindings: {"pi":3.1415926535898},
    expression: `/* $pi comes from the Bindings panel; $cosine is a Taylor series. */
(
  $factorial := function($n) { $reduce([1..$n], function($a, $b) { $a * $b }, 1) };
  $cosine := function($x) {
    $sum([0..10].($power(-1, $) * $power($x, 2 * $) / $factorial(2 * $)))
  };
  $cosine(angle * $pi / 180)
)`,
  },
];

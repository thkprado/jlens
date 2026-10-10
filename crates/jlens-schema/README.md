# jlens-schema

Structural metadata for JSON documents, and a language service for
[JSONata](https://jsonata.org/) expressions. Part of [jlens](https://github.com/thkprado/jlens).

- **Infer** a schema from sample documents. It records types, required and optional fields,
  number ranges, string formats, small enumerations and examples.
- **Describe** fields with descriptions, labels, allowed values and hidden flags, and convert
  to and from JSON Schema 2020-12.
- **Assist** people writing expressions: completion of fields, variables and functions;
  diagnostics for fields that do not exist (with "did you mean"); hover; signature help.

```rust
use jlens_schema::{LanguageService, Schema};
use serde_json::json;

let schema = Schema::infer(&[json!({"items": [{"type": "x", "price": 1}]})]);
let service = LanguageService::new(Some(schema));

let text = "items[type = 'x'].";
let labels: Vec<_> = service
    .complete(text, text.len())
    .items
    .into_iter()
    .map(|item| item.label)
    .collect();
assert_eq!(labels, ["type", "price"]);

let warnings = service.diagnostics("items.prise");
assert!(warnings[0].message.contains("did you mean `price`"));
```

See the [API reference](https://docs.rs/jlens-schema) and the schema guide of the jlens book.

## License

MIT.

# 0025 — A `$ref` to another document: the application gives the documents, and the load reports what it did not follow

**Status:** Accepted.

## Context

Until 0.32 `jlens-schema` followed a `$ref` only as a JSON Pointer into its own document
(`#`, `#/$defs/…`). Any other failed the load whole with `cannot resolve $ref`: a schema split
into files, one that names its parts by `$id` or `$anchor`, or one with a single `$ref` to a
definition that was removed. An application that checks stored expressions against a schema
that another system sends (roadmap 0.33) then had nothing to check them against.

Loading such a schema also met an old bug. The target of a `$ref` that is not recursive is
copied in the place of each `$ref` ([ADR 0024](0024-recursive-schemas.md)), so definitions that
each refer twice to the next double at each level: 20 levels, a document of 1.9 KB, loaded in
2.2 s, and 30 would not end. A document from another system must not be able to do that.

## Decision

1. **jlens never fetches a document.** The application gives each one with its URI
   (`SchemaLoader::document`), and the load reads only what it was given. A library that opens
   a URL found in its input would give whoever writes a schema a request from the application's
   network, and a load whose result depends on the network is not the same twice.
2. **A `$ref` is resolved as JSON Schema 2020-12 says.** Its base is the `$id` of the nearest
   schema around it that has one, itself resolved against the one above, and at last the URI
   the document was given with (`SchemaLoader::load_at`), or none. A document is known by the
   URI it was given with and by each `$id` in it. The fragment is a JSON Pointer from the
   schema the rest of the URI names, or the `$anchor` of a schema in it; an `$id` that is a
   fragment alone (`"$id": "#name"`) is read as an anchor, as it was until draft-07. Two URIs
   are the same when their texts are equal once resolved (RFC 3986, section 5.2): nothing is
   normalized but the dot segments of a path. A base may be relative or empty: with no `$id`
   and no URI, `common.json#/$defs/person` is a place of the document given as `common.json`.
3. **A `$ref` that cannot be followed leaves its place unknown, and the load goes on.** The
   place refers to a definition the schema does not have, named by the resolved URI, which
   [ADR 0024](0024-recursive-schemas.md) already reads as unknown: nothing is completed and
   nothing is warned under it, whatever keywords are beside the `$ref` or merged with it by
   `allOf`, `anyOf` or `oneOf`, since the missing schema could declare any field. What an
   expression uses marks the place unknown, so giving the document later shows as a change.
   `Schema::to_json_schema` writes such a reference back as the `$ref` it was resolved to.
4. **The load reports what it did not follow** (`LoadReport`), beside the schema: for each
   `$ref`, its place (the document's URI and a JSON Pointer), its text, the URI it resolves to,
   and the document that was not given, or none when the document is there without that place.
   `LoadReport::documents` lists the documents to give. Only the `$ref`s the load met are
   reported: one under a definition that nothing leads to is not read. The report is a struct
   that may grow: the keywords the load does not read are to be reported the same way
   (roadmap 0.34).
5. **A load is bounded by the size of its documents.** Before reading a shape, the load counts
   the JSON values it would read, each target once for each `$ref` to it. Within the budget, 16
   times the values of the documents and no less than 100,000, every `$ref` that is not
   recursive is copied in place, as before. Past it, each target that would read more than its
   share, the budget divided by the number of `$ref`s, is kept once as a definition, as a
   recursive one is; the load then reads no more than the documents' values plus the budget.
   Failing the load instead was dropped: the schema is valid, and a definition serves it.

## Consequences

- `Schema::from_json_schema` no longer fails for a `$ref` it cannot follow; it loads with no
  other document and drops the report. The two failures left are a document that is not a
  schema and one nested more than 64 levels deep.
- A `$ref` inside a schema with its own `$id` is now resolved against that `$id`: a pointer
  starts at that schema, not at the document's root. Bundled schemas, which embed each document
  with its `$id`, depend on it.
- A property named `$ref`, `$id` or `enum` is a property: the load tells a map of schemas
  (`properties`, `$defs`, `definitions`, `patternProperties`, `dependentSchemas`) from a schema,
  and reads no schema under `enum`, `const`, `examples` and `default`.
- A schema past the budget has definitions that are not recursive. Their places differ from a
  copy in what ADR 0024 lists: the keywords beside the `$ref` count only with a `type`, an
  override through one changes every place that refers to it, and under a value read whole,
  what is under one is listed once. Written back and read again, such a schema may copy in
  place what it kept, and answers the same.
- The budget's worst case, a schema just under it, loads in about 40 ms (a doubling of 14
  levels; Ryzen, release build); the same schema with 20, 30 or 400 levels loads in 3 to 4 ms.
- `$dynamicRef`, `$recursiveRef` and the `id` of draft-04 are not read; the survey of the
  keywords jlens ignores is roadmap 0.34.
- The packages over `jlens-schema` (the CLI, the language server, the WebAssembly and Java
  packages, the C API) load such schemas at once, as they call `Schema::from_json_schema`.
  Giving them the documents and the report is a separate item of the roadmap.

## Results: the documents and the report at every end (0.33 #10)

- **One JSON form of the report**, that of `LoadReport`: `{"unresolved": [{"place",
  "reference", "uri", "document"}]}`, `document` absent when the document is there without
  the place, and `{}` when every `$ref` was followed. `fixtures/uses/cases.json` has, under
  `references`, a schema of two documents, its report with the other document and alone, and
  for two expressions what each uses and what giving the document changes; the Rust crate,
  the C API, the Java package, the WebAssembly package and the CLI each answer it.
- **The C API**: `jlens_schema_new` takes a second buffer, the sources (0 for none): the JSON
  object `{"uri": …, "documents": {uri: buffer}}`, each document in a buffer of its own, named
  by its handle as a variable's schema is in a service's declarations
  ([ADR 0023](0023-schema-through-the-c-api.md)). Inlining the documents in the sources would
  make the host write one JSON text out of several, and a document that is not JSON would
  fail as the sources. `jlens_schema_report` replies with the report of a schema, which is
  kept with it. Sources not in their form fail with `U2005` and the token `sources`.
- **Java**: `Schema.loader()` with `uri`, `document` and `load`; `Schema.fromJson` is the load
  with no document. `unresolved()` is a list of `UnresolvedRef` records and
  `missingDocuments()` the documents to give. The report is read once, when the schema is
  made: one more call for each schema, and none after, nor after the schema is closed.
- **The WebAssembly package** has no schema object: `LanguageService` takes the sources as its
  fourth argument, `{ uri, documents }`, and `schemaReport(schema, sources)` gives the report
  of one schema. The documents serve the schemas of the variables too; `uri` is that of the
  input's schema.
- **The CLI and the language server read files, never the network.** A document is read from
  the file at the same relative path from the schema's file as between the two URIs: that of
  the document, and the schema's `$id` or else the URI of its file. So `common.json` beside the
  schema is found whether the schema has an `$id` on some host or none, and a document on
  another host, or with a query, is not read. `..` is allowed: the schemas are the user's own
  files, and what is read is used only as a schema. A variable's schema, written in
  `jlens.json`, refers to files from that folder. `SchemaLoader::load_asking`, in
  `jlens-schema`, is the loop behind it: load, ask a function for the documents the report
  names, load again with those it gave, until none is new; each URI is asked for once and no
  more than 256, so that links between folders cannot make it endless. A chain of documents
  more than 64 deep still fails as a schema nested too deep.
- **What is not followed is a warning, not a failure**: a line on standard error for each
  document not found, with the number of `$ref`s to it, and one for each `$ref` to a place
  its document does not have; a message in the language server, once for each service it
  makes. `--deny-warnings` does not count them: they are about the schema, not the
  expression checked. A file that is there and is not JSON fails, as the schema's does.
- **The size.** The Windows library is 3,637,760 bytes, 16.4% over 0.32.0, against 12.2%
  after 0.33 #7: the recursive schemas (#8), the loader with its URI resolver (#9) and the
  two exports of this item. Roadmap 0.33 #12 measures the library and the JAR as released.

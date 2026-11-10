# 0024 — A recursive schema keeps its `$ref`s as references to definitions

**Status:** Accepted. Amended by [ADR 0025](0025-references-to-other-documents.md): a target
that is not recursive is kept as a definition too when its copies would make the load much
larger than the schema, and a `$ref` that cannot be followed is a reference to a name the
schema does not define.

## Context

`jlens-schema` reads a JSON Schema into a tree of shapes (`Shape`): each field and each array's
items own their shape, and the target of a `$ref` is copied in its place. A recursive schema,
one where a `$ref` leads back to itself (`"$ref": "#"`, a definition that refers to itself, two
that refer to each other), has no such tree. Until 0.32 the load copied 64 levels and failed
whole with `$ref nesting is too deep`, which left an application nothing to check its
expressions against (roadmap 0.33 #8). Trees, linked lists, expressions and the `Optional[Node]`
of generated schemas (`anyOf: [{"$ref": …}, {"type": "null"}]`) are common.

Three ways to hold a recursive schema were weighed:

- **Copy the target a fixed number of levels.** The tree stays, but an expression deeper than
  the bound gets no completion and no warning, and the size multiplies: a node with two
  references to itself is 2ⁿ shapes at level n, and n definitions that all refer to each other
  are n! paths before one repeats.
- **Copy lazily**, a level when an expression first reaches it. The shapes keep one address
  each, which the typing relies on, but the schema then changes while it is shared between
  threads, a walk over everything (`**`, a value read whole) never ends by itself, and a shape
  cloned away from its schema loses what it refers to.
- **Keep the reference**, by name, and follow it where the shapes are read.

## Decision

1. **A recursive `$ref` becomes a reference by name.** `Schema` has `definitions`, a map from
   names to shapes, and `Shape` has `references`, the names of the definitions the value may
   also have the shape of. A value has the shape of any of them, and of the shape itself when it
   has types of its own: `anyOf: [{"$ref": "#/$defs/node"}, {"type": "null"}]` is a shape of
   type `null` that refers to `node`, and `allOf: [{"$ref": "#/$defs/base"}]` beside
   `properties` is an object that also has the fields of `base`. The typing already takes a
   type as a set of shapes, so the references add members to the set and nothing is merged.
2. **Only what is recursive is a definition.** The load finds the `$ref`s that lead back to
   themselves (the strongly connected components of the graph of references) before it reads a
   shape. Every other `$ref` is copied in place as before: a schema that loaded until 0.32 gives
   the same shapes. A recursive root is itself a reference to the definition `#`. (Since
   [ADR 0025](0025-references-to-other-documents.md), a schema whose copies would pass the
   load's budget keeps the targets copied most as definitions.)
3. **The names** are those of `#/$defs/…`, and the text of the `$ref` otherwise.
   `Schema::to_json_schema` writes the definitions under `$defs` and the references as `$ref`
   (one) or `anyOf` (several), and what it writes reads back as the same schema.
4. **Who reads a shape follows its references,** with the schema's definitions at hand:
   `Schema::shape_at`, the overrides, the typing (completion, hover, the diagnostics) and what
   an expression uses. They go as deep as the path or the expression goes. A walk over
   everything stops where it meets a shape again: `**` gives each shape once, and under a value
   read whole, what an expression uses lists what is under each definition once, at the place
   nearest to that value, first in the order of the paths.
5. **A reference that leads nowhere is unknown.** A name the schema does not define, or
   references that only lead to each other, leave their place without a shape: nothing is
   completed and nothing is warned under it.
6. **A variable is declared with its schema** (`LanguageService::with_variable_schema`), as its
   definitions are read with it. `with_variable`, which takes a shape alone, stays: the
   references of such a shape are unknown.

## Consequences

- A path of the overrides that goes through a reference changes the definition, and so every
  place that refers to it: `children[].label` in a tree describes the `label` of every node.
- Annotations beside a recursive `$ref` (`description`, `title`) describe that field in
  completion and hover. Other keywords beside it count only with a `type`: a shape without
  types that refers to a definition stands for the definition alone.
- `Shape` and `Schema` gained a public field each, and a `Schema` built by hand must give
  `definitions`; `..Schema::default()` does. Code that walks `Shape::fields` and `Shape::items`
  by itself does not see through a reference: `Schema::members` gives the shapes one stands for.
- What an expression uses of a recursive schema is finite, and still sound for the comparison of
  two schemas: a change inside a definition shows in the one place it is listed. Where that
  place is does not depend on the order the schema is written in.
- A schema nested more than 64 levels without recursion still fails, now with `the schema is
  nested too deep`.

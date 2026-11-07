# 0022 — Objects keep their fields in a vector, with an index past 8 fields

**Status:** Accepted.

## Context

Every object jlens builds or converts holds its fields in an `IndexMap<Rc<str>, Value>` hashed
with foldhash: a vector of entries and a hash table of their positions, each allocated apart.
Tuples are such objects, one per item, and so is each match `$match` returns. A lookup hashes
its key, whatever the size of the object.

Roadmap 0.31 #1 counted what objects hold at 0.30.0
([notes](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.31.md)):

- No object the benchmark workload builds, converts or reads has more than 6 fields. Of the 483
  objects of the JSONata test suite's datasets, 86% have 3 fields or fewer, 99.6% 6 or fewer, the
  largest 11.
- Lookups matter as much as construction: a path over the items looks its key up in each item.
- The hash table is one of the 7 allocations of an object built by a constructor, one of the 4 of
  a match of `$match`, and one of the 3 of a tuple. The map makes an `Object` 104 bytes, 64 of
  them the `IndexMap` and its hasher.

Objects can still be large: a document whose keys are identifiers has thousands of fields in one
object, and reading or building it must stay linear.

## Decision

1. **A map of jlens's own,** `jlens_core::map::Map`, private to the crate: the fields in a vector,
   in insertion order, with their shared `Rc<str>` keys. A lookup compares the key's length with
   each field's, then its text. Its operations are those of `IndexMap` that jlens uses, with
   the same meaning: a key inserted again keeps its position and its first key, `shift_remove`
   keeps the order, `swap_remove` moves the last field into the gap.
2. **An index past 8 fields.** A map of more than 8 fields keeps the position of each key in a
   `hashbrown::HashTable<usize>` hashed with foldhash, in one allocation apart, built whole when
   the map passes 8 fields and dropped when a removal brings it back to 8. Measured on 0.31 #2
   (below), up to 8 fields a search in order finds a key present faster than foldhash, one
   absent within about a nanosecond, and builds the map faster.
3. **No fields kept in the object itself.** The fields stay in the vector's allocation.
4. **No hasher in each map.** A map without an index has no hasher; an index makes its own
   `RandomState`. The hasher that the lazy values of one evaluation share (ADR 0010) goes away
   with `IndexMap`.

A property test runs random operations on the map and on `IndexMap` and compares their answers,
fields and order after each. `IndexMap` stays for the maps that are not objects (the groups of
`group-by`, the host functions).

## The measurements

Ryzen 3700X, Windows, the system heap; times of one lookup in a map of _n_ fields with keys of
the workload's kind (`code`, `customer`, …), averaged over every key, and of building a map of _n_
fields with a check for duplicates, dropped after.

| Fields | Key present: in order / foldhash | Key absent | Building |
|---:|---:|---:|---:|
| 1 | 5.1 / 6.5 ns | 2.3 / 4.5 ns | 35 / 86 ns |
| 3 | 6.3 / 10.9 | 4.5 / 5.7 | 43 / 115 |
| 6 | 7.5 / 11.0 | 5.7 / 5.8 | 60 / 164 |
| 8 | 8.6 / 11.2 | 7.1 / 5.8 | 75 / 192 |
| 12 | 11.4 / 11.2 | 10.4 / 5.8 | 128 / 256 |
| 16 | 13.4 / 11.7 | 14.2 / 5.8 | 189 / 319 |
| 64 | 36.6 / 12.1 | 44.1 / 6.3 | 2,302 / 1,048 |

With `mimalloc`, building costs 10 to 37 ns in order against 51 to 134 ns with foldhash up to 6
fields. Fields kept in the object were measured too: 3 or 4 of them save 10 to 20 ns per object
of 2 or 3 fields on the system heap, nothing with `mimalloc`, cost 20 to 40 ns per object that
holds more (the workload's items have 6), and make each small object about three times larger.

## Consequences

- An object takes one allocation fewer, and an `Object` 72 bytes instead of 104. A lookup in an
  object of 6 fields or fewer hashes nothing.
- An object of more than 8 fields costs what it cost: a vector, a table, a hashed key per lookup.
  Between 9 and about 12 fields, a key present is found as fast either way.
- `hashbrown` becomes a direct dependency of `jlens-core`; it was already in its tree, through
  `indexmap`.
- Nothing public changes. `deep_eq` and `$distinct` can compare two small objects field by field
  (roadmap 0.31 #4).

## Alternatives

- **Keep `IndexMap`.** It keeps the hash table and the hashed lookups that the measurements show
  slower for the objects jlens meets.
- **No index at all.** An object of thousands of fields would be quadratic to build and slow to
  read.
- **Fields in the object, a `SmallVec`.** Measured above: a gain on the system heap only, a loss
  with `mimalloc` and for larger objects, and three times the memory of a small object, which a
  document of 10,000 items pays 10,000 times.
- **Keys sorted, searched by halves.** The fields keep their insertion order, which JSONata shows;
  a sorted copy would be a second allocation, as the hash table is.

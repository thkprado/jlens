# Performance notes: 0.28

How the performance work of roadmap 0.28 was measured: methods, traps and intermediate
numbers that the [performance page](../src/architecture/performance.md) leaves out. The page holds the
current tables and what a user needs; the item numbers are those of
[the roadmap](../src/architecture/roadmap.md#028-what-027-left-in-distinct-and-match-and-the-regex-flag-u).

## `$distinct` below a shared field (0.28 #2)

0.27 looked items past the first 16 up by the first fields of an object, as far as they were
scalars, and hashed the others whole ([0.27 notes](perf-0.27.md#distinct-027-6-12)). Items that
differ only below a field they all share, or arrays, were read whole: 2.65 ms for 25 items of
100 KB against a `Document`, where comparing them one by one takes 17.5 µs.

The first fields now go down a chain of levels: the scalars an array or an object starts with,
then the array or object after them, and the same within it, up to eight scalars and eight
levels. Comparing an item that starts the same way reads exactly those first, in its own order,
so a difference there makes it false with nothing else of either item read, and the result and
the errors stay the list's. A level below is taken only when the 16 items kept, told apart by
the levels above it, fall in eight groups or fewer: an item then shares those levels with some of
them, and comparing it with those reads the level below too. Records whose first fields differ
stay on their first level, so that a `Source` is not asked for levels comparing would not read
([ADR 0010](../src/adr/0010-lazy-input.md#amendment-a-level-read-in-case-it-is-needed-027)).

`$count($distinct($))`, in µs, the faster of two processes, each on a new copy of the program:
0.27, now, and comparing alone (the list, the 16 raised past the array's length). `rows` is a
list of `{a, b, c}` records whose first `a` differs by item where named; `{kind, list}` has a
`kind` all the items share; `[kind, list]` is the same as an array; `{kind, meta: {v, list}}`
one level further down.

| Items | `Document`, 0.27 | now | list | `evaluate`, 0.27 | now | list |
|---|---:|---:|---:|---:|---:|---:|
| 25 of `{kind, list}`, 100 KB | 12,033 | 14.7 | 25.6 | 51,687 | 6,542 | 5,848 |
| 128 of `{kind, list}`, 10 KB | 5,441 | 24.7 | 736 | 23,716 | 2,430 | 3,265 |
| 25 of `[kind, list]`, 100 KB | 12,145 | 11.6 | 19.1 | 51,093 | 5,488 | 5,637 |
| 25 of `{kind, meta: {v, list}}`, 100 KB | 12,175 | 21.6 | 42.1 | 53,068 | 5,681 | 5,622 |
| 128 of `{rows, id}`, 10 KB | 5,302 | 20.8 | 575 | 24,180 | 2,555 | 3,103 |
| 25 of `{id, rows}`, 100 KB | 5.54 | 5.75 | 9.34 | 14.5 | 14.8 | 17.9 |
| 128 of `{id, rows}`, 100 KB | 9.60 | 9.78 | 257 | 56.5 | 54.9 | 266 |
| 128 of `{id, text}`, 100 KB | 11.0 | 11.5 | 203 | 4,624 | 4,729 | 4,803 |
| 128 of `{id, 7,000 fields}`, 100 KB | 14.6 | 14.9 | 267 | 102,401 | 101,950 | 106,937 |

With `evaluate` the time is reading the levels below, which comparing reads as well; against a
`Document` it is the lookups. `{rows, id}`, the `id` after the list, gains as well: the first
fields follow the list to its first record.

The first version walked the levels in one loop for every item, and cost records of three fields
about 2 ns per item, 4 to 5% on 1,000 of them against a `Document`, aligned builds included, so
not placement. One level of an object, the common case, has its own loop now. With functions and
blocks aligned (`-C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=5`),
the medians of eight processes: 25 records of three fields 6.72 → 6.42 µs and 1,000 of them 52.4
→ 52.0 µs. Unaligned, the cases of 0.27's tables (numbers, strings, `i % 8`, objects of three
fields, nested objects half repeated, from 25 to 100,000 items) stay within −1 and +4%, the spread
of copies of one program. Two smaller costs went with it: the groups are counted on the table
the lookups use, with no list sorted, and the size of each level is left out of the hash.

`cargo xtask ab` against 9d2aa35, seven rounds: `distinct items` +1.4% against a `Document`,
+0.9% with `evaluate`, +1.0% through a `Source`. Against a `Document`, `path` +3.4%, `object
constructor` +3.4% and `transform` +3.1% (fastest +0.4%) run no `$distinct`; 0.27 measured
`path` from −1.2 to +4.1% over five runs. Conformance and 20,000 differential cases agree.

The tests: a property over nested shapes in the unit tests, checked by a mutation (items kept
read by position, not by key) that it catches; and through a `Source`, a level the first fields
read and cannot (the 16th item kept the first of its length, so that no comparison has read it),
which must go back to comparing and fail as comparing does. Its mutation, such a level taken as
"differs from all", passes the property over random items, which rarely build that order, and
fails the named test.

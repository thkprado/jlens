# Performance notes: 0.28

How the performance work of roadmap 0.28 was measured: methods, traps and intermediate
numbers that the [performance page](../src/architecture/performance.md) leaves out. The page holds the
current tables and what a user needs; the item numbers are those of
[the roadmap](../src/architecture/roadmap.md#028-what-027-left-in-distinct-and-match-and-the-regex-flags-u-and-v).

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

## `$distinct` with few distinct values (0.28 #3)

An array with fewer than 16 distinct values never leaves the list, so each item is compared with
each value kept until one equals it: 4.5 comparisons an item on `i % 8`. 0.27 measured 1.48 ms
on 100,000 of them against a `Document`, and jsonata-js 229 µs. The cause was the cost of each
comparison, not their number. Each went through `deep_eq`, a call that returns a `Result` and
that does not inline, as it recurses, at about 5 ns. A number or a string equals only a value of
its own kind, compared as `===` compares it, with nothing read and no error. So the list compares
a number or a string in a loop of its own, inline, and only arrays and objects through
`deep_eq`. Strings still took 26 ns an item: `==` on two `str` calls `memcmp`, which costs more
than the comparison on a few bytes. `Text`'s equality now compares strings of up to 16 bytes
itself, byte by byte, everywhere strings are compared. It is `#[inline(always)]`: with
`#[inline]` it stayed a call, and 100,000 strings took 1.75 ms against 0.81 ms.

`$count($distinct($))`, in µs, the faster of two processes, each on a new copy of the program,
before and after, alternated. `few` is 0.27's `i % 8`; `few strings` holds `"s" & (i % 8)`, and
`few objects` `{"a": i % 8, "b": "x"}`. jsonata-js 2.2.2 ran on the
same machine, the median of 15 samples in one process after a warm-up:

| Items | `Document`, before | after | `evaluate`, before | after | jsonata-js |
|---|---:|---:|---:|---:|---:|
| 25 `few` | 1.94 | 1.54 | 2.46 | 2.00 | |
| 1,000 `few` | 23.1 | 4.66 | 33.8 | 15.3 | |
| 100,000 `few` | 2,185 | 312 | 3,556 | 1,685 | 355 |
| 25 `few strings` | 2.40 | 1.65 | 4.06 | 3.32 | |
| 1,000 `few strings` | 42.2 | 9.40 | 84.4 | 51.5 | |
| 100,000 `few strings` | 4,161 | 803 | 10,042 | 6,663 | 3,473 |
| 1,000 `few objects` | 134 | 133 | 414 | 414 | 1,172 |

This machine gives jsonata-js 355 µs where 0.27's table gave 229. Distinct numbers, strings and
objects of three fields, from 25 to 100,000 items, stay within the spread of copies of one
program. With `evaluate`, the time left is converting the input. Objects with few values
compare through `deep_eq`, 30 ns a comparison, and stay 9 times ahead of jsonata-js.

`cargo xtask ab` against 8cfacf5, seven rounds: against a `Document`, every case from −1.9 to
+1.4% (`strings` −1.2%, `distinct items` +1.4%); with `evaluate`, from −2.1 to +0.4%. Conformance
and 20,000 differential cases agree. A unit test compares strings on each side of the 16 bytes,
differing at the first, the middle and the last byte, and joined texts with flat ones.

## `$match` and the allocations of each match (0.28 #4)

0.27 left `$match` at 0.70 µs a match against 0.21 to 0.30 µs in jsonata-js, and named five
allocations: the matched string, the object, its map's table and entries, and the groups. A
counting allocator in a program of its own found 6.5 a match for `/\w+/` on `text-20k.json`, and 2
a piece for `$split`: the search allocated too. `find_from_interruptible` built a backtracker for
each search, with its backtrack stack, and the stack grew again each time.

The same program with mimalloc in place of the Windows heap took `$match` from 1,060 to 480 µs:
on this machine, an allocation and its free cost about 57 ns, more than half the time of a match.
So the change counts allocations:

- `jlens-regress` has a `Searcher`, which searches one text as `find_from_interruptible` does and
  keeps the backtracker's storage from one search to the next. A completed search leaves it as a
  new one has it, one `Exhausted` backtrack and no group set (the upstream iterator relies on
  this too); after an interrupt it is dropped. `regex::Matches`, which runs the matches of
  `$match`, `$replace`, `$split` and `$contains`, keeps one for its text.
- The matches of a regex without groups share one empty `groups` array, made once a call.
  Nothing changes an array of the result in place; the only flag set on an array that is not
  fresh, `KEEP_SINGLETON`, is read on sequences alone.

The four allocations left are those of the object: its string, its `Rc`, and the table and
entries of its `IndexMap`. Fewer needs a representation of small objects without a hash table,
which every object of the crate shares; tuples meet the same cost.

On `text-20k.json` against a `Document`, in µs, the median of 15 samples, each program a new copy,
before and after alternated; jsonata-js 2.2.2 on the same machine, the median of 15 samples after
a warm-up:

| Expression | Allocations, before | after | Before | After | jsonata-js |
|---|---:|---:|---:|---:|---:|
| `$count($match(text, /\w+/))`, 1,558 matches | 10,182 | 6,290 | 1,059 | 808 | 356 |
| `$count($match(text, /(\w)(\w*)/))` | 20,320 | 14,090 | 1,631 | 1,265 | 386 |
| `$count($split(text, /\s/))`, 2,076 pieces | 4,205 | 2,131 | 445 | 343 | 447 |
| `$length($replace(text, /\w+/, "x"))` | 2,396 | 61 | 346 | 232 | 476 |

Per match, `$match` 0.68 → 0.52 µs. With mimalloc it takes 377 µs, against 478 before, and
jsonata-js 356: the gap left is the Windows heap. With groups, each match also allocates the
capture list regress returns, the list of groups and its array, and a string per group.

`cargo xtask growth` against the code before (`$match`, `$split` and `$replace`, 1,000 to 64,000
items, in both modes): `$match` −20%, `$split` with a regex −25%, `$replace` with a regex −20%,
`$replace` with a function −2%, the texts within their noise; every case stays linear.

`cargo xtask ab` against 409bd34, seven rounds: `regex over a long text` 340 → 218 µs against a
`Document` and 344 → 218 µs with `evaluate`, −36%; `regex`, short texts, +1.5% and +0.1%; every
other case within the floor, `names over items` with `evaluate` (+5.1%) and `path` against a
`Document` (+3.4%) at +2.3% and +2.6% when run again alone, ten rounds, where nothing changed.
A test of `jlens-regress` compares a searcher's matches with searches of their own from every
start of thirteen patterns (groups, backreferences, lookarounds, `u`, `i`, `m`), and after an
interrupt.

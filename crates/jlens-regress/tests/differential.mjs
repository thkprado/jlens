// Runs the cases written by `cargo xtask regex-differential` in V8 and writes one outcome per
// case. The xtask runs the same cases in jlens-regress and compares the outcomes.
//
//   node differential.mjs <cases.json> <outcomes.json>
//
// An outcome is {"error": true} when the pattern does not compile with its flags, {"hung": true}
// when its searches did not finish within the timeout, or {"results": [...]}, one per input:
// {"matches": [...], "split": <bool>}. Each input is searched as `String.prototype.matchAll`
// does, up to `max` matches; a match is {"ranges": [...], "groups": {...}}, the UTF-16 ranges of
// the match and of its groups (`null` for a group that did not take part), read with the `d`
// flag. "split" is true when the search stopped because V8 would go on, or a match would start
// or end, inside a surrogate pair, which jlens-regress cannot.
//
// The searches run in their own realm, under `vm`'s timeout, which interrupts a match.

import { readFileSync, writeFileSync } from "node:fs";
import vm from "node:vm";

const [casesPath, outcomesPath] = process.argv.slice(2);
if (!casesPath || !outcomesPath) {
  console.error("usage: node differential.mjs <cases.json> <outcomes.json>");
  process.exit(2);
}
const { cases, timeout, max } = JSON.parse(readFileSync(casesPath, "utf8"));

const context = vm.createContext({});
vm.runInContext(`globalThis.run = ${run.toString()}`, context);

const outcomes = cases.map((c) => {
  context.c = c;
  context.max = max;
  try {
    return JSON.parse(vm.runInContext("JSON.stringify(run(c, max))", context, { timeout }));
  } catch (e) {
    if (e.code === "ERR_SCRIPT_EXECUTION_TIMEOUT") return { hung: true };
    throw e;
  }
});
writeFileSync(outcomesPath, JSON.stringify(outcomes));

// Runs inside the realm: it may refer to nothing outside its body.
function run({ pattern, flags, inputs }, max) {
  let re;
  try {
    re = new RegExp(pattern, flags + "dg");
  } catch {
    return { error: true };
  }
  const unicode = flags.includes("u") || flags.includes("v");
  const insidePair = (s, i) => i > 0 && i < s.length
    && s.charCodeAt(i - 1) >= 0xd800 && s.charCodeAt(i - 1) <= 0xdbff
    && s.charCodeAt(i) >= 0xdc00 && s.charCodeAt(i) <= 0xdfff;
  const results = inputs.map((input) => {
    re.lastIndex = 0;
    const matches = [];
    while (matches.length < max) {
      const m = re.exec(input);
      if (m === null) break;
      const ranges = [...m.indices].map((r) => r ?? null);
      const groups = m.indices.groups
        ? Object.fromEntries(Object.entries(m.indices.groups).map(([k, r]) => [k, r ?? null]))
        : {};
      if (ranges.some((r) => r !== null && (insidePair(input, r[0]) || insidePair(input, r[1])))) {
        return { matches, split: true };
      }
      matches.push({ ranges, groups });
      if (m[0].length === 0) {
        // AdvanceStringIndex, as matchAll does after an empty match.
        const i = re.lastIndex;
        re.lastIndex = unicode && i < input.length ? i + (input.codePointAt(i) > 0xffff ? 2 : 1) : i + 1;
        if (insidePair(input, re.lastIndex)) return { matches, split: true };
      }
    }
    return { matches, split: false };
  });
  return { results };
}

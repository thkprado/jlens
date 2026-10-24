// Builds the corpus of V8's answers that `tests/corpus.rs` checks jlens-regress against.
// Run through `cargo xtask regex-corpus`, which fetches test262 and records regress's tests.
//
//   node generate.mjs <test262 checkout> <test262 commit> <regress cases (JSON lines)> <out>
//
// Every test262 file of `built-ins/RegExp` and `annexB/built-ins/RegExp` runs in a fresh
// realm whose `RegExp.prototype.exec` and `RegExp` constructor record their calls: `exec` is
// what `test`, `match`, `replace`, `split` and the other methods call, so a call records the
// pattern, the flags, the input and `lastIndex`; the constructor records the patterns compiled
// from strings. The negative syntax tests are not run: their regular expression literal is
// recorded. Then each recorded call is replayed in this realm, whose RegExp nobody touched,
// with the `d` flag added to read the ranges of the match and of its groups. The tests'
// assertions do not matter, only the calls they make.

import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const [test262, commit, regressCases, out] = process.argv.slice(2);
if (!out) {
  console.error("usage: node generate.mjs <test262> <commit> <regress cases> <out>");
  process.exit(2);
}

// Inputs longer than this are left out: test262's property escape tests match strings of
// every code point of a property, which regress's own tests cover.
const MAX_INPUT = 4096;

/** Recorded calls by key, in the order first seen; the value is the first source. */
const calls = new Map();
const stats = { files: 0, run: 0, module: 0, parseNegative: 0, longInputs: 0 };

function record(kind, pattern, flags, input, lastIndex, from) {
  if (input !== undefined && input.length > MAX_INPUT) {
    stats.longInputs++;
    return;
  }
  const key = JSON.stringify([kind, pattern, flags, input, lastIndex]);
  if (!calls.has(key)) calls.set(key, { kind, pattern, flags, input, lastIndex, from });
}

// Installed in each test's realm before the harness. It keeps its own references to what it
// calls, since tests replace methods of RegExp.prototype, Function.prototype and others.
const INSTRUMENT = `(function (record) {
  "use strict";
  const R = RegExp, P = R.prototype, exec = P.exec, apply = Reflect.apply;
  const construct = Reflect.construct;
  const getter = (name) => Object.getOwnPropertyDescriptor(P, name).get;
  const source = getter("source");
  const flagGetters = [["g", "global"], ["i", "ignoreCase"], ["m", "multiline"],
    ["s", "dotAll"], ["u", "unicode"], ["v", "unicodeSets"], ["y", "sticky"]]
    .map(([c, name]) => [c, getter(name)]);
  // The getters throw on anything but a RegExp object, so they also tell real ones apart.
  function patternOf(re) {
    if (re === P) return null;
    try {
      let flags = "";
      for (const [c, get] of flagGetters) if (apply(get, re, [])) flags += c;
      return [apply(source, re, []), flags];
    } catch {
      return null;
    }
  }
  P.exec = function exec(string) {
    const pf = patternOf(this);
    if (pf === null) return apply(exec0, this, [string]);
    let s;
    try {
      s = \`\${string}\`;
    } catch {
      return apply(exec0, this, [string]);
    }
    // lastIndex is an own data property: reading it runs no code.
    const sticky = pf[1].includes("g") || pf[1].includes("y");
    const li = sticky ? this.lastIndex : 0;
    // ToLength, as exec applies it.
    if (typeof li === "number") record("exec", pf[0], pf[1], s, Math.min(Math.max(Math.trunc(li) || 0, 0), 2 ** 53 - 1));
    return apply(exec0, this, [s]);
  };
  const exec0 = exec;
  function noteCompile(args) {
    const [p, f] = args;
    if (typeof p === "string" && (f === undefined || typeof f === "string")) {
      record("compile", p, f === undefined ? "" : f);
    }
  }
  const proxy = new Proxy(R, {
    construct(target, args, newTarget) {
      noteCompile(args);
      return construct(target, args, newTarget === proxy ? target : newTarget);
    },
    apply(target, thisArg, args) {
      noteCompile(args);
      return apply(target, thisArg, args);
    },
  });
  globalThis.RegExp = proxy;
})`;

const harness = new Map();
function harnessFile(name) {
  if (!harness.has(name)) {
    harness.set(name, fs.readFileSync(path.join(test262, "harness", name), "utf8"));
  }
  return harness.get(name);
}

function frontmatter(text) {
  const m = text.match(/\/\*---([\s\S]*?)---\*\//);
  const yaml = m ? m[1] : "";
  const list = (key) => {
    const l = yaml.match(new RegExp(`^${key}:\\s*\\[(.*)\\]`, "m"));
    return l ? l[1].split(",").map((s) => s.trim()).filter(Boolean) : [];
  };
  return {
    includes: list("includes"),
    flags: list("flags"),
    parseNegative: /^negative:\s*\n\s+phase:\s*parse/m.test(yaml),
  };
}

function walk(dir) {
  const files = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) files.push(...walk(p));
    else if (entry.name.endsWith(".js") && !entry.name.includes("_FIXTURE")) files.push(p);
  }
  return files;
}

async function runTest(file) {
  const rel = path.relative(path.join(test262, "test"), file).split(path.sep).join("/");
  const text = fs.readFileSync(file, "utf8");
  const meta = frontmatter(text);
  stats.files++;
  if (meta.parseNegative) {
    // The file is not valid JavaScript: what makes it invalid is its literal.
    stats.parseNegative++;
    const body = text.slice(text.indexOf("$DONOTEVALUATE()"));
    for (const line of body.split("\n")) {
      const lit = line.trim().match(/^\/(.+)\/([a-z]*);?$/);
      if (lit) record("compile", lit[1], lit[2], undefined, undefined, rel);
    }
    return;
  }
  if (meta.flags.includes("module")) {
    stats.module++;
    return;
  }
  stats.run++;
  const context = vm.createContext({});
  const recordHere = (kind, pattern, flags, input, lastIndex) =>
    record(kind, pattern, flags, input, lastIndex, rel);
  vm.runInContext(INSTRUMENT, context)(recordHere);
  context.print = () => {};
  context.$262 = {
    global: context,
    evalScript: (code) => vm.runInContext(code, context),
    gc() {},
  };
  let source = "";
  if (!meta.flags.includes("raw")) {
    const includes = ["assert.js", "sta.js", ...meta.includes];
    if (meta.flags.includes("async")) includes.push("doneprintHandle.js");
    source = includes.map(harnessFile).join("\n") + "\n";
  }
  source += text;
  if (meta.flags.includes("onlyStrict")) source = `"use strict";\n${source}`;
  try {
    vm.runInContext(source, context, { filename: rel, timeout: 10000 });
  } catch {
    // A failing assertion only ends the test; what it called is recorded.
  }
  // Lets an asynchronous test's promise jobs run.
  await new Promise((resolve) => setImmediate(resolve));
}

const dirs = ["built-ins/RegExp", "annexB/built-ins/RegExp"];
const files = dirs.flatMap((d) => walk(path.join(test262, "test", d))).sort();
for (const file of files) await runTest(file);

let regress = 0;
if (fs.existsSync(regressCases)) {
  for (const line of fs.readFileSync(regressCases, "utf8").split("\n")) {
    if (!line.trim()) continue;
    const c = JSON.parse(line);
    regress++;
    const from = `regress/tests/${c.test}.rs`;
    if (c.input === undefined) record("compile", c.pattern, c.flags, undefined, undefined, from);
    else record("exec", c.pattern, c.flags + "g", c.input, c.lastIndex, from);
  }
}

// What the engine is asked: only `i`, `m`, `s`, `u` and `v` change a match; `g` and `y` say
// where it starts.
const syntaxFlags = (flags) => [...new Set(flags)].filter((c) => "imsuv".includes(c)).join("");
const validFlags = (flags) => {
  try {
    new RegExp("", flags);
    return true;
  } catch {
    return false;
  }
};

/** A string as JSON keeps it, or its UTF-16 code units when it holds a lone surrogate. */
const str = (s) => (s.isWellFormed() ? s : [...s].flatMap((c) =>
  [...Array(c.length).keys()].map((i) => c.charCodeAt(i))));

const cases = [];
const seen = new Set();
let invalidFlags = 0;
for (const c of calls.values()) {
  const flags = c.kind === "exec"
    ? [...new Set(c.flags)].filter((f) => "gimsuvy".includes(f)).join("")
    : syntaxFlags(c.flags);
  if (!validFlags(flags)) {
    invalidFlags++;
    continue;
  }
  const key = JSON.stringify([c.kind, c.pattern, flags, c.input, c.lastIndex]);
  if (seen.has(key)) continue;
  seen.add(key);
  const out = { from: c.from, pattern: str(c.pattern), flags };
  if (c.kind === "compile") {
    try {
      new RegExp(c.pattern, flags);
      out.compiles = true;
    } catch (e) {
      out.error = e.message.toWellFormed();
    }
  } else {
    let re;
    try {
      re = new RegExp(c.pattern, flags + "d");
    } catch (e) {
      // A pattern compiled from a literal in another realm compiles here too.
      throw new Error(`${c.from}: /${c.pattern}/${flags} does not compile: ${e.message}`);
    }
    const positional = flags.includes("g") || flags.includes("y");
    if (positional) {
      re.lastIndex = c.lastIndex;
      out.lastIndex = c.lastIndex;
    }
    out.input = str(c.input);
    const m = re.exec(c.input);
    if (m === null) {
      out.match = null;
    } else {
      out.match = [...m.indices].map((r) => r ?? null);
      if (m.indices.groups) {
        out.groups = Object.fromEntries(
          Object.entries(m.indices.groups).map(([k, r]) => [k, r ?? null]));
      }
    }
  }
  cases.push(out);
}

// Some tests loop over every code point with one pattern. Of such a group, the corpus keeps
// every answer given at most MAX_SAME times and an even sample of the more common answers.
// regress's property escape tests match every code point of every property, one at a time:
// a few of each are enough to compare V8's Unicode tables with regress's.
const MAX_SAME = 256;
const maxSame = (from) => (from === "regress/tests/unicode_property_escapes.rs" ? 4 : MAX_SAME);
const groups = new Map();
for (const c of cases) {
  const key = JSON.stringify([c.from, c.pattern, c.flags]);
  const answer = JSON.stringify([c.compiles, c.error, c.match, c.groups]);
  if (!groups.has(key)) groups.set(key, new Map());
  const answers = groups.get(key);
  if (!answers.has(answer)) answers.set(answer, []);
  answers.get(answer).push(c);
}
const kept = new Set();
for (const answers of groups.values()) {
  for (const same of answers.values()) {
    const step = Math.max(1, same.length / maxSame(same[0].from));
    for (let i = 0; i < same.length; i += step) kept.add(same[Math.floor(i)]);
  }
}
const sampledOut = cases.length - kept.size;
const corpus = cases.filter((c) => kept.has(c));

const header = {
  generator: "cargo xtask regex-corpus",
  test262: commit,
  node: process.version,
  v8: process.versions.v8,
};
const json = JSON.stringify(header).slice(0, -1) + ',"cases":[\n'
  + corpus.map((c) => JSON.stringify(c)).join(",\n") + "\n]}\n";
fs.writeFileSync(out, json);
console.log(`test262: ${stats.files} files, ${stats.run} run, ${stats.parseNegative} syntax `
  + `literals, ${stats.module} modules skipped; ${stats.longInputs} calls on inputs over `
  + `${MAX_INPUT} code units left out; regress: ${regress} recorded calls; `
  + `${invalidFlags} calls with invalid flags left out; ${sampledOut} left out of large groups`);
console.log(`${corpus.length} cases written to ${out}`);

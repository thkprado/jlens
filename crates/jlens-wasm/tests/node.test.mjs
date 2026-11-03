// Runs suite.mjs in Node.js against the package built by `cargo xtask wasm`: the ES module, and
// jlens.global.js, the classic script with the `.wasm` embedded.

import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { runInThisContext } from "node:vm";

import init, * as jlens from "../pkg/jlens.js";
import { cases } from "./suite.mjs";

await init({ module_or_path: await readFile(new URL("../pkg/jlens_bg.wasm", import.meta.url)) });

// Reads a file of the repository, relative to this directory.
const load = (path) => readFile(new URL(path, import.meta.url), "utf8");

// The classic script, as a page's <script> or a worker's importScripts runs it.
runInThisContext(await load("../pkg/jlens.global.js"), { filename: "jlens.global.js" });
const global = globalThis.jlens;
await global.init();

for (const [name, run] of cases) {
  test(name, () => run(jlens, { virtualTime: false, load }));
}
for (const [name, run] of cases) {
  test(`global: ${name}`, () => run(global, { virtualTime: false, load }));
}

test("global: the exports of the module, with init for its default", () => {
  const expected = Object.keys(jlens).map((k) => (k === "default" ? "init" : k)).sort();
  const actual = Object.keys(global).sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`expected ${expected}, got ${actual}`);
  }
  if (!Object.isFrozen(global)) throw new Error("the global is not frozen");
  if (global.Expression === jlens.Expression) throw new Error("the global shares the module");
});

test("global: imported as a module, initialized synchronously", async () => {
  delete globalThis.jlens;
  await import("../pkg/jlens.global.js");
  const imported = globalThis.jlens;
  globalThis.jlens = global;
  if (!imported || imported === global) throw new Error("importing it set no new global");
  imported.initSync();
  const result = new imported.Expression("$sum(items.(price * qty))").evaluate({
    items: [{ price: 2.5, qty: 4 }, { price: 1, qty: 3 }],
  });
  if (result !== 13) throw new Error(`expected 13, got ${result}`);
});

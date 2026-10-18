// Runs suite.mjs in Node.js against the package built by `cargo xtask wasm`.

import { readFile } from "node:fs/promises";
import { test } from "node:test";

import init, * as jlens from "../pkg/jlens.js";
import { cases } from "./suite.mjs";

await init({ module_or_path: await readFile(new URL("../pkg/jlens_bg.wasm", import.meta.url)) });

// Reads a file of the repository, relative to this directory.
const load = (path) => readFile(new URL(path, import.meta.url), "utf8");

for (const [name, run] of cases) {
  test(name, () => run(jlens, { virtualTime: false, load }));
}

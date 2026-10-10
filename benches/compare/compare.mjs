// Compares jsonata-js with jlens-wasm, both in Node.js, on the workload in ../cases.json.
//
//   cargo xtask wasm && cd benches/compare && npm install && node compare.mjs
//
// For each case it prints the median time per evaluation over several rounds:
// - jsonata-js: `await expression.evaluate(input)` (jsonata 2.x evaluation is asynchronous);
// - jlens-wasm `evaluate`: a JavaScript object in and out (converted through JSON);
// - jlens-wasm `evaluateJson`: JSON text in and out;
// - jlens-wasm `evaluateDocument`: a `Document` converted once, outside the measurement.

import { readFile } from "node:fs/promises";
import jsonata from "jsonata";
import init, { Document, Expression } from "../../crates/jlens-wasm/pkg/jlens.js";

const here = (path) => new URL(path, import.meta.url);
await init({ module_or_path: await readFile(here("../../crates/jlens-wasm/pkg/jlens_bg.wasm")) });

const spec = JSON.parse(await readFile(here("../cases.json"), "utf8"));
const inputText = await readFile(here(`../${spec.input}`), "utf8");
const input = JSON.parse(inputText);
const cases = await Promise.all(
  spec.cases.map(async (c) => ({
    name: c.name,
    source: c.expr ?? (await readFile(here(`../${c.file}`), "utf8")),
  })),
);

/** Median time per call in microseconds, over `rounds` rounds of about 200 ms each. */
async function measure(run, rounds = 7) {
  for (let t = performance.now(); performance.now() - t < 300; ) await run(); // warm up
  const times = [];
  for (let r = 0; r < rounds; r++) {
    let n = 0;
    const start = performance.now();
    while (performance.now() - start < 200) {
      await run();
      n++;
    }
    times.push(((performance.now() - start) * 1000) / n);
  }
  times.sort((a, b) => a - b);
  return times[Math.floor(times.length / 2)];
}

const fmt = (us) => (us >= 1000 ? `${(us / 1000).toFixed(2)} ms` : `${us.toFixed(1)} µs`);

console.log(`Node.js ${process.version}; input: ${spec.input} (${inputText.length} bytes)\n`);
const doc = new Document(input);
console.log("| Case | jsonata-js | jlens-wasm `evaluate` | jlens-wasm `evaluateJson` | jlens-wasm `evaluateDocument` |");
console.log("|---|---:|---:|---:|---:|");
for (const { name, source } of cases) {
  const js = jsonata(source);
  const wasm = new Expression(source);
  // The implementations must agree before their speed is compared.
  const expected = JSON.stringify(await js.evaluate(input));
  const actual = JSON.stringify(wasm.evaluate(input));
  if (expected !== actual) {
    console.log(`| ${name} | results differ: ${expected?.slice(0, 60)} vs ${actual?.slice(0, 60)} | | | |`);
    continue;
  }
  const a = await measure(() => js.evaluate(input));
  const b = await measure(() => wasm.evaluate(input));
  const c = await measure(() => wasm.evaluateJson(inputText));
  const d = await measure(() => wasm.evaluateDocument(doc));
  console.log(`| ${name} | ${fmt(a)} | ${fmt(b)} | ${fmt(c)} | ${fmt(d)} |`);
  wasm.free();
}

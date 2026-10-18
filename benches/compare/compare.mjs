// Compares jsonata-js with jlens-wasm, both in Node.js, on the workload in ../cases.json.
//
//   cargo xtask wasm && cd benches/compare && npm install && node compare.mjs
//
// It first prints the time of the steps that convert the input. Then, for each case, it prints
// the median time per evaluation over several rounds:
// - jsonata-js: `await expression.evaluate(input)` (jsonata 2.x evaluation is asynchronous);
// - jlens-wasm `evaluate`: a JavaScript object in and out (converted through JSON);
// - jlens-wasm `evaluateJson`: JSON text in and out;
// - jlens-wasm `evaluateDocument`: a `Document` converted once, outside the measurement.
//
// Options:
// - `--max-steps N` evaluates jlens-wasm with `maxSteps: N` (ADR 0018);
// - `--pkg DIR` loads jlens-wasm from DIR instead of `crates/jlens-wasm/pkg`, such as the
//   package built by another revision.

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import jsonata from "jsonata";

const { values: args } = parseArgs({ options: { "max-steps": { type: "string" }, pkg: { type: "string" } } });
const here = (path) => new URL(path, import.meta.url);
const pkg = args.pkg ? pathToFileURL(`${args.pkg}/`) : here("../../crates/jlens-wasm/pkg/");
const { default: init, Document, Expression } = await import(new URL("jlens.js", pkg));
await init({ module_or_path: await readFile(new URL("jlens_bg.wasm", pkg)) });
// The bindings and options after the input, or nothing, as a caller without a limit passes.
const limits = args["max-steps"] ? [null, { maxSteps: Number(args["max-steps"]) }] : [];

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

console.log(`Node.js ${process.version}; input: ${spec.input} (${inputText.length} bytes)`);
console.log(`jlens-wasm from ${pkg.pathname}${limits.length ? `, maxSteps ${limits[1].maxSteps}` : ""}\n`);
// The steps of converting the input, which `evaluate` takes on every call.
const compact = JSON.stringify(input);
console.log("| Step | Time |");
console.log("|---|---:|");
console.log(`| \`JSON.stringify\` of the input | ${fmt(await measure(() => JSON.stringify(input)))} |`);
console.log(`| \`Document.fromJson\` of that text, then \`free\` | ${fmt(await measure(() => Document.fromJson(compact).free()))} |`);
console.log(`| \`new Document\` of the input, then \`free\` | ${fmt(await measure(() => new Document(input).free()))} |\n`);

const doc = new Document(input);
console.log("| Case | jsonata-js | jlens-wasm `evaluate` | jlens-wasm `evaluateJson` | jlens-wasm `evaluateDocument` |");
console.log("|---|---:|---:|---:|---:|");
for (const { name, source } of cases) {
  const js = jsonata(source);
  const wasm = new Expression(source);
  // The implementations must agree before their speed is compared.
  const expected = JSON.stringify(await js.evaluate(input));
  const actual = JSON.stringify(wasm.evaluate(input, ...limits));
  if (expected !== actual) {
    console.log(`| ${name} | results differ: ${expected?.slice(0, 60)} vs ${actual?.slice(0, 60)} | | | |`);
    continue;
  }
  const a = await measure(() => js.evaluate(input));
  const b = await measure(() => wasm.evaluate(input, ...limits));
  const c = await measure(() => wasm.evaluateJson(inputText, ...limits));
  const d = await measure(() => wasm.evaluateDocument(doc, ...limits));
  console.log(`| ${name} | ${fmt(a)} | ${fmt(b)} | ${fmt(c)} | ${fmt(d)} |`);
  wasm.free();
}

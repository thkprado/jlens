// Compares jlens-wasm with the other JSONata engines on npm, all in Node.js, on the workload in
// ../cases.json.
//
//   cargo xtask wasm && cd benches/compare && npm install && node compare.mjs
//
// It first prints the time of the steps that convert the input. Then, for each case, it prints
// the median time per evaluation over several rounds:
// - jsonata-js: `await expression.evaluate(input)` (jsonata 2.x evaluation is asynchronous);
// - jsonata2js: `compile(source).evaluate(input)`, the expression translated to JavaScript once;
// - gnata-js: the Go engine gnata built for WebAssembly, an expression compiled once, its input
//   and result through JSON text, as the package's `jsonata(source).evaluate(input)` does
//   (its loader needs a browser, so the module is loaded here as that loader does);
// - jlens-wasm `evaluate`: a JavaScript object in and out (converted through JSON);
// - jlens-wasm `evaluateJson`: JSON text in and out;
// - jlens-wasm `evaluateDocument`: a `Document` converted once, outside the measurement.
//
// Options:
// - `--max-steps N` evaluates jlens-wasm with `maxSteps: N` (ADR 0018);
// - `--pkg DIR` loads jlens-wasm from DIR instead of `crates/jlens-wasm/pkg`, such as the
//   package built by another revision;
// - `--engines LIST` times only the other engines named, of `jsonata-js,jsonata2js,gnata-js`
//   (all by default; `--engines jsonata-js` when comparing two builds). jsonata-js is the
//   reference every engine is checked against, timed or not.
//
// An engine whose result is not jsonata-js's reads `differs` in its cell, and one that throws
// reads `fails`.

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import jsonata from "jsonata";
import jsonata2js from "jsonata2js";

const { values: args } = parseArgs({
  options: { "max-steps": { type: "string" }, pkg: { type: "string" }, engines: { type: "string" } },
});
const others = ["jsonata-js", "jsonata2js", "gnata-js"];
const engines = args.engines ? args.engines.split(",") : others;
for (const e of engines) if (!others.includes(e)) throw new Error(`unknown engine ${e}`);
const here = (path) => new URL(path, import.meta.url);
const pkg = args.pkg ? pathToFileURL(`${args.pkg}/`) : here("../../crates/jlens-wasm/pkg/");
const { default: init, Document, Expression } = await import(new URL("jlens.js", pkg));
await init({ module_or_path: await readFile(new URL("jlens_bg.wasm", pkg)) });
// The bindings and options after the input, or nothing, as a caller without a limit passes.
const limits = args["max-steps"] ? [null, { maxSteps: Number(args["max-steps"]) }] : [];

const spec = JSON.parse(await readFile(here("../cases.json"), "utf8"));
const inputText = await readFile(here(`../${spec.input}`), "utf8");
const input = JSON.parse(inputText);
// A case reads the workload's input, or its own.
const inputs = new Map([[spec.input, { text: inputText, value: input }]]);
const cases = [];
for (const c of spec.cases) {
  const file = c.input ?? spec.input;
  if (!inputs.has(file)) {
    const text = await readFile(here(`../${file}`), "utf8");
    inputs.set(file, { text, value: JSON.parse(text) });
  }
  cases.push({
    name: c.name,
    source: c.expr ?? (await readFile(here(`../${c.file}`), "utf8")),
    input: inputs.get(file),
  });
}

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

// gnata-js, loaded as its browser loader does: `wasm_exec.js` defines `Go`, and running the
// module registers its functions on the global object.
const gnata = engines.includes("gnata-js") ? await loadGnata() : null;
async function loadGnata() {
  const dir = here("node_modules/gnata-js/wasm/");
  await import(new URL("wasm_exec.js", dir));
  const go = new globalThis.Go();
  const { instance } = await WebAssembly.instantiate(await readFile(new URL("gnata.wasm", dir)), go.importObject);
  go.run(instance);
  const ok = (r) => {
    if (r instanceof Error) throw r;
    return r;
  };
  return (source) => {
    const handle = ok(globalThis._gnataCompile(source));
    return (value) => {
      const raw = ok(globalThis._gnataEvalHandle(handle, value != null ? JSON.stringify(value) : ""));
      return raw === "" ? undefined : JSON.parse(raw);
    };
  };
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

for (const input of inputs.values()) input.doc = new Document(input.value);
const jlensColumns = ["jlens-wasm `evaluate`", "jlens-wasm `evaluateJson`", "jlens-wasm `evaluateDocument`"];
console.log(`| Case | ${[...engines, ...jlensColumns].join(" | ")} |`);
console.log(`|---|${[...engines, ...jlensColumns].map(() => "---:").join("|")}|`);
for (const { name, source, input } of cases) {
  const js = jsonata(source);
  const wasm = new Expression(source);
  // The implementations must agree before their speed is compared.
  const expected = JSON.stringify(await js.evaluate(input.value));
  const actual = JSON.stringify(wasm.evaluate(input.value, ...limits));
  if (expected !== actual) {
    console.log(`| ${name} | results differ: ${expected?.slice(0, 60)} vs ${actual?.slice(0, 60)} |`);
    continue;
  }
  const cells = [];
  for (const engine of engines) cells.push(await timeOther(engine, source, input.value, expected, js));
  cells.push(fmt(await measure(() => wasm.evaluate(input.value, ...limits))));
  cells.push(fmt(await measure(() => wasm.evaluateJson(input.text, ...limits))));
  cells.push(fmt(await measure(() => wasm.evaluateDocument(input.doc, ...limits))));
  console.log(`| ${name} | ${cells.join(" | ")} |`);
  wasm.free();
}

/** The cell of another engine: its time, or why it was not timed. */
async function timeOther(engine, source, value, expected, js) {
  let run;
  try {
    if (engine === "jsonata-js") run = () => js.evaluate(value);
    else if (engine === "jsonata2js") {
      const e = jsonata2js.compile(source);
      run = () => e.evaluate(value);
    } else {
      const evaluate = gnata(source);
      run = () => evaluate(value);
    }
    if (JSON.stringify(await run()) !== expected) return "differs";
  } catch {
    return "fails";
  }
  return fmt(await measure(run));
}

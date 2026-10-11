// Runs the cases written by `cargo xtask differential` through jsonata-js, the reference
// implementation, and writes one outcome per case. The xtask runs the same cases in jlens and
// compares the outcomes.
//
//   node differential.mjs <cases.json> <outcomes.json>
//
// An outcome is {"value": <JSON>}, {"undefined": true} or {"error": <code>}. Functions are
// dropped as `JSON.stringify` drops them, as jlens does when it converts a result to JSON.

import { readFileSync, writeFileSync } from "node:fs";
import jsonata from "jsonata";

const [casesPath, outcomesPath] = process.argv.slice(2);
if (!casesPath || !outcomesPath) {
  console.error("usage: node differential.mjs <cases.json> <outcomes.json>");
  process.exit(2);
}
const { inputs, bindings, timeout, depth, cases } = JSON.parse(readFileSync(casesPath, "utf8"));

// jsonata-js lambdas are objects; built-in functions and regexes are JavaScript functions.
const dropFunctions = (_key, value) =>
  typeof value === "function" || value?._jsonata_lambda || value?._jsonata_function
    ? undefined
    : value;

const outcomes = [];
for (const { expr, input } of cases) {
  let outcome;
  try {
    const compiled = jsonata(expr, { timeout, stack: depth });
    // A fresh copy for each case: jsonata-js may modify its input (`@` does).
    const data = input === null ? undefined : structuredClone(inputs[input]);
    const result = await compiled.evaluate(data, structuredClone(bindings));
    const text = JSON.stringify(result, dropFunctions);
    outcome = text === undefined ? { undefined: true } : { value: JSON.parse(text) };
  } catch (err) {
    // An error without a code is a JavaScript exception inside jsonata-js.
    outcome = { error: err?.code ?? `js: ${err?.message ?? err}` };
  }
  outcomes.push(outcome);
}
writeFileSync(outcomesPath, JSON.stringify(outcomes));

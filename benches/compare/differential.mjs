// Runs the cases written by `cargo xtask differential` through jsonata-js, the reference
// implementation, and writes one outcome per case. The xtask runs the same cases in jlens and
// compares the outcomes.
//
//   node differential.mjs <cases.json> <outcomes.json>
//
// An outcome is {"value": <JSON>}, {"undefined": true} or {"error": <code>}. Functions are
// dropped as `JSON.stringify` drops them, as jlens does when it converts a result to JSON. An
// outcome also has "mutated": true when the evaluation changed its input document (a group-by
// on an empty array adds an item to it), which the rest of the expression may see.
//
// The cases run in a worker thread. A case that never finishes, or runs the worker out of
// memory, is a synchronous loop inside jsonata-js, which its timeout cannot interrupt
// (`$match('abc', /b/) ?? 1`): the worker is stopped, the case's outcome is {"hung": true}, and
// a new worker goes on with the next case.

import { readFileSync, writeFileSync } from "node:fs";
import { Worker, isMainThread, parentPort, workerData } from "node:worker_threads";
import jsonata from "jsonata";

// How long a case may run before it is taken as hung: ten times the timeout of the cases.
const HUNG_FACTOR = 10;

if (isMainThread) {
  const [casesPath, outcomesPath] = process.argv.slice(2);
  if (!casesPath || !outcomesPath) {
    console.error("usage: node differential.mjs <cases.json> <outcomes.json>");
    process.exit(2);
  }
  const { cases, timeout } = JSON.parse(readFileSync(casesPath, "utf8"));
  const outcomes = [];
  while (outcomes.length < cases.length) {
    await runFrom(outcomes.length);
  }
  writeFileSync(outcomesPath, JSON.stringify(outcomes));

  // Runs the cases from `start` in a worker, until they are done or one hangs.
  function runFrom(start) {
    return new Promise((resolve) => {
      const worker = new Worker(new URL(import.meta.url), {
        workerData: { casesPath, start },
        resourceLimits: { maxOldGenerationSizeMb: 1024 },
      });
      let timer;
      const watch = () => {
        clearTimeout(timer);
        timer = setTimeout(hung, timeout * HUNG_FACTOR);
      };
      const hung = () => {
        clearTimeout(timer);
        worker.removeAllListeners();
        worker.terminate();
        outcomes.push({ hung: true });
        resolve();
      };
      worker.on("message", (outcome) => {
        outcomes.push(outcome);
        watch();
      });
      // Out of memory.
      worker.on("error", hung);
      worker.on("exit", () => {
        if (outcomes.length < cases.length) {
          hung();
        } else {
          clearTimeout(timer);
          resolve();
        }
      });
      watch();
    });
  }
} else {
  const { casesPath, start } = workerData;
  const { inputs, bindings, timeout, depth, cases } = JSON.parse(readFileSync(casesPath, "utf8"));

  // jsonata-js lambdas are objects; built-in functions and regexes are JavaScript functions.
  const dropFunctions = (_key, value) =>
    typeof value === "function" || value?._jsonata_lambda || value?._jsonata_function
      ? undefined
      : value;

  for (const { expr, input } of cases.slice(start)) {
    let outcome;
    // A fresh copy for each case: jsonata-js may modify its input (`@` does).
    const data = input === null ? undefined : structuredClone(inputs[input]);
    try {
      const compiled = jsonata(expr, { timeout, stack: depth });
      const result = await compiled.evaluate(data, structuredClone(bindings));
      const text = JSON.stringify(result, dropFunctions);
      outcome = text === undefined ? { undefined: true } : { value: JSON.parse(text) };
    } catch (err) {
      // An error without a code is a JavaScript exception inside jsonata-js.
      outcome = { error: err?.code ?? `js: ${err?.message ?? err}` };
    }
    if (input !== null && JSON.stringify(data) !== JSON.stringify(inputs[input])) {
      outcome.mutated = true;
    }
    parentPort.postMessage(outcome);
  }
}

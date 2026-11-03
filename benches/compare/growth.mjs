// The cases of `../growth.json` in jsonata-js, in the format of `cargo xtask growth`, whose
// `--write-inputs` leaves the inputs this script reads in `target/growth`.
//
//   cargo xtask growth --write-inputs --case '$count' && node growth.mjs [--sizes N,N] [text]...
//
// Only the cases whose name holds one of the texts run, or all of them. Each size takes the
// fastest of a few runs, and a case that takes longer than `BUDGET` ms stops there.

import { readFileSync } from "node:fs";
import jsonata from "jsonata";

const BUDGET = 4000;
const MIN_TIME = 40;
const MIN_RUNS = 3;

const args = process.argv.slice(2);
let sizes = [1000, 4000, 16000, 64000];
const wanted = [];
while (args.length) {
  const arg = args.shift();
  if (arg === "--sizes") sizes = args.shift().split(",").map(Number);
  else wanted.push(arg);
}

const root = new URL("../../", import.meta.url);
const spec = JSON.parse(readFileSync(new URL("benches/growth.json", root)));
const inputs = sizes.map((n) =>
  JSON.parse(readFileSync(new URL(`target/growth/input-${n}.json`, root))),
);

function source(c, n) {
  if (c.repeat === undefined) return c.expr;
  const times = c.nest ? Math.min(n, 900) : n;
  let text = c.prefix;
  for (let i = 0; i < times; i++) text += c.repeat.replaceAll("{i}", i);
  return text + c.suffix;
}

async function measure(c, n, input) {
  const text = source(c, n);
  const built = c.repeat !== undefined;
  const expression = jsonata(text);
  let fastest = Infinity;
  let spent = 0;
  for (let runs = 0; runs < MIN_RUNS || spent < MIN_TIME; runs++) {
    const start = performance.now();
    await (built ? jsonata(text) : expression).evaluate(input);
    const time = performance.now() - start;
    fastest = Math.min(fastest, time);
    spent += time;
    if (time > BUDGET / 4) break;
  }
  return fastest;
}

console.log(`case\tmode\t${sizes.join("\t")}\texponents`);
for (const c of spec.cases) {
  if (wanted.length && !wanted.some((w) => c.name.includes(w))) continue;
  const times = [];
  let stop = "";
  for (let i = 0; i < sizes.length; i++) {
    try {
      const time = await measure(c, sizes[i], inputs[i]);
      times.push(time);
      if (time > BUDGET) break;
    } catch (e) {
      stop = ` stopped by ${e.code ?? e.message}`;
      break;
    }
  }
  const exponents = times
    .slice(1)
    .map((t, i) => (Math.log(t / times[i]) / Math.log(sizes[i + 1] / sizes[i])).toFixed(2));
  const shown = times.map((t) => (t * 1000).toFixed(1));
  while (shown.length < sizes.length) shown.push("-");
  console.log(`${c.name}\tjsonata-js\t${shown.join("\t")}\t${exponents.join(" ")}${stop}`);
}

// jlens-regress: V8's times for the cases of `engine.rs`, for reference. Run it with
// `node crates/jlens-regress/benches/engine.mjs [case...]`. Each case finds every match in its
// text with `exec` and the flag `g`, as jsonata-js does. A compilation is measured twice:
// `new RegExp`, which only parses the pattern, and `new RegExp` and a first `exec`, where V8
// compiles it to machine code. Both get a pattern V8 has not seen (the case's, with an
// alternative `|<n>` added), as V8 keeps compiled patterns by their source.

import { readFileSync } from 'node:fs';

const ROUNDS = 31;
const SAMPLE_NS = 5e6;

const cases = JSON.parse(readFileSync(new URL('engine.json', import.meta.url), 'utf8'));
const filters = process.argv.slice(2);
const selected = (name) => filters.length === 0 || filters.some((f) => name.includes(f));

function findAll(re, text) {
  re.lastIndex = 0;
  let count = 0;
  let m;
  while ((m = re.exec(text)) !== null) {
    count++;
    if (m[0].length === 0) re.lastIndex += text.codePointAt(re.lastIndex) > 0xffff ? 2 : 1;
  }
  return count;
}

let sink = 0;

// The median time of one call of `f`, in nanoseconds.
function measure(f) {
  let iterations = 1;
  for (;;) {
    const start = process.hrtime.bigint();
    for (let i = 0; i < iterations; i++) sink ^= f();
    if (Number(process.hrtime.bigint() - start) >= SAMPLE_NS) break;
    iterations *= 2;
  }
  const samples = [];
  for (let round = 0; round < ROUNDS; round++) {
    const start = process.hrtime.bigint();
    for (let i = 0; i < iterations; i++) sink ^= f();
    samples.push(Number(process.hrtime.bigint() - start) / iterations);
  }
  samples.sort((a, b) => a - b);
  return samples[ROUNDS >> 1];
}

function formatTime(ns) {
  if (ns >= 1e6) return `${(ns / 1e6).toFixed(2)} ms`;
  if (ns >= 1e3) return `${(ns / 1e3).toFixed(1)} µs`;
  return `${ns.toFixed(0)} ns`;
}

console.log(`V8 ${process.versions.v8}, Node.js ${process.version}`);
console.log(`${'case'.padEnd(32)} ${'matches'.padStart(7)} ${'exec'.padStart(12)}`);
for (const { name, pattern, flags, text } of cases.matches) {
  if (!selected(name)) continue;
  const { repeat, times } = cases.texts[text];
  const subject = repeat.repeat(times);
  const re = new RegExp(pattern, `${flags}g`);
  const matches = findAll(re, subject);
  const time = measure(() => findAll(re, subject));
  console.log(`${name.padEnd(32)} ${String(matches).padStart(7)} ${formatTime(time).padStart(12)}`);
}
console.log(`${''.padEnd(32)} ${''.padStart(7)} ${'new RegExp'.padStart(12)} ${'and exec'.padStart(12)}`);
let unique = 0;
for (const { name, pattern, flags } of cases.compiles) {
  if (!selected(name)) continue;
  const parse = measure(() => new RegExp(`${pattern}|${unique++}`, `${flags}g`).flags.length);
  const compile = measure(() => new RegExp(`${pattern}|${unique++}`, `${flags}g`).exec('') === null);
  console.log(
    `${name.padEnd(32)} ${''.padStart(7)} ${formatTime(parse).padStart(12)} ${formatTime(compile).padStart(12)}`,
  );
}

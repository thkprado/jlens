// Runs every `jsonata-example` block of the book through jsonata-js, the reference
// implementation, and reports the examples whose documented result it does not produce.
// `cargo xtask doc-check` checks the same examples against jlens. Examples marked `differs:`
// document a known difference and are skipped.
//
//   cd benches/compare && npm install && node doc-examples.mjs

import { readFileSync, readdirSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join, relative } from "node:path";
import jsonata from "jsonata";

const docs = fileURLToPath(new URL("../../docs", import.meta.url));
const pages = [];
const walk = (dir) =>
  readdirSync(dir).forEach((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path);
    else if (path.endsWith(".md")) pages.push(path);
  });
walk(join(docs, "src"));

let checked = 0;
let skipped = 0;
let failed = 0;
for (const page of pages.sort()) {
  const lines = readFileSync(page, "utf8").split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trim() !== "```jsonata-example") continue;
    const line = i + 1;
    const indent = lines[i].length - lines[i].trimStart().length;
    const body = [];
    for (i++; !lines[i].trim().startsWith("```"); i++) {
      const own = lines[i].length - lines[i].trimStart().length;
      body.push(lines[i].slice(Math.min(indent, own)));
    }
    let input;
    let bindings = {};
    let differs = false;
    while (body.length && /^(input|bindings|differs):/.test(body[0])) {
      const header = body.shift();
      const value = header.slice(header.indexOf(":") + 1).trim();
      if (header.startsWith("differs:")) differs = true;
      else if (header.startsWith("bindings:")) bindings = JSON.parse(value);
      else if (/^[[{"\d-]/.test(value)) input = JSON.parse(value);
      else input = JSON.parse(readFileSync(join(docs, "examples", value), "utf8"));
    }
    if (differs) {
      skipped++;
      continue;
    }
    const split = body.findIndex((l) => l.startsWith("=>"));
    const expression = body.slice(0, split).join("\n");
    const expected = body.slice(split).join("\n").slice(2).trim();
    let actual;
    try {
      actual = await jsonata(expression).evaluate(input, bindings);
    } catch (err) {
      actual = { error: err.code };
    }
    let ok;
    if (expected === "*") ok = !actual?.error;
    else if (expected === "undefined") ok = actual === undefined;
    else if (expected.startsWith("error ")) ok = actual?.error === expected.slice(6).trim();
    else ok = JSON.stringify(actual) === JSON.stringify(JSON.parse(expected));
    checked++;
    if (!ok) {
      failed++;
      console.log(`${relative(docs, page)}:${line}\n  ${expression.replace(/\n/g, "\n  ")}`);
      console.log(`  documented: ${expected}\n  jsonata-js: ${JSON.stringify(actual)}\n`);
    }
  }
}
console.log(`${checked} examples checked against jsonata-js, ${failed} differ, ${skipped} skipped (differs:)`);
process.exitCode = failed ? 1 : 0;

// Tests of the TextMate grammar, with the engine VS Code uses (vscode-textmate).
//
// - Snapshots: each `fixtures/*.jsonata` file is tokenized and compared with its `.snap` file.
//   Together the fixtures use every scope of the grammar. After changing the grammar, run
//   `npm test -- --update` (or set UPDATE_SNAPSHOTS=1) and review the diff of the `.snap` files.
// - Corpus: every expression of the JSONata test suite and every example of the book is
//   tokenized. No character other than whitespace may be left without a scope, no string,
//   comment or regular expression may stay open at the end, and the grammar must find the
//   regular expressions that jsonata-js's parser finds.
//
//   npm ci && npm test

import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import vsctm from "vscode-textmate";
import oniguruma from "vscode-oniguruma";
import jsonata from "jsonata";

const require = createRequire(import.meta.url);
const here = fileURLToPath(new URL(".", import.meta.url));
const repo = join(here, "../../..");
const grammarPath = join(here, "../syntaxes/jsonata.tmLanguage.json");
const update = process.argv.includes("--update") || process.env.UPDATE_SNAPSHOTS === "1";

await oniguruma.loadWASM(readFileSync(require.resolve("vscode-oniguruma/release/onig.wasm")));
const registry = new vsctm.Registry({
  onigLib: Promise.resolve({
    createOnigScanner: (patterns) => new oniguruma.OnigScanner(patterns),
    createOnigString: (text) => new oniguruma.OnigString(text),
  }),
  loadGrammar: async (scope) =>
    scope === "source.jsonata"
      ? vsctm.parseRawGrammar(readFileSync(grammarPath, "utf8"), grammarPath)
      : null,
});
const grammar = await registry.loadGrammar("source.jsonata");

/** Tokenizes `text` line by line; returns the tokens of each line and the final rule stack. */
function tokenize(text) {
  let stack = vsctm.INITIAL;
  const lines = text.split(/\r?\n/).map((line) => {
    const result = grammar.tokenizeLine(line, stack);
    stack = result.ruleStack;
    const tokens = result.tokens
      .map((t) => ({
        start: t.startIndex,
        end: Math.min(t.endIndex, line.length),
        scopes: t.scopes.slice(1),
      }))
      .filter((t) => t.start < t.end);
    return { line, tokens };
  });
  return { lines, stack };
}

/** The snapshot format: each line, then one line per token under it with its scopes. */
function snapshot(text) {
  const out = [];
  for (const { line, tokens } of tokenize(text.replace(/\r?\n$/, "")).lines) {
    out.push(`>${line}`);
    for (const t of tokens) {
      if (!t.scopes.length && !line.slice(t.start, t.end).trim()) continue;
      const marker = " ".repeat(t.start) + "^".repeat(t.end - t.start);
      out.push(`#${marker} ${t.scopes.join(" ") || "-"}`);
    }
  }
  return out.join("\n") + "\n";
}

/** Every scope name the grammar can assign. */
function grammarScopes() {
  const scopes = new Set();
  const walk = (node) => {
    if (Array.isArray(node)) return node.forEach(walk);
    if (!node || typeof node !== "object") return;
    for (const [key, value] of Object.entries(node)) {
      if ((key === "name" || key === "contentName") && typeof value === "string") {
        value.split(" ").forEach((s) => scopes.add(s));
      } else walk(value);
    }
  };
  const raw = JSON.parse(readFileSync(grammarPath, "utf8"));
  walk(raw.repository);
  return scopes;
}

const fixtures = join(here, "fixtures");

test("the fixtures match their snapshots", () => {
  const stale = [];
  for (const name of readdirSync(fixtures).filter((n) => n.endsWith(".jsonata")).sort()) {
    const actual = snapshot(readFileSync(join(fixtures, name), "utf8"));
    const snap = join(fixtures, `${name}.snap`);
    if (update) writeFileSync(snap, actual);
    else if (!existsSync(snap) || readFileSync(snap, "utf8").replace(/\r\n/g, "\n") !== actual)
      stale.push(name);
  }
  assert.deepEqual(stale, [], "snapshots differ; run `npm test -- --update` and review the diff");
});

test("the fixtures use every scope of the grammar", () => {
  const used = new Set();
  for (const name of readdirSync(fixtures).filter((n) => n.endsWith(".jsonata"))) {
    for (const { tokens } of tokenize(readFileSync(join(fixtures, name), "utf8")).lines) {
      tokens.forEach((t) => t.scopes.forEach((s) => s.split(" ").forEach((x) => used.add(x))));
    }
  }
  const unused = [...grammarScopes()].filter((s) => !used.has(s)).sort();
  assert.deepEqual(unused, [], "scopes no fixture exercises");
});

/** Expressions of the test suite and the book, with the error code they expect, if any. */
function corpus() {
  const items = [];
  const groups = join(repo, "tests/jsonata-suite/groups");
  for (const group of readdirSync(groups).sort()) {
    const dir = join(groups, group);
    for (const file of readdirSync(dir).filter((n) => n.endsWith(".json")).sort()) {
      const spec = JSON.parse(readFileSync(join(dir, file), "utf8"));
      for (const [i, c] of (Array.isArray(spec) ? spec : [spec]).entries()) {
        const expr = c["expr-file"] ? readFileSync(join(dir, c["expr-file"]), "utf8") : c.expr;
        if (typeof expr !== "string") continue;
        const code = c.code ?? c.error?.code;
        items.push({ where: `${group}/${file}#${i}`, expr, code });
      }
    }
  }
  const docs = join(repo, "docs/src");
  const pages = [];
  const walk = (dir) =>
    readdirSync(dir).forEach((name) => {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path);
      else if (path.endsWith(".md")) pages.push(path);
    });
  walk(docs);
  for (const page of pages.sort()) {
    const lines = readFileSync(page, "utf8").split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const fence = lines[i].trim();
      if (fence !== "```jsonata-example" && fence !== "```jsonata") continue;
      const where = `${relative(repo, page).replaceAll("\\", "/")}:${i + 1}`;
      const indent = lines[i].length - lines[i].trimStart().length;
      const body = [];
      for (i++; !lines[i].trim().startsWith("```"); i++) {
        const own = lines[i].length - lines[i].trimStart().length;
        body.push(lines[i].slice(Math.min(indent, own)));
      }
      if (fence === "```jsonata") {
        items.push({ where, expr: body.join("\n") });
        continue;
      }
      while (body.length && /^(input|bindings|differs):/.test(body[0])) body.shift();
      const split = body.findIndex((l) => l.startsWith("=>"));
      const result = body.slice(split).join("\n").slice(2).trim();
      const code = result.startsWith("error ") ? result.slice(6).trim() : undefined;
      items.push({ where, expr: body.slice(0, split).join("\n"), code });
    }
  }
  return items;
}

// Errors of the lexer: an expression that expects one may leave a string, a comment or a
// regular expression open.
const LEXER_ERRORS = new Set(["S0101", "S0102", "S0103", "S0104", "S0105", "S0106", "S0301", "S0302"]);

/** How many regular expressions jsonata-js's syntax tree holds; undefined if it does not parse. */
function astRegexes(expr) {
  let ast;
  try {
    ast = jsonata(expr).ast();
  } catch {
    return undefined;
  }
  let found = 0;
  const seen = new Set();
  const walk = (node) => {
    if (!node || typeof node !== "object" || seen.has(node)) return;
    seen.add(node);
    if (node.type === "regex" && node.value instanceof RegExp) found++;
    Object.values(node).forEach(walk);
  };
  walk(ast);
  return found;
}

test("the suite's expressions and the book's examples are tokenized", () => {
  const problems = [];
  const items = corpus();
  assert.ok(items.length > 1500, `only ${items.length} expressions found`);
  for (const { where, expr, code } of items) {
    const { lines, stack } = tokenize(expr);
    if (!LEXER_ERRORS.has(code) && stack.depth !== 1) {
      problems.push(`${where}: a string, comment or regular expression stays open\n  ${expr}`);
      continue;
    }
    let regexes = 0;
    for (const { line, tokens } of lines) {
      for (const t of tokens) {
        const text = line.slice(t.start, t.end);
        if (!t.scopes.length && text.trim()) {
          problems.push(`${where}: no scope for ${JSON.stringify(text.trim())}\n  ${expr}`);
        }
        if (t.scopes.some((s) => s.includes("punctuation.definition.string.begin")) &&
            t.scopes.some((s) => s.includes("string.regexp"))) regexes++;
      }
    }
    if (LEXER_ERRORS.has(code)) continue;
    const expected = astRegexes(expr);
    if (expected !== undefined && expected !== regexes) {
      problems.push(`${where}: ${regexes} regular expressions, jsonata-js finds ${expected}\n  ${expr}`);
    }
  }
  assert.deepEqual(problems, []);
});

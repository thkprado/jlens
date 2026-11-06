// The jlens web editor: a JSONata editor with completion and diagnostics driven by the input
// document, live evaluation, and an explanation and audit tree of every result.
//
// Serve the repository root (`cargo xtask serve`) after building the package (`cargo xtask
// wasm`), then open /examples/web-editor/. The import map of index.html maps `jlens-wasm` to the
// package; `cargo xtask site` rewrites that entry for the site. Evaluation runs in a Web Worker
// (worker.js), so that a slow expression never blocks typing.

import { closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { bracketMatching } from "@codemirror/language";
import { lintKeymap } from "@codemirror/lint";
import { Compartment } from "@codemirror/state";
import { EditorView, highlightActiveLine, keymap, lineNumbers } from "@codemirror/view";

import init, { LanguageService, inferSchema, missingOverridePaths, version } from "jlens-wasm";
import { renderAudit, renderExplanation, spansOf } from "./audit-view.js";
import { domainValues } from "./domains.js";
import { hostDeclarations } from "./host-functions.js";
import { jlensExtensions } from "./jlens-codemirror.js";
import { samples } from "./samples.js";
import { MAX_ADDRESS, decodeShare, encodeShare } from "./share.js";

const $ = (id) => document.getElementById(id);

await init();
$("status").textContent = `jlens ${version()} (WebAssembly)`;

/** The state in the address: a shared one (`#s=...`), the example it names, or `null`. */
async function fromAddress() {
  try {
    const shared = await decodeShare(location.hash);
    if (shared) return { id: "", ...shared };
  } catch (err) {
    $("share-status").textContent = `This shared address is damaged: ${err.message}`;
  }
  return samples.find((s) => `#${s.id}` === location.hash) ?? null;
}

const initial = (await fromAddress()) ?? samples[0];
let input = initial.input;
let bindings = initial.bindings;
let overrides = initial.overrides ?? {};
let service = new LanguageService();
let schema = null;

/**
 * Rebuilds the language service from the input document, the bindings and the schema overrides.
 * Overrides that `inferSchema` rejects are shown as the panel's error, and left out.
 */
function updateService() {
  try {
    schema = inferSchema([input], overrides);
    $("input-error").textContent = "";
    showMissingPaths(missingOverridePaths([input], overrides));
  } catch (err) {
    showMissingPaths([]);
    $("input-error").textContent = err.message; // "invalid overrides: ..."
    schema = inferSchema([input]);
  }
  const variables = Object.fromEntries(
    Object.entries(bindings).map(([name, value]) => [name, inferSchema([value])]),
  );
  service.free();
  service = new LanguageService(schema, variables, hostDeclarations);
  $("schema").textContent = JSON.stringify(schema, null, 2);
}

/**
 * Warns about the override paths the input lacks: they are declared as new fields, which is also
 * what a misspelt path does. Warnings do not stop sharing.
 */
function showMissingPaths(missing) {
  $("overrides-warnings").replaceChildren(
    ...missing.map(({ path, suggestion }) => {
      const item = document.createElement("li");
      item.textContent = suggestion
        ? `${path} is not in the input; did you mean ${suggestion}?`
        : `${path} is not in the input; declared as a new field.`;
      return item;
    }),
  );
}

function select(span) {
  editor.dispatch({ selection: { anchor: span.start, head: span.end }, scrollIntoView: true });
  editor.focus();
}

// The worker that evaluates, started on first use. Import maps do not apply to workers, so it
// receives the address of the package in its own.
const workerUrl = new URL("worker.js", import.meta.url);
workerUrl.searchParams.set("jlens", import.meta.resolve("jlens-wasm"));
let worker = null;
let busy = false;
let latest = 0;

/** Shows an evaluation's reply in the result, explanation and audit panels. */
function show(reply) {
  const { error } = reply;
  if (error) {
    $("result").textContent = error.code ? `${error.code}: ${error.message}` : error.message;
    $("result").className = "error";
  } else {
    $("result").textContent =
      reply.result === undefined ? "(undefined)" : JSON.stringify(reply.result, null, 2);
    $("result").className = "";
  }
  if (reply.audit) {
    renderExplanation($("explanation"), reply.explanation, select, spansOf(reply.audit));
    renderAudit($("audit"), reply.audit, select);
    $("status").textContent = `jlens ${version()} · evaluated in ${reply.elapsed.toFixed(1)} ms`;
  }
  document.body.dataset.ready = "true";
}

/**
 * Evaluates the expression in the worker. An evaluation still running is dropped: its worker is
 * terminated and replaced, so the reply always belongs to the latest expression.
 */
function evaluate() {
  if (busy) {
    worker.terminate();
    worker = null;
  }
  if (!worker) {
    const current = new Worker(workerUrl, { type: "module" });
    current.onmessage = ({ data }) => {
      const reply = JSON.parse(data); // JSON text: see worker.js
      if (current !== worker || reply.id !== latest) return;
      busy = false;
      show(reply);
    };
    current.onerror = (event) => {
      if (current !== worker) return;
      worker = null;
      busy = false;
      show({ error: { message: event.message || "The evaluation worker failed to start." } });
    };
    worker = current;
  }
  busy = true;
  worker.postMessage({
    id: ++latest,
    source: editor.state.doc.toString(),
    input,
    bindings,
    level: $("level").value,
  });
}

let timer;
function schedule() {
  $("share-status").textContent = "";
  clearTimeout(timer);
  timer = setTimeout(evaluate, 150);
}

// The editor's colors come from the page's CSS variables, so they follow its light and dark
// modes. A CodeMirror theme takes precedence over CodeMirror's base theme, which a plain
// stylesheet does not: the base theme would otherwise draw a black cursor on the dark background.
const editorTheme = EditorView.theme({
  "&": { minHeight: "320px", maxHeight: "60vh", backgroundColor: "var(--panel)", color: "var(--text)" },
  ".cm-scroller": {
    fontFamily: 'ui-monospace, "Cascadia Code", Menlo, Consolas, monospace',
    fontSize: "13px",
    lineHeight: "1.5",
  },
  ".cm-content": { caretColor: "var(--caret)" },
  ".cm-cursor, .cm-dropCursor": { borderLeft: "2px solid var(--caret)" },
  ".cm-gutters": {
    backgroundColor: "var(--code-bg)",
    color: "var(--muted)",
    borderRight: "1px solid var(--border)",
  },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--accent) 8%, transparent)" },
  ".cm-activeLineGutter": {
    backgroundColor: "color-mix(in srgb, var(--accent) 16%, transparent)",
    color: "var(--text)",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
    backgroundColor: "color-mix(in srgb, var(--accent) 30%, transparent)",
  },
  "&.cm-focused": { outline: "none" },
});

// Tells CodeMirror whether the page is dark, so that its own defaults (tooltips, panels) match.
const darkScheme = window.matchMedia("(prefers-color-scheme: dark)");
const colorScheme = new Compartment();
const schemeFlag = (dark) => EditorView.theme({}, { dark });

const editor = new EditorView({
  doc: initial.expression,
  parent: $("expression"),
  extensions: [
    lineNumbers(),
    history(),
    highlightActiveLine(),
    bracketMatching(),
    closeBrackets(),
    keymap.of([
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...historyKeymap,
      ...completionKeymap,
      ...lintKeymap,
    ]),
    editorTheme,
    colorScheme.of(schemeFlag(darkScheme.matches)),
    EditorView.lineWrapping,
    jlensExtensions(() => service, $("signature"), { values: domainValues }),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) schedule();
    }),
  ],
});

darkScheme.addEventListener("change", (event) => {
  editor.dispatch({ effects: colorScheme.reconfigure(schemeFlag(event.matches)) });
});

/** Parses a text area that holds a JSON object; an empty one is `{}`. */
function readObject(id, what) {
  let parsed;
  try {
    parsed = $(id).value.trim() ? JSON.parse($(id).value) : {};
  } catch (err) {
    throw new Error(`${what}: ${err.message}`);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`${what} must be a JSON object.`);
  }
  return parsed;
}

/** Reads the input document, the bindings and the schema overrides from their text areas. */
function readInputs() {
  try {
    input = JSON.parse($("input").value);
    bindings = readObject("bindings", "Bindings");
    overrides = readObject("overrides", "Schema overrides");
  } catch (err) {
    $("input-error").textContent = err.message;
    return;
  }
  updateService();
  schedule();
}

/** Shows the input document, the bindings and the schema overrides in their text areas. */
function showInputs() {
  $("input").value = JSON.stringify(input, null, 2);
  $("bindings").value = JSON.stringify(bindings, null, 2);
  $("overrides").value = JSON.stringify(overrides, null, 2);
}

/** Replaces the expression, the input, the bindings and the overrides with an example's. */
function loadSample(sample) {
  input = sample.input;
  bindings = sample.bindings;
  overrides = sample.overrides ?? {};
  showInputs();
  updateService();
  editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: sample.expression } });
}

// A shared state is no example: the selector shows "Shared" until another is chosen.
$("sample").append(new Option("Shared", ""));
$("sample").options[0].hidden = true;
for (const sample of samples) $("sample").append(new Option(sample.label, sample.id));
$("sample").value = initial.id;
$("sample").addEventListener("change", () => {
  const sample = samples.find((s) => s.id === $("sample").value);
  if (!sample) return;
  window.history.replaceState(null, "", `#${sample.id}`); // `history` is CodeMirror's here
  loadSample(sample);
});

showInputs();
for (const id of ["input", "bindings", "overrides"]) {
  $(id).addEventListener("input", () => {
    clearTimeout(timer);
    timer = setTimeout(readInputs, 300);
  });
}
$("level").addEventListener("change", evaluate);

// An address pasted into this tab changes only the fragment: nothing reloads.
window.addEventListener("hashchange", async () => {
  $("share-status").textContent = "";
  const state = await fromAddress();
  if (!state) return;
  $("sample").value = state.id;
  loadSample(state);
});

/** Puts the expression, the input, the bindings and the overrides in the address, and copies it. */
async function share() {
  const status = $("share-status");
  if ($("input-error").textContent) {
    status.textContent = "Correct the input document, the bindings or the schema overrides first.";
    return;
  }
  const expression = editor.state.doc.toString();
  const fragment = await encodeShare({ expression, input, bindings, overrides });
  const address = new URL(fragment, location.href).href;
  if (address.length > MAX_ADDRESS) {
    const kb = (address.length / 1000).toFixed(1);
    status.textContent = `Too large to share: the address would be ${kb} KB, and some tools cut addresses beyond 8 KB.`;
    return;
  }
  window.history.replaceState(null, "", fragment);
  $("sample").value = "";
  try {
    await navigator.clipboard.writeText(address);
    status.textContent = "Address copied.";
  } catch {
    status.textContent = "The address bar holds the shared address.";
  }
}
$("share").addEventListener("click", share);

for (const tab of document.querySelectorAll("[role=tab]")) {
  tab.addEventListener("click", () => {
    for (const other of document.querySelectorAll("[role=tab]")) {
      const selected = other === tab;
      other.setAttribute("aria-selected", String(selected));
      $(`tab-${other.dataset.tab}`).hidden = !selected;
    }
  });
}

updateService();
evaluate();

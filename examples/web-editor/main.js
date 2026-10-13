// The jlens web editor: a JSONata editor with completion and diagnostics driven by the input
// document, live evaluation, and an explanation and audit tree of every result.
//
// Serve the repository root (`cargo xtask serve`) after building the package (`cargo xtask
// wasm`), then open /examples/web-editor/. The import map of index.html maps `jlens-wasm` to the
// package; `cargo xtask site` rewrites that entry for the site.

import { closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { bracketMatching } from "@codemirror/language";
import { lintKeymap } from "@codemirror/lint";
import { Compartment } from "@codemirror/state";
import { EditorView, highlightActiveLine, keymap, lineNumbers } from "@codemirror/view";

import init, {
  Expression,
  LanguageService,
  inferSchema,
  version,
} from "jlens-wasm";
import { renderAudit, renderExplanation, spansOf } from "./audit-view.js";
import { jlensExtensions } from "./jlens-codemirror.js";
import { samples } from "./samples.js";

const $ = (id) => document.getElementById(id);

await init();
$("status").textContent = `jlens ${version()} (WebAssembly)`;

// The example named in the address (`#invoice`), or the first one.
const initial = samples.find((s) => `#${s.id}` === location.hash) ?? samples[0];
let input = initial.input;
let bindings = initial.bindings;
let service = new LanguageService();
let schema = null;

/** Rebuilds the language service from the input document and the bindings. */
function updateService() {
  schema = inferSchema([input]);
  const variables = Object.fromEntries(
    Object.entries(bindings).map(([name, value]) => [name, inferSchema([value])]),
  );
  service.free();
  service = new LanguageService(schema, variables);
  $("schema").textContent = JSON.stringify(schema, null, 2);
}

function select(span) {
  editor.dispatch({ selection: { anchor: span.start, head: span.end }, scrollIntoView: true });
  editor.focus();
}

/** Evaluates the expression and refreshes the result, explanation and audit panels. */
function evaluate() {
  const source = editor.state.doc.toString();
  let expr;
  try {
    expr = new Expression(source);
  } catch (err) {
    $("result").textContent = `${err.code}: ${err.message}`;
    $("result").className = "error";
    return;
  }
  try {
    const started = performance.now();
    const outcome = expr.evaluateWithAudit(input, bindings, {
      level: $("level").value,
      timeout: 2000,
    });
    const elapsed = performance.now() - started;
    if (outcome.error) {
      $("result").textContent = `${outcome.error.code}: ${outcome.error.message}`;
      $("result").className = "error";
    } else {
      $("result").textContent =
        outcome.result === undefined ? "(undefined)" : JSON.stringify(outcome.result, null, 2);
      $("result").className = "";
    }
    renderExplanation($("explanation"), outcome.explanation, select, spansOf(outcome.audit));
    renderAudit($("audit"), outcome.audit, select);
    $("status").textContent = `jlens ${version()} · evaluated in ${elapsed.toFixed(1)} ms`;
  } finally {
    expr.free();
  }
}

let timer;
function schedule() {
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
    jlensExtensions(() => service, $("signature")),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) schedule();
    }),
  ],
});

darkScheme.addEventListener("change", (event) => {
  editor.dispatch({ effects: colorScheme.reconfigure(schemeFlag(event.matches)) });
});

/** Reads the input document and the bindings from their text areas. */
function readInputs() {
  try {
    input = JSON.parse($("input").value);
    const parsed = $("bindings").value.trim() ? JSON.parse($("bindings").value) : {};
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
      throw new Error("Bindings must be a JSON object.");
    }
    bindings = parsed;
    $("input-error").textContent = "";
  } catch (err) {
    $("input-error").textContent = err.message;
    return;
  }
  updateService();
  schedule();
}

/** Replaces the expression, the input document and the bindings with an example's. */
function loadSample(sample) {
  input = sample.input;
  bindings = sample.bindings;
  $("input").value = JSON.stringify(sample.input, null, 2);
  $("bindings").value = JSON.stringify(sample.bindings, null, 2);
  $("input-error").textContent = "";
  updateService();
  editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: sample.expression } });
}

for (const sample of samples) $("sample").append(new Option(sample.label, sample.id));
$("sample").value = initial.id;
$("sample").addEventListener("change", () => {
  const sample = samples.find((s) => s.id === $("sample").value);
  window.history.replaceState(null, "", `#${sample.id}`); // `history` is CodeMirror's here
  loadSample(sample);
});

$("input").value = JSON.stringify(initial.input, null, 2);
$("bindings").value = JSON.stringify(initial.bindings, null, 2);
$("input").addEventListener("input", () => {
  clearTimeout(timer);
  timer = setTimeout(readInputs, 300);
});
$("bindings").addEventListener("input", () => {
  clearTimeout(timer);
  timer = setTimeout(readInputs, 300);
});
$("level").addEventListener("change", evaluate);

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
document.body.dataset.ready = "true";

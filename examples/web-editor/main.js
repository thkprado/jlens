// The jlens web editor: a JSONata editor with completion and diagnostics driven by the input
// document, live evaluation, and an explanation and audit tree of every result.
//
// Serve the repository root (`cargo xtask serve`) after building the package (`cargo xtask
// wasm`), then open /examples/web-editor/.

import { closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { bracketMatching } from "@codemirror/language";
import { lintKeymap } from "@codemirror/lint";
import { EditorView, highlightActiveLine, keymap, lineNumbers } from "@codemirror/view";

import init, {
  Expression,
  LanguageService,
  inferSchema,
  version,
} from "../../crates/jlens-wasm/pkg/jlens.js";
import { renderAudit, renderExplanation, spansOf } from "./audit-view.js";
import { jlensExtensions } from "./jlens-codemirror.js";

const SAMPLE_INPUT = {
  number: "SO-12345",
  customer: { login: "alice.walker99", name: "Acme Stores" },
  items: [
    { code: "1", color: "blue", type: "apparel", unitPrice: 10000.0, quantity: 2, tax: 1500.0 },
    { code: "2", color: "black", type: "electronic", unitPrice: 25000.0, quantity: 1, tax: 4000.0 },
    { code: "3", color: "white", type: "electronic", unitPrice: 12000.0, quantity: 1, tax: 3000.0 },
  ],
};

const SAMPLE_BINDINGS = { approvedCustomers: ["alice.walker99", "bob.harrison42"] };

const SAMPLE_EXPRESSION = `(
  $total       := $sum(items.(unitPrice * quantity));
  $electronics := $sum(items[type = 'electronic'].(unitPrice * quantity));

  $approved :=
    /** @label "Authorized customer" @message "The customer must be an approved partner." */
    customer.login in $approvedCustomers
    and
    /** @label "Minimum order grand total" @message "The total must exceed 50,000.00." */
    $total > 50000
    and
    /** @label "Electronics share" */
    $electronics > $total * 0.30 + 1000;

  $approved
    ? { "action": "FREE_SHIPPING", "code": "OK_200" }
    : { "action": "REGULAR_FEE", "code": "SHIP_FEE_402" }
)`;

const $ = (id) => document.getElementById(id);

await init();
$("status").textContent = `jlens ${version()} (WebAssembly)`;

let input = SAMPLE_INPUT;
let bindings = SAMPLE_BINDINGS;
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

const editor = new EditorView({
  doc: SAMPLE_EXPRESSION,
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
    EditorView.lineWrapping,
    jlensExtensions(() => service, $("signature")),
    EditorView.updateListener.of((update) => {
      if (update.docChanged) schedule();
    }),
  ],
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

$("input").value = JSON.stringify(SAMPLE_INPUT, null, 2);
$("bindings").value = JSON.stringify(SAMPLE_BINDINGS, null, 2);
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

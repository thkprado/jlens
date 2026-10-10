// Renders jlens audit trees and explanations as DOM.

const icons = { true: "✔", false: "✘", value: "•", undefined: "∅", error: "⚠" };

function json(value, limit = 120) {
  if (value === undefined) return "undefined";
  const text = JSON.stringify(value);
  return text.length > limit ? `${text.slice(0, limit)}…` : text;
}

function element(tag, className, text) {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** A definition list row: `name: value`. */
function row(list, name, value) {
  list.append(element("dt", null, name), element("dd", null, value));
}

/** Whether a node, or a node below it, is labelled, failed or raised an error. */
function notable(node) {
  return (
    Boolean(node.label) ||
    node.outcome === "false" ||
    node.outcome === "error" ||
    (node.children ?? []).some(notable)
  );
}

/**
 * Renders an audit node and its children as nested `<details>`. `select(span)` is called when a
 * node is clicked, to show its source range.
 */
function renderNode(node, select, depth = 0) {
  const details = document.createElement("details");
  // Open the top levels, and the way to labelled or failed checks.
  details.open = depth < 2 || notable(node);
  details.className = `audit-node outcome-${node.outcome}`;

  const summary = document.createElement("summary");
  summary.append(element("span", "icon", icons[node.outcome] ?? "•"));
  if (node.label) summary.append(element("strong", "label", node.label));
  const source = element("code", "source", node.source);
  source.title = node.source;
  summary.append(source);
  if (node.outcome !== "true" && node.outcome !== "false") {
    summary.append(element("span", "output", `→ ${json(node.output)}`));
  }
  summary.addEventListener("click", () => select(node.span));
  details.append(summary);

  const facts = document.createElement("dl");
  if (node.message) row(facts, "message", node.message);
  if (node.checkId) row(facts, "id", node.checkId);
  row(facts, "kind", node.op ? `${node.kind} ${node.op}` : node.kind);
  if (node.operands) row(facts, "operands", `${json(node.operands[0])}  vs  ${json(node.operands[1])}`);
  if (node.shortCircuited) row(facts, "short-circuited", "the right side was not evaluated");
  if (node.branch) row(facts, "branch", node.branch);
  if (node.error) row(facts, "error", `${node.error.code}: ${node.error.message}`);
  if (node.input !== undefined) row(facts, "input", json(node.input, 300));
  if (node.outcome === "true" || node.outcome === "false") row(facts, "output", json(node.output));
  for (const filter of node.filters ?? []) {
    const name = filter.label ? `filter “${filter.label}”` : `filter [${filter.source}]`;
    const rejected = filter.rejected?.length ? `; rejected e.g. ${json(filter.rejected, 300)}` : "";
    row(facts, name, `${filter.passed} of ${filter.tested} passed${rejected}`);
  }
  details.append(facts);

  for (const child of node.children ?? []) details.append(renderNode(child, select, depth + 1));
  return details;
}

/** Fills `container` with the audit tree of an `evaluateWithAudit` result. */
export function renderAudit(container, audit, select) {
  container.replaceChildren();
  const header = audit.header;
  const meta = element(
    "p",
    "audit-header",
    `${header.engine} · level ${header.level} · ${audit.nodeCount} node(s)` +
      `${audit.truncated ? " (truncated)" : ""} · ${header.durationMs.toFixed(2)} ms`,
  );
  container.append(meta);
  if (audit.root) container.append(renderNode(audit.root, select));
  else container.append(element("p", "hint", "Nothing was recorded."));
}

/** Fills `list` with the lines of an explanation. */
export function renderExplanation(list, explanation, select, spans) {
  list.replaceChildren();
  const result = element("li", "explanation-result");
  result.append(element("strong", null, "result: "), element("code", null, explanation.result));
  list.append(result);
  for (const line of explanation.lines) {
    const li = element("li", `depth-${Math.min(line.depth, 6)}`);
    li.style.setProperty("--depth", line.depth);
    const icon = line.passed === true ? "✔" : line.passed === false ? "✘" : "•";
    li.append(element("span", `icon passed-${line.passed}`, icon), ` ${line.text}`);
    if (line.message) li.append(element("div", "message", line.message));
    const span = spans.get(line.node);
    if (span) li.addEventListener("click", () => select(span));
    list.append(li);
  }
}

/** Maps syntax-tree node ids to spans, for the explanation lines. */
export function spansOf(audit) {
  const spans = new Map();
  const visit = (node) => {
    spans.set(node.node, node.span);
    for (const f of node.filters ?? []) spans.set(f.node, spans.get(f.node) ?? node.span);
    (node.children ?? []).forEach(visit);
  };
  if (audit.root) visit(audit.root);
  return spans;
}

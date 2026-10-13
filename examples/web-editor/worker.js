// Evaluates expressions for the web editor off the page's thread, so that typing never waits for
// an evaluation. Import maps do not apply to workers: the page passes the address of the package
// in the worker's own address (`worker.js?jlens=<url>`).
//
// Request: { id, source, input, bindings, level }. Reply, as JSON text: { id, elapsed, result?,
// error?, audit?, explanation? }, where `error` is { code?, message }. A reply without `audit`
// means the expression did not compile or the evaluation threw. The reply is JSON text because
// the audit tree of a deep recursion nests as deep as the recursion, and cloning it for
// `postMessage` overflows a worker's stack, which is smaller than the page's.
//
// Each expression gets the editor's host functions (host-functions.js).

import { hostFunctions } from "./host-functions.js";

const ready = import(new URL(location.href).searchParams.get("jlens")).then(async (jlens) => {
  await jlens.default();
  return jlens;
});

const reply = (message) => self.postMessage(JSON.stringify(message));
const errorInfo = (err) => ({ code: err.code, message: String(err.message ?? err) });

self.onmessage = async ({ data: { id, source, input, bindings, level } }) => {
  let expr;
  try {
    const { Expression } = await ready;
    expr = new Expression(source);
    for (const f of hostFunctions) expr.registerFunction(f.name, f.implementation, f.signature);
  } catch (err) {
    reply({ id, error: errorInfo(err) });
    return;
  }
  try {
    const started = performance.now();
    const outcome = expr.evaluateWithAudit(input, bindings, { level, timeout: 2000 });
    const elapsed = performance.now() - started;
    reply({
      id,
      elapsed,
      result: outcome.result,
      error: outcome.error && errorInfo(outcome.error),
      audit: outcome.audit,
      explanation: outcome.explanation,
    });
  } catch (err) {
    reply({ id, error: errorInfo(err) });
  } finally {
    expr.free();
  }
};

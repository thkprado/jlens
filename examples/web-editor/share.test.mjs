// The round trip of shared addresses (share.js). Run by `cargo xtask wasm-test --example`, or
// alone with `node --test examples/web-editor/share.test.mjs`.

import assert from "node:assert/strict";
import { test } from "node:test";

import { MAX_ADDRESS, PREFIX, decodeShare, encodeShare } from "./share.js";
import { samples } from "./samples.js";

test("every example survives the round trip, within the size limit", async () => {
  for (const { expression, input, bindings } of samples) {
    const fragment = await encodeShare({ expression, input, bindings });
    assert.match(fragment, /^#s=[A-Za-z0-9_-]+$/);
    assert.ok(fragment.length < MAX_ADDRESS, `${fragment.length} characters`);
    assert.deepEqual(await decodeShare(fragment), { expression, input, bindings });
  }
});

test("text outside ASCII and every kind of JSON value survive the round trip", async () => {
  const state = {
    expression: '"Olá, " & name & " ✓ 😀" /* ç */',
    input: { name: "Zoë", n: [0, -1.5, 1e300], ok: true, none: null, nested: { "": [{}] } },
    bindings: { emoji: "😀", list: [] },
  };
  assert.deepEqual(await decodeShare(await encodeShare(state)), state);
});

test("fragments that are not shared ones, and damaged ones", async () => {
  assert.equal(await decodeShare("#invoice"), null);
  assert.equal(await decodeShare(""), null);
  await assert.rejects(decodeShare(`${PREFIX}not-deflate`));
  await assert.rejects(decodeShare(`${PREFIX}!!!`));
  const noExpression = await encodeShare({ input: {}, bindings: {} });
  await assert.rejects(decodeShare(noExpression));
});

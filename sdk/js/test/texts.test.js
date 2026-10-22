import test from "node:test";
import assert from "node:assert/strict";

import { OPS, dayOf, faceDetail, friendsDetail, friendsSignText, jsonString, playMessage, FACE_ACTION } from "../index.js";
import { fx } from "./_testdata.js";

test("the ops are the server's", () => {
  assert.deepEqual({ ...OPS }, fx("friends_texts.json").ops);
});

test("every friends text rebuilds byte for byte", () => {
  const cases = fx("friends_texts.json").cases;
  assert.deepEqual(new Set(cases.map((c) => c.op)), new Set(Object.keys(OPS)));
  for (const c of cases) {
    assert.equal(dayOf(c.at), c.day, c.op);
    assert.equal(friendsDetail(c.op, c.other, c.at), c.detail, c.op);
    assert.equal(friendsSignText(c.wallet, c.op, c.other, c.at), c.text, c.op);
    const upper = "0x" + c.wallet.slice(2).toUpperCase();
    assert.equal(friendsSignText(upper, c.op, c.other, c.at), c.text, "a checksummed wallet signs lowercase");
  }
});

test("nothing is built for a bad op or wallet", () => {
  const w = "0x1111111111111111111111111111111111111111";
  assert.equal(friendsDetail("poke", w, 1), null);
  assert.equal(friendsDetail("ask", "0x123", 1), null);
  assert.equal(friendsSignText("me", "ask", w, 1), null);
});

test("the day turns at UTC midnight", () => {
  assert.equal(dayOf(1761004799), "2025-10-20");
  assert.equal(dayOf(1761004800), "2025-10-21");
});

test("every face text rebuilds byte for byte", () => {
  for (const c of fx("face.json").cases) {
    const f = c.fields;
    const detail = faceDetail(c.at, { name: f.name, pictureSha256: f.picture_sha256, kind: f.kind,
      harness: f.harness, model: f.model });
    assert.equal(detail, c.detail);
    assert.equal(playMessage(FACE_ACTION, c.wallet, detail, c.day), c.text);
  }
});

test("JSON strings are Python's", () => {
  assert.equal(jsonString('Doc "W" \\o/'), '"Doc \\"W\\" \\\\o/"');
  assert.equal(jsonString("é😀"), '"\\u00e9\\ud83d\\ude00"');
});

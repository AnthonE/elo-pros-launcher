import test from "node:test";
import assert from "node:assert/strict";

import { OPS, dayOf, faceDetail, friendsDetail, friendsSignText, jsonString, playMessage, FACE_ACTION, BUDDY_ACTION,
  BUDDY_OPS, buddyChange, claimText, entitleText, readPlay, sameArgs, submitDetail, submitText } from "../index.js";
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
      harness: f.harness, model: f.model, avatar: f.avatar });
    assert.equal(detail, c.detail);
    assert.equal(playMessage(FACE_ACTION, c.wallet, detail, c.day), c.text);
  }
});

test("JSON strings are Python's", () => {
  assert.equal(jsonString('Doc "W" \\o/'), '"Doc \\"W\\" \\\\o/"');
  assert.equal(jsonString("é😀"), '"\\u00e9\\ud83d\\ude00"');
});

test("the buddy ops are the server's", () => {
  assert.deepEqual([...BUDDY_OPS], fx("buddy_texts.json").ops);
});

test("every buddy text reads back as the change asked for", () => {
  for (const c of fx("buddy_texts.json").cases) {
    const got = buddyChange(c.text, c.wallet);
    assert.deepEqual(got, { day: c.day, op: c.op, args: c.args, at: c.at }, c.op);
    assert.ok(sameArgs(c.asked, got.args), `${c.op}: ${JSON.stringify(c.asked)}`);
    assert.deepEqual(readPlay(c.text, BUDDY_ACTION, "0x" + c.wallet.slice(2).toUpperCase()),
      { day: c.day, detail: c.detail }, "a checksummed wallet reads the same text");
  }
});

test("a buddy text for another change is never the one asked for", () => {
  for (const c of fx("buddy_texts.json").refused) {
    const got = buddyChange(c.text, c.wallet);
    assert.ok(!(got && got.op === c.op && sameArgs(c.asked, got.args)), c.why);
  }
});

test("every board text rebuilds byte for byte", async () => {
  for (const c of fx("board_texts.json").cases) {
    if (c.op === "claim") {
      assert.equal(claimText(c.wallet, c.quest_id, c.day), c.text);
    } else {
      assert.equal(await submitDetail(c.quest_id, c.url, c.n), c.detail);
      assert.equal(c.detail.split(" ")[1], c.url_sha16);
      assert.equal(await submitText(c.wallet, c.quest_id, c.url, c.n, c.day), c.text);
    }
  }
});

test("every entitle text rebuilds byte for byte", () => {
  for (const c of fx("entitle_texts.json").cases) assert.equal(entitleText(c.game, c.wallet, c.day), c.text);
});

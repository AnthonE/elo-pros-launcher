/* Moving a player between a game's servers: the handoff a server signs and
   the door the next one admits it at, against the shared fixture. */
import test from "node:test";
import assert from "node:assert/strict";

import { HandoffDoor, HandoffRefused, ServerKey, handoffText, readHandoff, verifyHandoff,
  HANDOFF_SKEW_S } from "../index.js";
import { fx } from "./_testdata.js";

const FX = fx("handoff.json");
const carryOf = (r) => r.carry_utf8 || "";
const key = () => ServerKey.fromSeed(FX.game, FX.key_id, FX.seed_hex);
const clock = (t) => { const c = () => c.t; c.t = t; return c; };
const code = async (p) => { try { await p; return "admitted"; } catch (e) { assert.ok(e instanceof HandoffRefused, e); return e.code; } };

test("the text and the token match the shared fixture", async () => {
  const k = await key();
  assert.equal(k.publicB64, FX.public_b64);
  for (const c of FX.cases) {
    const carry = carryOf(c) || null;
    assert.equal(await handoffText(FX.game, FX.key_id, c.player, FX.from, FX.to, c.id, FX.exp, carry), c.text);
    assert.equal(await k.handoff(c.player, FX.from, FX.to, { carry, ttl: FX.exp - FX.now, now: FX.now, id: c.id }), c.token);
    assert.equal(readHandoff(c.token).player, c.player);
  }
});

test("every case admits once, then every refusal answers its code", async () => {
  const door = new HandoffDoor(FX.game, [FX.to], { keys: FX.keys, clock: clock(FX.now) });
  for (const c of FX.cases) assert.equal((await door.admit(c.token, carryOf(c))).player, c.player);
  for (const r of FX.refused) assert.equal(await code(door.admit(r.token, carryOf(r))), r.code, r.code);
});

test("a refused handoff is not used up", async () => {
  const door = new HandoffDoor(FX.game, [FX.to], { keys: FX.keys, clock: clock(FX.now) });
  const c = FX.cases[2];
  assert.equal(await code(door.admit(c.token, "tampered")), "carry");
  assert.equal(await code(door.admit(c.token, carryOf(c))), "admitted");
});

test("the window", async () => {
  const t = FX.cases[0].token, kw = { game: FX.game, names: [FX.to], keys: FX.keys };
  assert.equal(await code(verifyHandoff(t, { ...kw, now: FX.exp + HANDOFF_SKEW_S - 1 })), "admitted");
  assert.equal(await code(verifyHandoff(t, { ...kw, now: FX.exp + HANDOFF_SKEW_S })), "expired");
});

test("issuing refuses what no door would take", async () => {
  const k = await key(), p = FX.cases[0].player;
  await assert.rejects(k.handoff("0xABC", FX.from, FX.to));
  await assert.rejects(k.handoff(p, "no-port", FX.to));
  await assert.rejects(k.handoff(p, FX.from, FX.to, { ttl: 121 }));
  await assert.rejects(k.handoff(p, FX.from, FX.to, { carry: new Uint8Array(4097) }));
  await assert.rejects(k.handoff(p, FX.from, FX.to, { id: "short" }));
});

class FakeClient {
  constructor(rows) { this.rows = rows; this.calls = 0; }
  async serverKeys(game) {
    this.calls += 1;
    return this.rows === null ? { kind: "could_not_look", ok: false, error: "unreachable" }
      : { kind: "answer", ok: true, value: { game, keys: this.rows } };
  }
}
const rows = (...ids) => ids.map((id) => ({ id, pub: FX.public_b64 }));

test("a door reading the keys: caches, picks up a rotated key, drops a revoked one", async () => {
  const k = await key(), p = FX.cases[0].player;
  const fresh = (t) => k.handoff(p, FX.from, FX.to, { now: Math.trunc(t) });
  const cl = new FakeClient(rows("old-1"));
  const c = clock(FX.now);
  const door = new HandoffDoor(FX.game, [FX.to], { client: cl, clock: c });
  assert.equal(await code(door.admit(await fresh(c.t))), "unknown_key");
  assert.equal(await code(door.admit(await fresh(c.t))), "unknown_key");
  assert.equal(cl.calls, 1, "an unknown key does not hammer the origin");
  cl.rows = rows(FX.key_id);
  c.t += 60;
  assert.equal(await code(door.admit(await fresh(c.t))), "admitted");
  cl.rows = [];
  c.t += 299;
  assert.equal(await code(door.admit(await fresh(c.t))), "admitted");
  c.t += 1;
  assert.equal(await code(door.admit(await fresh(c.t))), "unknown_key");
});

test("could not look is its own answer", async () => {
  const door = new HandoffDoor(FX.game, [FX.to], { client: new FakeClient(null), clock: clock(FX.now) });
  assert.equal(await code(door.admit(FX.cases[0].token)), "no_keys");
  assert.throws(() => new HandoffDoor(FX.game, [FX.to]));
});

test("admits run one at a time, so a token raced twice admits once", async () => {
  const door = new HandoffDoor(FX.game, [FX.to], { keys: FX.keys, clock: clock(FX.now) });
  const t = FX.cases[0].token;
  const got = await Promise.all([code(door.admit(t)), code(door.admit(t))]);
  assert.deepEqual(got.sort(), ["admitted", "used"]);
});

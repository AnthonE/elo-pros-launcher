/* The client against a local stub server: a refusal, could-not-look, and the
   guard that refuses to sign a text the change did not ask for. */
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";

import { ActError, Client, friendsSignText } from "../index.js";
import { fx } from "./_testdata.js";

const ME = "0x1111111111111111111111111111111111111111";
const THEM = "0x7777777777777777777777777777777777777777";
const AT = 1761003600;

async function stub(route) {
  const seen = [];
  const server = createServer((req, res) => {
    let raw = "";
    req.on("data", (c) => { raw += c; });
    req.on("end", () => {
      seen.push(req.url);
      const [status, body] = route(req.url, raw ? JSON.parse(raw) : null);
      const text = typeof body === "string" ? body : JSON.stringify(body);
      res.writeHead(status, { "Content-Type": "application/json" });
      res.end(text);
    });
  });
  await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
  return { origin: `http://127.0.0.1:${server.address().port}`, seen,
    close: () => new Promise((ok) => server.close(ok)) };
}

test("a text other than the change asked for is never signed", async () => {
  const offered = friendsSignText(ME, "block", THEM, AT);
  const s = await stub((path) => (path === "/api/friends/prepare"
    ? [200, { sign_this: offered, at: AT, args: { wallet: THEM }, op: "block" }] : [500, {}]));
  const signed = [];
  try {
    await assert.rejects(new Client(s.origin).friend(ME, "ask", THEM, (t) => { signed.push(t); return "0xsig"; }),
      (e) => e instanceof ActError && e.code === "mismatch" && e.offered === offered);
  } finally {
    await s.close();
  }
  assert.deepEqual(signed, [], "the signer was never asked");
  assert.deepEqual(s.seen, ["/api/friends/prepare"], "nothing was acted");
});

test("the rebuilt text is signed once and acted", async () => {
  const text = friendsSignText(ME, "ask", THEM, AT);
  const s = await stub((path, body) => {
    if (path === "/api/friends/prepare") return [200, { sign_this: text, at: AT, args: { to: THEM }, op: "ask" }];
    if (path === "/api/friends/act" && body.signature === "0xsig" && body.args.to === THEM) {
      return [200, { ok: true, op: "ask", view: fx("friends_view.json") }];
    }
    return [401, { error: "bad act" }];
  });
  const signed = [];
  let got;
  try {
    got = await new Client(s.origin).friend(ME, "ask", THEM, async (t) => { signed.push(t); return "0xsig"; });
  } finally {
    await s.close();
  }
  assert.equal(got.kind, "answer");
  assert.equal(got.value.view.buddies.length, 1);
  assert.deepEqual(signed, [text]);
});

test("bad input sends nothing", async () => {
  const c = new Client("http://127.0.0.1:9");
  await assert.rejects(c.friend(ME, "poke", THEM, () => "0x"), (e) => e.code === "bad_input");
  await assert.rejects(c.friend(ME, "ask", ME, () => "0x"), (e) => e.code === "bad_input");
  assert.equal((await c.friends("0x123")).status, 0);
  assert.equal((await c.leaderboard("gates", { kind: "robot" })).kind, "refused");
});

test("a refusal and a dead origin are different answers", async () => {
  const s = await stub((path) => {
    if (path.startsWith("/api/friends/")) return [404, { error: "not here" }];
    if (path.startsWith("/api/ticket/gates/of/")) {
      return [503, { game: "gates", entitled: null, reachable: false, why: "the chain read failed" }];
    }
    return [200, "not json"];
  });
  try {
    const c = new Client(s.origin);
    const got = await c.friends(ME);
    assert.equal(got.kind, "refused");
    assert.equal(got.status, 404);
    assert.deepEqual(await c.owns("gates", ME), { kind: "could_not_look", ok: false, error: "the chain read failed" });
    assert.equal((await c.face(ME)).kind, "could_not_look", "a body that is not JSON is could-not-look");
  } finally {
    await s.close();
  }
  const dead = await stub(() => [200, {}]);
  await dead.close();
  assert.equal((await new Client(dead.origin, { timeoutMs: 2000 }).friends(ME)).kind, "could_not_look");
});

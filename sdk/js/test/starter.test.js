/* The web-game starter (examples/web-game), run for real against a stub
   meter: challenge, sign-in, the copy check, the roster sweep. */
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { readFileSync } from "node:fs";

import { start } from "../examples/web-game/server.mjs";
import { fx } from "./_testdata.js";

const EVM = fx("signin_check.json").cases[0];
const ADDR = EVM.expect.address;

/* The meter's half: /signin/check answers for the nonce it is sent; the copy
   reads answer from `owned`, which a test flips. */
async function meter(state) {
  const server = createServer((req, res) => {
    let raw = "";
    req.on("data", (c) => { raw += c; });
    req.on("end", () => {
      const body = raw ? JSON.parse(raw) : null;
      state.seen.push([req.method, req.url, body]);
      const send = (status, out) => { res.writeHead(status, { "Content-Type": "application/json" }); res.end(JSON.stringify(out)); };
      if (req.url === "/api/signin/check") {
        if (state.checkDown) return send(503, { ok: false, reachable: false, error: "chain down" });
        if (body.signature !== "0xgood") return send(401, { ok: false, code: "signature", error: "not this wallet's" });
        return send(200, { ok: true, address: ADDR, chain: "evm", chain_id: "4663", domain: body.domain,
          nonce: body.nonce, uri: "https://x", issued_at: "x" });
      }
      if (req.url.startsWith("/api/ticket/my-game/of/")) {
        if (state.owned === null) return send(503, { error: "rpc down" });
        return send(200, { game: "my-game", entitled: state.owned });
      }
      if (req.url === "/api/ticket/my-game/check") {
        if (state.rosterDown) return send(502, { error: "rpc down" });
        return send(200, { game: "my-game", wallets: Object.fromEntries(body.wallets.map((w) => [w,
          { entitled: state.rosterNull ? null : state.owned }])) });
      }
      send(404, { error: "no" });
    });
  });
  await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
  return { origin: `http://127.0.0.1:${server.address().port}`, close: () => new Promise((ok) => server.close(ok)) };
}

async function boot(state) {
  const m = await meter(state);
  const logs = [];
  const game = await start({ port: 0, domain: "play.my-game.example", slug: "my-game", origin: m.origin,
    sweepMs: 0, log: (l) => logs.push(l) });
  const base = `http://127.0.0.1:${game.port}`;
  const nonce = async () => (await (await fetch(base + "/nonce")).json()).nonce;
  const signin = (body) => fetch(base + "/signin", { method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body) });
  return { m, game, base, nonce, signin, logs,
    close: async () => { await game.close(); await m.close(); } };
}

test("challenge → sign-in → session, checked for our domain and that nonce", async () => {
  const state = { seen: [], owned: true };
  const g = await boot(state);
  try {
    const n = await g.nonce();
    assert.match(n, /^[A-Za-z0-9]{16}$/);
    const r = await g.signin({ message: "m", signature: "0xgood", nonce: n });
    const body = await r.json();
    assert.equal(r.status, 200, JSON.stringify(body));
    assert.equal(body.address, ADDR);
    const check = state.seen.find(([, url]) => url === "/api/signin/check");
    assert.equal(check[2].domain, "play.my-game.example");
    assert.equal(check[2].nonce, n);
    const me = await (await fetch(g.base + "/me", { headers: { Authorization: `Bearer ${body.session}` } })).json();
    assert.equal(me.address, ADDR);
    assert.equal((await fetch(g.base + "/me")).status, 401);
  } finally {
    await g.close();
  }
});

test("a nonce works once, and only one we issued", async () => {
  const state = { seen: [], owned: true };
  const g = await boot(state);
  try {
    const n = await g.nonce();
    assert.equal((await g.signin({ message: "m", signature: "0xgood", nonce: n })).status, 200);
    assert.equal((await g.signin({ message: "m", signature: "0xgood", nonce: n })).status, 401, "replayed");
    assert.equal((await g.signin({ message: "m", signature: "0xgood", nonce: "MadeUpNonce123" })).status, 401);
    const bad = await g.nonce();
    assert.equal((await g.signin({ message: "m", signature: "0xbad", nonce: bad })).status, 401, "not the wallet's");
    assert.equal((await g.signin({ message: "m", signature: "0xgood", nonce: bad })).status, 401,
      "a nonce spent on a failed sign-in stays spent");
  } finally {
    await g.close();
  }
});

test("no copy is turned away; a copy check that could not look lets the player in", async () => {
  const state = { seen: [], owned: false };
  const g = await boot(state);
  try {
    const r = await g.signin({ message: "m", signature: "0xgood", nonce: await g.nonce() });
    assert.equal(r.status, 402);
    assert.match((await r.json()).buy, /game\.html\?slug=my-game$/);
    state.owned = null;
    assert.equal((await g.signin({ message: "m", signature: "0xgood", nonce: await g.nonce() })).status, 200);
  } finally {
    await g.close();
  }
});

test("a sign-in check that could not be made is a 503, not a refusal", async () => {
  const state = { seen: [], owned: true, checkDown: true };
  const g = await boot(state);
  try {
    assert.equal((await g.signin({ message: "m", signature: "0xgood", nonce: await g.nonce() })).status, 503);
  } finally {
    await g.close();
  }
});

test("the sweep signs out a sold copy, and nobody on a failed read", async () => {
  const state = { seen: [], owned: true };
  const g = await boot(state);
  try {
    await g.signin({ message: "m", signature: "0xgood", nonce: await g.nonce() });
    assert.equal(g.game.sessions.size, 1);
    state.rosterDown = true;
    assert.deepEqual(await g.game.sweep(), { swept: 1, dropped: 0 });
    state.rosterDown = false;
    state.rosterNull = true;
    assert.deepEqual(await g.game.sweep(), { swept: 1, dropped: 0 }, "null is never a kick");
    state.rosterNull = false;
    state.owned = false;
    assert.deepEqual(await g.game.sweep(), { swept: 1, dropped: 1 });
    assert.equal(g.game.sessions.size, 0);
  } finally {
    await g.close();
  }
});

test("the page pins elo-3.js by the released file's own hash", () => {
  const html = readFileSync(new URL("../examples/web-game/index.html", import.meta.url), "utf8");
  const sri = "sha384-" + createHash("sha384").update(readFileSync(new URL("../elo-3.js", import.meta.url))).digest("base64");
  assert.match(html, /<script src="https:\/\/elopros\.com\/sdk\/elo-3\.js"/);
  assert.ok(html.includes(`integrity="${sri}"`), `want ${sri}`);
  assert.ok(html.includes('crossorigin="anonymous"'));
});

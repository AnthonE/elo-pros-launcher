/* A game server's half: the sign-in check, the roster sweep and the nonce,
   against a local stub server that answers the way the meter does. */
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";

import { Client, ROSTER_PAGE, newNonce } from "../index.js";
import { fx } from "./_testdata.js";

const SIGNIN = fx("signin_check.json");
const EVM = SIGNIN.cases[0];
const SOL = SIGNIN.cases[1];

async function stub(route) {
  const seen = [];
  const server = createServer((req, res) => {
    let raw = "";
    req.on("data", (c) => { raw += c; });
    req.on("end", () => {
      const body = raw ? JSON.parse(raw) : null;
      seen.push({ url: req.url, method: req.method, body });
      const [status, out] = route(req.url, body);
      res.writeHead(status, { "Content-Type": "application/json" });
      res.end(typeof out === "string" ? out : JSON.stringify(out));
    });
  });
  await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
  return { origin: `http://127.0.0.1:${server.address().port}`, seen,
    close: () => new Promise((ok) => server.close(ok)) };
}

test("checkPass sends the pass, the game, your host and your nonce", async () => {
  const PASS = { ok: true, player: "acct_" + "a".repeat(20), account: "acct_" + "a".repeat(20), wallet: null,
    name: "Bob", named: true, game: "my-game", domain: "play.example", exp: 1900000000 };
  const s = await stub((url) => (url === "/api/account/play/check" ? [200, PASS] : [404, { error: "no" }]));
  try {
    const c = new Client(s.origin);
    const got = await c.checkPass("pass.sig", { game: "my-game", domain: "Play.Example", nonce: "a".repeat(32) });
    assert.equal(got.kind, "answer");
    assert.equal(got.value.player, PASS.player);
    assert.deepEqual(s.seen[0].body, { token: "pass.sig", game: "my-game", domain: "play.example", nonce: "a".repeat(32) });
    assert.equal((await c.checkPass("pass.sig", { game: "my-game", domain: "play.example:443", nonce: "a".repeat(32) })).status, 0);
    assert.equal((await c.checkPass("pass.sig", { game: "my-game", domain: "play.example", nonce: "a".repeat(65) })).status, 0);
    assert.equal(s.seen.length, 1);
  } finally {
    await s.close();
  }
});

test("checkSignIn sends the message, the signature, your domain and nonce", async () => {
  const s = await stub((url, body) => (url === "/api/signin/check" && body.domain === EVM.domain
    ? [200, { ok: true, address: EVM.expect.address, chain: "evm", chain_id: "4663" }]
    : [404, { error: "no" }]));
  try {
    const got = await new Client(s.origin).checkSignIn(EVM.message, EVM.signature,
      { domain: EVM.domain, nonce: EVM.nonce, chainId: 4663, maxAgeS: 300 });
    assert.equal(got.kind, "answer");
    assert.equal(got.value.address, EVM.expect.address);
    assert.deepEqual(s.seen[0].body, { message: EVM.message, signature: EVM.signature,
      domain: EVM.domain, nonce: EVM.nonce, chain_id: "4663", max_age_s: 300 });
    assert.equal(s.seen[0].method, "POST");
  } finally {
    await s.close();
  }
});

test("a sign-in that does not check out is refused, with the field that failed", async () => {
  const s = await stub(() => [401, { ok: false, code: "nonce", error: "this sign-in carries another nonce" }]);
  try {
    const got = await new Client(s.origin).checkSignIn(SOL.message, SOL.signature,
      { domain: SOL.domain, nonce: "Different1" });
    assert.equal(got.kind, "refused");
    assert.equal(got.status, 401);
    assert.equal(got.body.code, "nonce");
    assert.match(got.error, /another nonce/);
  } finally {
    await s.close();
  }
});

test("a chain that could not be asked is could_not_look, never a refusal", async () => {
  const s = await stub(() => [503, { ok: false, reachable: false, code: "unreachable",
    error: "could not reach the chain to check this wallet's signature" }]);
  try {
    const got = await new Client(s.origin).checkSignIn(EVM.message, EVM.signature,
      { domain: EVM.domain, nonce: EVM.nonce });
    assert.equal(got.kind, "could_not_look");
    assert.match(got.error, /could not reach the chain/);
  } finally {
    await s.close();
  }
});

test("checkSignIn refuses before asking when the domain or nonce is missing or malformed", async () => {
  const s = await stub(() => [200, { ok: true }]);
  try {
    const c = new Client(s.origin);
    for (const opts of [{ nonce: EVM.nonce }, { domain: EVM.domain },
      { domain: "https://game.example", nonce: EVM.nonce }, { domain: "game.example/play", nonce: EVM.nonce },
      { domain: EVM.domain, nonce: "550e8400-e29b-41d4" }, { domain: EVM.domain, nonce: EVM.nonce, maxAgeS: 0 }]) {
      const got = await c.checkSignIn(EVM.message, EVM.signature, opts);
      assert.equal(got.kind, "refused", JSON.stringify(opts));
      assert.equal(got.status, 0, "refused here, before asking");
    }
    assert.equal((await c.checkSignIn("", EVM.signature, { domain: EVM.domain, nonce: EVM.nonce })).status, 0);
    assert.equal((await c.checkSignIn(EVM.message, "", { domain: EVM.domain, nonce: EVM.nonce })).status, 0);
    assert.deepEqual(s.seen, [], "nothing was sent");
  } finally {
    await s.close();
  }
});

const W = (i) => "0x" + i.toString(16).padStart(40, "0");
const SOLW = "GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB";

test("checkRoster pages the roster and keys every wallet", async () => {
  const roster = Array.from({ length: ROSTER_PAGE + 5 }, (_, i) => W(i + 1));
  roster.push(roster[0].toUpperCase().replace("0X", "0x"), SOLW);
  const s = await stub((url, body) => {
    const wallets = {};
    for (const w of body.wallets) wallets[w] = { entitled: w !== W(3) };
    return [200, { game: "gates", wallets }];
  });
  try {
    const got = await new Client(s.origin).checkRoster("gates", roster);
    assert.equal(got.kind, "answer");
    assert.equal(s.seen.length, 2, "two pages");
    assert.equal(s.seen[0].body.wallets.length, ROSTER_PAGE);
    assert.ok(s.seen.every((x) => x.url === "/api/ticket/gates/check" && x.method === "POST"));
    const keys = Object.keys(got.value.entitled);
    assert.equal(keys.length, ROSTER_PAGE + 6, "a repeated wallet is swept once");
    assert.equal(got.value.entitled[W(3)], false);
    assert.equal(got.value.entitled[W(4)], true);
    assert.equal(got.value.entitled[SOLW], true, "a Solana wallet keeps its case");
    assert.equal(got.value.ticketed, true);
  } finally {
    await s.close();
  }
});

test("a page that could not be read is null for every wallet on it — never a kick", async () => {
  const roster = Array.from({ length: ROSTER_PAGE + 2 }, (_, i) => W(i + 1));
  let n = 0;
  const s = await stub((url, body) => {
    n += 1;
    if (n === 2) return [502, { error: "rpc down" }];
    return [200, { game: "gates", wallets: Object.fromEntries(body.wallets.map((w) => [w, { entitled: false }])) }];
  });
  try {
    const got = await new Client(s.origin).checkRoster("gates", roster);
    assert.equal(got.kind, "answer");
    assert.equal(got.value.entitled[W(1)], false);
    assert.equal(got.value.entitled[W(ROSTER_PAGE + 1)], null);
    assert.equal(got.value.entitled[W(ROSTER_PAGE + 2)], null);
  } finally {
    await s.close();
  }
});

test("a row the server could not read stays null", async () => {
  const s = await stub(() => [200, { game: "gates", wallets: { [W(1)]: { entitled: null } } }]);
  try {
    const got = await new Client(s.origin).checkRoster("gates", [W(1), W(2)]);
    assert.equal(got.value.entitled[W(1)], null);
    assert.equal(got.value.entitled[W(2)], null, "a wallet the reply left out is not a zero");
  } finally {
    await s.close();
  }
});

test("no page read is could_not_look; a free title is everyone's", async () => {
  const down = await stub(() => [503, { error: "down" }]);
  try {
    assert.equal((await new Client(down.origin).checkRoster("gates", [W(1)])).kind, "could_not_look");
  } finally {
    await down.close();
  }
  const free = await stub(() => [200, { game: "free-game", ticketed: false, every_wallet_entitled: true }]);
  try {
    const got = await new Client(free.origin).checkRoster("free-game", [W(1), SOLW]);
    assert.deepEqual(got.value, { game: "free-game", ticketed: false, entitled: { [W(1)]: true, [SOLW]: true } });
  } finally {
    await free.close();
  }
});

test("checkRoster refuses a bad roster before asking", async () => {
  const s = await stub(() => [200, {}]);
  try {
    const c = new Client(s.origin);
    assert.equal((await c.checkRoster("gates", [])).status, 0);
    assert.equal((await c.checkRoster("gates", [W(1), "0xnope"])).status, 0);
    assert.equal((await c.checkRoster("Not A Slug", [W(1)])).status, 0);
    assert.deepEqual(s.seen, []);
  } finally {
    await s.close();
  }
});

test("ticketOf takes a Solana wallet as given", async () => {
  const s = await stub((url) => [200, { game: "gates", wallet: SOLW, entitled: false, linked: false }]);
  try {
    const got = await new Client(s.origin).owns("gates", SOLW);
    assert.equal(got.kind, "answer");
    assert.equal(got.value, false);
    assert.equal(s.seen[0].url, `/api/ticket/gates/of/${SOLW}`);
  } finally {
    await s.close();
  }
});

test("newNonce is EIP-4361's alphabet, the length asked for, and fresh", async () => {
  const seen = new Set();
  for (let i = 0; i < 200; i++) {
    const n = await newNonce();
    assert.match(n, /^[A-Za-z0-9]{16}$/);
    seen.add(n);
  }
  assert.equal(seen.size, 200);
  assert.match(await newNonce(8), /^[A-Za-z0-9]{8}$/);
  await assert.rejects(newNonce(7), RangeError);
  await assert.rejects(newNonce(129), RangeError);
});

/* A game server's key: the signed text and signature match the shared
   fixture byte for byte, and the signed calls send what the origin checks. */
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";

import { Client, ServerKey, newServerNonce, serverText } from "../index.js";
import { fx } from "./_testdata.js";

const FX = fx("server_texts.json");
// PEM armour around a base64 body, built here so no key header is a literal.
const LABEL = ["PRIVATE", "KEY"].join(" ");
const pem = (body) => `-----BEGIN ${LABEL}-----\n${body}\n-----END ${LABEL}-----\n`;

test("texts and signatures match the shared fixture", async () => {
  const key = await ServerKey.fromSeed("my-game", "eu-1", FX.seed_hex);
  assert.equal(key.publicB64, FX.public_b64);
  for (const c of FX.cases) {
    const text = await serverText(c.method, c.path, c.who, c.at, c.body_utf8, c.nonce);
    assert.equal(text, c.text);
    assert.equal(await key.signText(text), c.signature_b64);
  }
});

test("a PEM gives the same key, and a bad one is refused", async () => {
  const key = await ServerKey.fromPem("my-game", "eu-1", pem(FX.private_pkcs8_b64));
  assert.equal(key.publicB64, FX.public_b64);
  await assert.rejects(ServerKey.fromPem("my-game", "eu-1", pem("AAAA")));
  await assert.rejects(ServerKey.fromSeed("My Game", "eu-1", FX.seed_hex));
  await assert.rejects(newServerNonce(4));
});

test("cloudPut signs the bare path and the body; cloudGet returns bytes", async () => {
  const seen = [];
  const server = createServer((req, res) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      seen.push({ method: req.method, url: req.url, headers: req.headers, body: Buffer.concat(chunks) });
      if (req.method === "GET" && req.url.endsWith("/save")) {
        res.writeHead(200, { "Content-Type": "application/octet-stream", ETag: '"abc"', "X-Elo-Version": "3" });
        res.end(Buffer.from([0, 1, 2]));
        return;
      }
      res.writeHead(req.method === "PUT" ? 201 : 200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ ok: true }));
    });
  });
  await new Promise((ok) => server.listen(0, "127.0.0.1", ok));
  try {
    const elo = new Client(`http://127.0.0.1:${server.address().port}`);
    const key = await ServerKey.fromSeed("my-game", "eu-1", FX.seed_hex);
    const w = "0x" + "11".repeat(20);
    const put = await elo.cloudPut(key, w, "save", '{"level":3}', { ifMatch: "abc" });
    assert.equal(put.ok, true);
    const s = seen.at(-1);
    assert.equal(s.url, `/api/cloud/my-game/${w}/save`);
    assert.equal(s.headers["if-match"], "abc");
    const text = await serverText("PUT", `/cloud/my-game/${w}/save`, "my-game/eu-1", s.headers["x-elo-time"],
      new Uint8Array(s.body), s.headers["x-elo-nonce"]);
    assert.equal(s.headers["x-elo-signature"], await key.signText(text));
    const got = await elo.cloudGet(key, w, "save");
    assert.deepEqual([...got.value.data], [0, 1, 2]);
    assert.equal(got.value.etag, "abc");
    assert.equal(got.value.version, 3);
    const bad = await elo.cloudPut(key, w, "../x", "");
    assert.equal(bad.kind, "refused");
    assert.equal(bad.status, 0);
    const acct = "acct_" + "0a".repeat(10);
    assert.equal((await elo.cloudPut(key, acct, "save", "x")).ok, true);
    assert.equal(seen.at(-1).url, `/api/cloud/my-game/${acct}/save`);
    await elo.workshopSubscribed("my-game", acct);
    assert.equal(seen.at(-1).url, `/api/workshop/my-game/subscribed/${acct}`);
    assert.equal((await elo.cloudPut(key, "acct_XYZ", "save", "x")).kind, "refused");
    assert.equal((await elo.quickJoin("my-game", 0)).kind, "refused");
    const rows = [{ id: "b-1", player: w, item: "vip" }];
    assert.equal((await elo.economyBuy(key, rows)).ok, true);
    const b = seen.at(-1);
    assert.equal(b.method, "POST");
    assert.equal(b.url, "/api/economy/my-game/buy");
    assert.deepEqual(JSON.parse(b.body.toString()), { buys: rows });
    const btext = await serverText("POST", "/economy/my-game/buy", "my-game/eu-1", b.headers["x-elo-time"],
      new Uint8Array(b.body), b.headers["x-elo-nonce"]);
    assert.equal(b.headers["x-elo-signature"], await key.signText(btext));
    await elo.economyGrant(key, [{ id: "g", player: w, amount: 3 }]);
    assert.equal(seen.at(-1).url, "/api/economy/my-game/grant");
    await elo.economySpend(key, [{ id: "s", player: w, item: "potion" }]);
    assert.equal(seen.at(-1).url, "/api/economy/my-game/spend");
    const tx = "0x" + "ab".repeat(32);
    await elo.economyBuyConfirm(key, "b-1", tx);
    assert.deepEqual(JSON.parse(seen.at(-1).body.toString()), { id: "b-1", tx });
    await elo.economyItems("my-game", "acct_" + "1".repeat(20));
    assert.equal(seen.at(-1).url, "/api/economy/my-game/items/acct_" + "1".repeat(20));
    await elo.economyBalance("my-game", w);
    assert.equal(seen.at(-1).url, `/api/economy/my-game/balance/${w}`);
    const n = seen.length;
    assert.equal((await elo.economyBuy(key, [])).kind, "refused");
    assert.equal((await elo.economyBuyConfirm(key, "b-1", "nope")).kind, "refused");
    assert.equal((await elo.economyItems("my-game", "bob")).kind, "refused");
    assert.equal(seen.length, n);
  } finally {
    await new Promise((ok) => server.close(ok));
  }
});

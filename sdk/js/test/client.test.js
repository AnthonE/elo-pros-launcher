/* The client against a local stub server: a refusal, could-not-look, and the
   guard that refuses to sign a text the change did not ask for. */
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";

import { ActError, BUDDY_ACTION, Client, claimText, dayOf, entitleText, friendsSignText, playMessage, submitDetail,
  submitText } from "../index.js";
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
  assert.equal((await c.guildsOf("0x123")).kind, "refused");
  assert.equal((await c.guildTags([])).kind, "refused");
  assert.equal((await c.guildTags(["0x" + "1".repeat(40), "nope"])).kind, "refused");
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

const today = () => dayOf(Math.floor(Date.now() / 1000));
/* buddy.detail_of's bytes, as Python writes them (5.0, not 5) */
const buddyText = (op, argsJson, at) => playMessage(BUDDY_ACTION, ME, `{"args":${argsJson},"at":${at},"op":"${op}"}`, today());

async function runBuddy(op, offeredJson, asked, opts) {
  const offered = buddyText(op, offeredJson, AT);
  const acted = [];
  const s = await stub((path, body) => {
    if (path === "/api/buddy/prepare") return [200, { sign_this: offered, at: AT, op }];
    if (path === "/api/buddy/act") { acted.push(body); return [200, { ok: true, done: { op }, you: {} }]; }
    return [404, { error: "no" }];
  });
  const signed = [];
  try {
    const got = await new Client(s.origin).buddy(ME, op, asked, (t) => { signed.push(t); return "0xsig"; }, opts);
    return { got, signed, acted, offered, seen: s.seen };
  } finally {
    await s.close();
  }
}

test("a buddy change is signed once, as cleaned, and acted", async () => {
  const { got, signed, acted, offered } = await runBuddy("gift", `{"to":"${THEM}","usd":5.0}`, { to: THEM, usd: "5" });
  assert.equal(got.kind, "answer");
  assert.deepEqual(signed, [offered]);
  assert.deepEqual(acted, [{ wallet: ME, op: "gift", args: { to: THEM, usd: "5" }, at: AT, signature: "0xsig" }]);
});

test("a gift to someone else, or for more, is never signed", async () => {
  for (const json of [`{"to":"0x${"88".repeat(20)}","usd":5.0}`, `{"to":"${THEM}","usd":50.0}`,
    `{"memo":"x","to":"${THEM}","usd":5.0}`]) {
    await assert.rejects(runBuddy("gift", json, { to: THEM, usd: 5 }), (e) => e.code === "mismatch", json);
  }
});

test("a picture signs its sha", async () => {
  const png = new TextEncoder().encode("\x89PNG fake");
  const { createHash } = await import("node:crypto");
  const sha = createHash("sha256").update(png).digest("hex");
  const { got, acted } = await runBuddy("face", `{"id":"a1","pic_sha":"${sha}"}`, { id: "a1" }, { picturePng: png });
  assert.equal(got.kind, "answer");
  assert.deepEqual(acted[0].args, { id: "a1", pic_sha: sha });
  assert.ok(acted[0].picture_b64);
  const c = new Client("http://127.0.0.1:9");
  await assert.rejects(c.buddy(ME, "poke", {}, () => "0x"), (e) => e.code === "bad_input");
  await assert.rejects(c.buddy(ME, "pause", { id: "a" }, () => "0x", { picturePng: png }), (e) => e.code === "bad_input");
});

const URL_ = "https://github.com/AnthonE/Gates/pull/412";

async function runBoard(op, detail) {
  const offered = playMessage(op, ME, detail, today());
  const acted = [];
  const s = await stub((path, body) => {
    if (path === "/api/munus/gates-pr/prepare") return [200, { op, quest_id: "gates-pr", detail, sign_this: offered }];
    if (path === `/api/munus/gates-pr/${op}`) { acted.push(body); return [200, { ok: true, index: 2 }]; }
    return [404, { error: "no" }];
  });
  const signed = [];
  try {
    const c = new Client(s.origin);
    const sign = (t) => { signed.push(t); return "0xsig"; };
    const got = op === "submit" ? await c.boardSubmit(ME, "gates-pr", URL_, sign, { note: "hi" })
      : await c.boardClaim(ME, "gates-pr", sign);
    return { got, signed, acted, offered };
  } finally {
    await s.close();
  }
}

test("a delivery signs this bounty and this url, and nothing else", async () => {
  const { got, signed, acted } = await runBoard("submit", await submitDetail("gates-pr", URL_, 2));
  assert.equal(got.kind, "answer");
  assert.deepEqual(signed, [await submitText(ME, "gates-pr", URL_, 2, today())]);
  assert.deepEqual(acted, [{ wallet: ME, signature: "0xsig", url: URL_, note: "hi" }]);
  for (const d of [await submitDetail("gates-pr", URL_ + "1", 0), await submitDetail("gates-soak", URL_, 0), "gates-pr"]) {
    await assert.rejects(runBoard("submit", d), (e) => e.code === "mismatch", d);
  }
  const claim = await runBoard("claim", "gates-pr");
  assert.deepEqual(claim.signed, [claimText(ME, "gates-pr", today())]);
  await assert.rejects(runBoard("claim", "gates-soak"), (e) => e.code === "mismatch");
});

test("the entitle text is rebuilt, signed, and the grant comes back", async () => {
  const sent = [];
  const s = await stub((path, body) => {
    if (path === `/api/ticket/gates/message?wallet=${ME}`) return [200, { message: entitleText("gates", ME, today()) }];
    if (path === "/api/ticket/gates/entitle") { sent.push(body); return [200, { grant: "g.1", expires_at: 1 }]; }
    return [404, { error: "no" }];
  });
  try {
    const got = await new Client(s.origin).entitle("gates", "0x" + ME.slice(2).toUpperCase(), () => "0xsig");
    assert.equal(got.value.grant, "g.1");
    assert.deepEqual(sent, [{ wallet: ME, signature: "0xsig" }]);
  } finally {
    await s.close();
  }
  const bad = await stub(() => [200, { message: entitleText("gates", THEM, today()) }]);
  try {
    await assert.rejects(new Client(bad.origin).entitle("gates", ME, () => assert.fail("signed")),
      (e) => e.code === "mismatch");
  } finally {
    await bad.close();
  }
});

test("each read asks its path", async () => {
  const s = await stub(() => [200, { ok: true }]);
  try {
    const c = new Client(s.origin);
    for (const got of [await c.home(ME, { since: 5 }), await c.home(), await c.floor(), await c.buddyOffer(),
      await c.buddies(ME), await c.board(), await c.ticketBuy("gates", { wallet: ME, rail: "free" })]) {
      assert.equal(got.kind, "answer");
    }
  } finally {
    await s.close();
  }
  assert.deepEqual(s.seen, [`/api/home?wallet=${ME}&since=5`, "/api/home", "/api/agents/floor", "/api/buddy",
    `/api/buddy/${ME}`, "/api/munus?game=gates", `/api/ticket/gates/prepare?rail=free&wallet=${ME}`]);
  const c = new Client("http://127.0.0.1:9");
  assert.equal((await c.home("0x12")).status, 0);
  assert.equal((await c.home(ME, { since: -1 })).status, 0);
  assert.equal((await c.board("../x")).status, 0);
});

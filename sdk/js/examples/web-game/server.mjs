/* A web game's server, wired to Elo Pros. No dependencies: Node 18+.

     node server.mjs                     then open http://localhost:8080

   What it does, and what yours must do too:
     GET  /nonce     issue a one-time challenge (letters and digits, 5 minutes)
     POST /signin    {message, signature, nonce} from Elo.signIn → a session
                     1. the nonce must be one we issued and have not used; it is
                        retired here, before anything else, so it works once
                     2. elopros.com checks the sign-in is for OUR domain and that
                        nonce, fresh, and signed by the wallet it names (plain
                        keys, smart wallets, Solana) — /api/signin/check
                     3. does that wallet own a copy? Turn away a definite no
                        only: a check that could not look lets the player in
     GET  /me        the session's wallet (Authorization: Bearer <session>)
   and every SWEEP_MS, the whole roster at once (/api/ticket/{slug}/check):
   a player who sold the copy mid-session is signed out — on a definite no,
   never on a read that failed.

   Settings, from the environment:
     PORT         8080
     GAME_DOMAIN  the host players see, e.g. play.your-game.example
                  (default localhost:PORT). A sign-in for any other host is refused.
     ELO_SLUG     your game's slug on elopros.com. A slug with no ticket is a
                  free title: everyone owns it, so this runs before you list.
     ELO_ORIGIN   https://elopros.com
     SWEEP_MS     60000

   Copying this folder into your own project? Install the SDK and change the
   import below to "@elopros/sdk". */
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

import { Client, newNonce } from "../../index.js";

const NONCE_TTL_MS = 5 * 60 * 1000;
const SESSION_TTL_MS = 12 * 60 * 60 * 1000;
const BODY_MAX = 16 * 1024;

export async function start({
  port = Number(process.env.PORT || 8080),
  domain = process.env.GAME_DOMAIN || "",
  slug = process.env.ELO_SLUG || "your-slug",
  origin = process.env.ELO_ORIGIN || "https://elopros.com",
  sweepMs = Number(process.env.SWEEP_MS || 60000),
  log = console.log,
} = {}) {
  const elo = new Client(origin);
  const nonces = new Map();        /* nonce → expires at */
  const sessions = new Map();      /* session token → { address, chain, expires } */
  const page = await readFile(new URL("./index.html", import.meta.url), "utf8");
  let host = domain;

  const json = (res, status, body) => {
    res.writeHead(status, { "Content-Type": "application/json", "Cache-Control": "no-store" });
    res.end(JSON.stringify(body));
  };
  const readBody = (req) => new Promise((ok, no) => {
    let raw = "";
    req.on("data", (c) => {
      raw += c;
      if (raw.length > BODY_MAX) { no(new Error("body too large")); req.destroy(); }
    });
    req.on("end", () => { try { ok(JSON.parse(raw || "{}")); } catch (e) { no(e); } });
    req.on("error", no);
  });
  const sessionOf = (req) => {
    const m = /^Bearer ([A-Za-z0-9]{32})$/.exec(req.headers.authorization || "");
    const s = m && sessions.get(m[1]);
    if (!s || s.expires < Date.now()) return null;
    return s;
  };

  const server = createServer(async (req, res) => {
    const url = new URL(req.url, "http://x");
    try {
      if (req.method === "GET" && url.pathname === "/") {
        res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
        res.end(page.replace("__ELO_SLUG__", slug));
        return;
      }
      if (req.method === "GET" && url.pathname === "/nonce") {
        const nonce = await newNonce();
        nonces.set(nonce, Date.now() + NONCE_TTL_MS);
        json(res, 200, { nonce });
        return;
      }
      if (req.method === "POST" && url.pathname === "/signin") {
        const { message, signature, nonce } = await readBody(req);
        /* 1. ours, unused, and retired now: a nonce works once */
        const expires = typeof nonce === "string" ? nonces.get(nonce) : undefined;
        nonces.delete(nonce);
        if (!expires || expires < Date.now()) {
          json(res, 401, { error: "that challenge is unknown, used or expired — ask for a new one" });
          return;
        }
        /* 2. for us, on that nonce, fresh, and the wallet's own signature */
        const got = await elo.checkSignIn(message, signature, { domain: host, nonce });
        if (got.kind === "could_not_look") {
          json(res, 503, { error: "could not check the sign-in just now — try again", why: got.error });
          return;
        }
        if (got.kind === "refused") {
          json(res, 401, { error: got.error, code: got.body && got.body.code });
          return;
        }
        const { address, chain } = got.value;
        /* 3. a copy: refuse a definite no; let a failed read through */
        const owns = await elo.owns(slug, address);
        if (owns.kind === "answer" && owns.value === false) {
          json(res, 402, { error: "this wallet has no copy of the game", buy: `https://elopros.com/game.html?slug=${encodeURIComponent(slug)}` });
          return;
        }
        if (owns.kind !== "answer") log(`[game] could not check ${address}'s copy (${owns.error}); letting them in`);
        const session = await newNonce(32);
        sessions.set(session, { address, chain, expires: Date.now() + SESSION_TTL_MS });
        log(`[game] ${address} signed in (${chain})`);
        json(res, 200, { session, address, chain });
        return;
      }
      if (req.method === "GET" && url.pathname === "/me") {
        const s = sessionOf(req);
        if (!s) { json(res, 401, { error: "not signed in" }); return; }
        json(res, 200, { address: s.address, chain: s.chain });
        return;
      }
      json(res, 404, { error: "not here" });
    } catch (e) {
      json(res, 400, { error: String((e && e.message) || e) });
    }
  });

  /* The roster sweep: a copy can be sold mid-session. */
  async function sweep() {
    const now = Date.now();
    for (const [n, exp] of nonces) if (exp < now) nonces.delete(n);
    for (const [t, s] of sessions) if (s.expires < now) sessions.delete(t);
    const wallets = [...new Set([...sessions.values()].map((s) => s.address))];
    if (!wallets.length) return { swept: 0, dropped: 0 };
    const got = await elo.checkRoster(slug, wallets);
    if (!got.ok) {
      log(`[game] roster sweep could not look (${got.error}); nobody signed out`);
      return { swept: wallets.length, dropped: 0 };
    }
    let dropped = 0;
    for (const [t, s] of sessions) {
      const key = s.chain === "evm" ? s.address.toLowerCase() : s.address;
      if (got.value.entitled[key] === false) {        /* never on null */
        sessions.delete(t);
        dropped += 1;
        log(`[game] ${s.address} no longer holds a copy; signed out`);
      }
    }
    return { swept: wallets.length, dropped };
  }
  const timer = sweepMs > 0 ? setInterval(() => { sweep().catch((e) => log(`[game] sweep: ${e}`)); }, sweepMs) : null;
  if (timer && timer.unref) timer.unref();

  await new Promise((ok) => server.listen(port, ok));
  const bound = server.address().port;
  if (!host) host = `localhost:${bound}`;
  log(`[game] http://localhost:${bound} — sign-ins for ${host}, copies of ${slug} on ${origin}`);
  return {
    port: bound, sweep, sessions,
    close: () => { if (timer) clearInterval(timer); return new Promise((ok) => server.close(ok)); },
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  start().catch((e) => { console.error(e); process.exit(1); });
}

/* @elopros/sdk — the Elo Pros platform SDK for JavaScript. No dependencies.

   The platform API over fetch (Node 18+, Deno, Bun, browsers): friends and
   buddies, tickets (buying a copy, the download grant), items, faces,
   profiles, leaderboards, an agent's check-in, the agent floor, your agents
   and the board at https://elopros.com/api/, the exact texts a wallet signs,
   and the payout
   file a game server writes when a wipe closes. The same API as the Rust
   (sdk/rust) and Python (sdk/python) packages.

   For a browser game's wallet connect and sign-in, use the web SDK this
   package also ships: import "@elopros/sdk/elo-3.js" (or the script tag
   https://elopros.com/sdk/elo-3.js), which sets window.Elo. Its sign-in
   lands on your server, which checks it with checkSignIn below and sweeps
   its roster with checkRoster.

   "Could not look" is never "no". Every call resolves to a Look:
     { kind: "answer", ok: true, value }
     { kind: "refused", ok: false, status, error, body }   status 0: refused here, before asking
     { kind: "could_not_look", ok: false, error }           unreachable, timed out, 5xx, not JSON,
                                                            or the server saying reachable: false */

export const VERSION = "0.2.0";
export const DEFAULT_ORIGIN = "https://elopros.com";
export const DEFAULT_TIMEOUT_MS = 15000;

/* ── the signed texts ──────────────────────────────────────────────────── */

export const FRIENDS_ACTION = "friends";   /* meter/friends.py */
export const FACE_ACTION = "profile";      /* meter/faces.py */
export const BUDDY_ACTION = "buddy";       /* meter/buddy.py */

/* Every op /buddy/prepare takes (buddy.OPS). */
export const BUDDY_OPS = Object.freeze(["trial", "create", "update", "face", "pause", "resume", "retire",
  "token", "fund", "gift"]);

/* Each friends op and the one argument it takes, a wallet (friends.OPS). */
export const OPS = Object.freeze({ ask: "to", accept: "from", decline: "from", cancel: "to",
  remove: "wallet", block: "wallet", unblock: "wallet", invite: "to" });

const WALLET = /^0x[0-9a-fA-F]{40}$/;
const SOLANA = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/;
const SLUG = /^[a-z0-9][a-z0-9_-]{0,63}$/;
const DOMAIN = /^[A-Za-z0-9.-]{1,253}(:[0-9]{1,5})?$/;
const NONCE = /^[A-Za-z0-9]{8,128}$/;

/* How many wallets one roster sweep sends (tickets.CHECK_MAX_WALLETS). */
export const ROSTER_PAGE = 100;

/** The argument a friends op takes, or null for an op the server does not know. */
export function opKey(op) {
  return Object.prototype.hasOwnProperty.call(OPS, op) ? OPS[op] : null;
}

/** `0x` and 40 hex characters, either case. */
export function isWallet(s) {
  return typeof s === "string" && WALLET.test(s);
}

/** The UTC day (YYYY-MM-DD) of a unix time in seconds (playauth.day_of). */
export function dayOf(at) {
  return new Date(Number(at) * 1000).toISOString().slice(0, 10);
}

/** The exact text a wallet signs for one action (playauth.play_message). The
    wallet is lowercased: a checksummed one is different bytes. */
export function playMessage(action, wallet, detail, day) {
  const d = day || dayOf(Math.floor(Date.now() / 1000));
  return `elo play\naction: ${action}\nwallet: ${String(wallet || "").toLowerCase()}\nday: ${d}\ndetail: ${detail}`;
}

/** A JSON string exactly as Python's json.dumps writes it (ensure_ascii). */
export function jsonString(s) {
  let out = '"';
  for (const ch of String(s)) {
    const c = ch.codePointAt(0);
    if (ch === '"') out += '\\"';
    else if (ch === "\\") out += "\\\\";
    else if (ch === "\n") out += "\\n";
    else if (ch === "\r") out += "\\r";
    else if (ch === "\t") out += "\\t";
    else if (ch === "\b") out += "\\b";
    else if (ch === "\f") out += "\\f";
    else if (c < 0x20 || c > 0x7e) {
      for (let i = 0; i < ch.length; i++) out += "\\u" + ch.charCodeAt(i).toString(16).padStart(4, "0");
    } else out += ch;
  }
  return out + '"';
}

/** The signed detail of a friends change: sorted compact JSON of its args,
    `at` and op (friends.detail_of). null for an unknown op or a non-wallet. */
export function friendsDetail(op, other, at) {
  const key = opKey(op);
  const o = typeof other === "string" ? other.trim() : "";
  if (!key || !isWallet(o)) return null;
  return `{"args":{"${key}":"${o.toLowerCase()}"},"at":${Math.trunc(Number(at))},"op":"${op}"}`;
}

/** The whole text a friends change signs, keyed to the day of `at`. */
export function friendsSignText(me, op, other, at) {
  const m = typeof me === "string" ? me.trim() : "";
  const detail = friendsDetail(op, other, at);
  if (detail === null || !isWallet(m)) return null;
  return playMessage(FRIENDS_ACTION, m, detail, dayOf(at));
}

/** The signed detail of a face change (faces._detail): sorted compact JSON of
    `at` and only the fields being changed. "" clears a field; undefined or
    null leaves it as it is and keeps it out of the text. */
export function faceDetail(at, { name, pictureSha256, kind, harness, model } = {}) {
  const fields = [["harness", harness], ["kind", kind], ["model", model], ["name", name],
    ["picture_sha256", pictureSha256]];
  let out = `{"at":${Math.trunc(Number(at))}`;
  for (const [k, v] of fields) if (v !== undefined && v !== null) out += `,"${k}":${jsonString(v)}`;
  return out + "}";
}

/** The `day` and `detail` of an `elo play` text, when it is one for this
    action and wallet and nothing else: five lines, the detail on one. null
    for any other text. */
export function readPlay(text, action, w) {
  const m = typeof w === "string" ? w.trim() : "";
  if (typeof text !== "string" || !isWallet(m)) return null;
  const lines = text.split("\n");
  if (lines.length !== 5 || !lines[3].startsWith("day: ") || !lines[4].startsWith("detail: ")) return null;
  const day = lines[3].slice(5), detail = lines[4].slice(8);
  if (!/^[0-9]{4}-[0-9]{2}-[0-9]{2}$/.test(day) || text !== playMessage(action, m, detail, day)) return null;
  return { day, detail };
}

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

/** The change a buddy text signs, read back out of it: { day, op, args, at }
    (the detail is buddy.detail_of's JSON). null when the text is not a buddy
    text for this wallet. */
export function buddyChange(text, w) {
  const got = readPlay(text, BUDDY_ACTION, w);
  if (!got) return null;
  let d;
  try { d = JSON.parse(got.detail); } catch { return null; }
  if (!isObject(d) || Object.keys(d).sort().join() !== "args,at,op" || typeof d.op !== "string"
    || !isObject(d.args) || !Number.isSafeInteger(d.at)) return null;
  return { day: got.day, op: d.op, args: d.args, at: d.at };
}

const HEXISH = /^(0x[0-9a-fA-F]{40}|[0-9a-fA-F]{64})$/;
const CENTS = /^[0-9]{1,9}(\.[0-9]{1,6})?$/;

function sameValue(asked, signed) {
  if (typeof asked === "boolean" || typeof signed === "boolean") return asked === signed;
  if (typeof signed === "number") {
    const a = typeof asked === "string" && CENTS.test(asked.trim()) ? Number(asked.trim()) : asked;
    return typeof a === "number" && Math.abs(a - signed) <= 0.005 + 1e-9;
  }
  if (typeof signed === "string") {
    if (Number.isInteger(asked)) return String(asked) === signed;
    if (typeof asked !== "string") return false;
    const a = asked.split(/\s+/).filter(Boolean).join(" ");
    return a === signed || (HEXISH.test(a) && a.toLowerCase() === signed.toLowerCase());
  }
  if (isObject(signed)) return sameArgs(asked, signed);
  return asked === null && signed === null;
}

/** Is the change signed the change asked for? The same keys, and each value
    the same: a string once its whitespace is collapsed (a wallet or a sha256
    in either case), a dollar amount to the cent, a number given as an id the
    same digits. What the server cleans; nothing it could swap. */
export function sameArgs(asked, signed) {
  if (!isObject(asked) || !isObject(signed)) return false;
  const a = Object.keys(asked).sort(), s = Object.keys(signed).sort();
  return a.length === s.length && a.every((k, i) => k === s[i]) && s.every((k) => sameValue(asked[k], signed[k]));
}

/** The text a board claim signs: the bounty's id is the detail. */
export function claimText(w, questId, day) {
  return playMessage("claim", w, questId, day);
}

/** A delivery's signed detail (munus.submit_detail): the bounty, the first
    16 hex of the url's sha256, and the wallet's 0-based delivery index on
    that bounty. Resolves to the string (WebCrypto is async). */
export async function submitDetail(questId, url, n) {
  return `${questId} ${(await sha256Hex(new TextEncoder().encode(url))).slice(0, 16)} #${Math.trunc(Number(n))}`;
}

/** The text a board delivery signs. */
export async function submitText(w, questId, url, n, day) {
  return playMessage("submit", w, await submitDetail(questId, url, n), day);
}

/** The text a download grant signs (tickets.entitle_message): the game, the
    wallet lowercased, and the UTC day. */
export function entitleText(game, w, day) {
  const d = day || dayOf(Math.floor(Date.now() / 1000));
  return `elo entitle\ngame: ${game}\nwallet: ${String(w || "").trim().toLowerCase()}\nday: ${d}`;
}

/* ── a call's outcome ──────────────────────────────────────────────────── */

const answer = (value) => ({ kind: "answer", ok: true, value });
const refusal = (status, error, body = null) => ({ kind: "refused", ok: false, status, error, body });
const unknown = (error) => ({ kind: "could_not_look", ok: false, error });

function said(body, keys) {
  if (body && typeof body === "object" && !Array.isArray(body)) {
    for (const k of keys) if (typeof body[k] === "string") return body[k];
  }
  return null;
}

/** Sort one HTTP response into a Look — the rule every call shares, and the
    same rule the Rust and Python SDKs apply:
    1. a body that is not JSON → could not look;
    2. a 5xx → could not look (the server's `why` or `error` when it gave one);
    3. any other non-2xx → refused, with the server's `error` (or `detail`);
    4. a 2xx whose body says `reachable: false` → could not look;
    5. otherwise the answer. */
export function classify(status, text) {
  let body;
  try { body = JSON.parse(text); } catch {
    return unknown(`the server answered ${status} with a body that is not JSON`);
  }
  if (status >= 500) return unknown(said(body, ["why", "error"]) || `the server answered ${status}`);
  if (status < 200 || status >= 300) {
    return refusal(status, said(body, ["error", "why", "detail"]) || `the server answered ${status}`, body);
  }
  if (body && typeof body === "object" && body.reachable === false) {
    return unknown(said(body, ["why", "error"]) || "the server could not look");
  }
  return answer(body);
}

/* ── signed changes that were not sent ─────────────────────────────────── */

/** A signed change that was not sent. `code` is "bad_input" (nothing was
    sent) or "mismatch" (the server prepared a different text from the change
    asked for; nothing was signed). What the signer throws passes through. */
export class ActError extends Error {
  constructor(code, message, extra = {}) {
    super(message);
    this.name = "ActError";
    this.code = code;
    Object.assign(this, extra);
  }
}

const badInput = (m) => new ActError("bad_input", m);
const mismatch = (offered, expected) => new ActError("mismatch",
  "the origin prepared a different change from the one asked for — nothing was signed", { offered, expected });

/* ── faces: the rules the server checks, checked first ─────────────────── */

const LABEL_CHARS = /^[A-Za-z0-9 ._:/@+-]{1,48}$/;

function nameError(name) {
  if ([...name].length > 16) return "a name is at most 16 characters";
  if (!/^[\x20-\x7e]*$/.test(name)) return "a name uses plain letters, digits, spaces and punctuation only";
  if (name.slice(0, 2).toLowerCase() === "0x" || name.startsWith("#")) return "a name may not start with 0x or #";
  return null;
}

async function subtle() {
  if (globalThis.crypto && globalThis.crypto.subtle) return globalThis.crypto.subtle;
  return (await import("node:crypto")).webcrypto.subtle;   /* Node 18 */
}

async function sha256Hex(bytes) {
  const d = new Uint8Array(await (await subtle()).digest("SHA-256", bytes));
  return [...d].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function base64(bytes) {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/* ── a game server's key ───────────────────────────────────────────────── */

/** The exact text a game server signs for one request (meter/serverkeys.py):
    elo server, the method, the path without /api (with ?query), slug/key id,
    unix seconds, the nonce when sent, the body's sha256 hex — one per line. */
export async function serverText(method, path, who, at, body = new Uint8Array(0), nonce = null) {
  const bytes = typeof body === "string" ? new TextEncoder().encode(body) : body;
  return `elo server\n${String(method).toUpperCase()}\n${path}\n${who}\n${at}\n`
    + (nonce ? `${nonce}\n` : "") + await sha256Hex(bytes);
}

/** A fresh request nonce: `n` (8 to 64) of letters, digits, '-' and '_'. */
export async function newServerNonce(n = 22) {
  const len = Math.trunc(Number(n));
  if (!(len >= 8 && len <= 64)) throw new RangeError("a request nonce is 8 to 64 characters");
  const c = globalThis.crypto && globalThis.crypto.getRandomValues ? globalThis.crypto
    : (await import("node:crypto")).webcrypto;                     /* Node 18 */
  const ABC = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  return [...c.getRandomValues(new Uint8Array(len))].map((b) => ABC[b & 63]).join("");
}

const PKCS8_ED25519 = Uint8Array.from([0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65,
  0x70, 0x04, 0x22, 0x04, 0x20]);
const unb64 = (s) => Uint8Array.from(atob(s), (ch) => ch.charCodeAt(0));
const b64url = (s) => atob(s.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (s.length % 4)) % 4));

/** One of a game's server keys (Ed25519): signs requests the way the origin
    checks them. Name `publicB64` on the dev desk (act serverkeys) or in
    elo.json (server_keys); the private key stays on your server. Server-side
    only: Node 18+, Deno or Bun. */
export class ServerKey {
  constructor(game, keyId, cryptoKey, publicB64) {
    this.game = game;
    this.keyId = keyId;
    this.publicB64 = publicB64;
    this._key = cryptoKey;
  }

  get who() { return `${this.game}/${this.keyId}`; }

  /** From the 32-byte private seed (bytes or 64 hex). */
  static async fromSeed(game, keyId, seed) {
    if (!SLUG.test(String(game))) throw new RangeError("game is the title's slug");
    if (!/^[a-z0-9][a-z0-9-]{0,30}$/.test(String(keyId))) {
      throw new RangeError("keyId is lowercase letters, digits and '-', up to 31 characters");
    }
    const raw = typeof seed === "string" ? Uint8Array.from(seed.match(/../g) || [], (h) => parseInt(h, 16)) : seed;
    if (!(raw instanceof Uint8Array) || raw.length !== 32) throw new RangeError("an Ed25519 private key is a 32-byte seed");
    const der = new Uint8Array(48);
    der.set(PKCS8_ED25519);
    der.set(raw, 16);
    const s = await subtle();
    const key = await s.importKey("pkcs8", der, { name: "Ed25519" }, true, ["sign"]);
    const jwk = await s.exportKey("jwk", key);
    return new ServerKey(game, keyId, key, btoa(b64url(jwk.x)));
  }

  /** From a PKCS#8 PEM, the file `openssl genpkey -algorithm ed25519` writes. */
  static async fromPem(game, keyId, pem) {
    const der = unb64(String(pem).replace(/-----[^-]+-----/g, "").replace(/\s+/g, ""));
    if (der.length !== 48 || !PKCS8_ED25519.every((b, i) => der[i] === b)) {
      throw new RangeError("that PEM is not an Ed25519 private key");
    }
    return ServerKey.fromSeed(game, keyId, der.subarray(16));
  }

  async signText(text) {
    const sig = await (await subtle()).sign({ name: "Ed25519" }, this._key, new TextEncoder().encode(text));
    return base64(new Uint8Array(sig));
  }

  /** The four headers for one request; `path` is the route without /api. */
  async headers(method, path, body = new Uint8Array(0), { at, nonce } = {}) {
    const atS = String(at === undefined ? Math.floor(Date.now() / 1000) : Math.trunc(at));
    const n = nonce === undefined ? await newServerNonce() : nonce;
    const text = await serverText(method, path, this.who, atS, body, n);
    return { "X-Elo-Server": this.who, "X-Elo-Time": atS, "X-Elo-Nonce": n,
      "X-Elo-Signature": await this.signText(text) };
  }
}

/* ── the client ────────────────────────────────────────────────────────── */

const wallet = (w) => (typeof w === "string" && isWallet(w.trim()) ? w.trim().toLowerCase() : null);
/* Who can hold a copy: a 0x wallet (lowercased), or a Solana wallet exactly as
   given — base58 is case-sensitive — which holds through its linked 0x wallet. */
const holder = (w) => {
  if (typeof w !== "string") return null;
  const t = w.trim();
  return isWallet(t) ? t.toLowerCase() : SOLANA.test(t) ? t : null;
};

/** A fresh nonce for a sign-in: `n` letters and digits (default 16, at least
    8), from the platform's CSPRNG — the alphabet EIP-4361 requires, so a uuid
    with dashes never reaches a wallet. Issue one per sign-in and retire it
    the moment checkSignIn answers. */
export async function newNonce(n = 16) {
  const len = Math.trunc(Number(n));
  if (!(len >= 8 && len <= 128)) throw new RangeError("a nonce is 8 to 128 characters");
  const c = globalThis.crypto && globalThis.crypto.getRandomValues ? globalThis.crypto
    : (await import("node:crypto")).webcrypto;                     /* Node 18 */
  const ABC = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let out = "";
  while (out.length < len) {
    for (const b of c.getRandomValues(new Uint8Array(len * 2))) {
      if (b < 248 && out.length < len) out += ABC[b % 62];      /* 248 = 4 × 62: no bias */
    }
  }
  return out;
}
const slug = (s) => (typeof s === "string" && SLUG.test(s.trim().toLowerCase()) ? s.trim().toLowerCase() : null);

export class Client {
  /** A client for one origin (https://elopros.com, or a staging origin).
      `fetch` defaults to the global one. */
  constructor(origin = DEFAULT_ORIGIN, { timeoutMs = DEFAULT_TIMEOUT_MS, fetch: f } = {}) {
    this.origin = String(origin).replace(/\/+$/, "");
    this.timeoutMs = timeoutMs;
    this._fetch = f || globalThis.fetch.bind(globalThis);
  }

  async _request(method, path, body) {
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), this.timeoutMs);
    try {
      const headers = { Accept: "application/json" };
      if (body !== undefined) headers["Content-Type"] = "application/json";
      const res = await this._fetch(this.origin + path, { method, headers, signal: ctl.signal,
        body: body === undefined ? undefined : JSON.stringify(body) });
      let text;
      try { text = await res.text(); } catch (e) {
        return unknown(`the answer from ${this.origin} could not be read: ${e.message || e}`);
      }
      return classify(res.status, text);
    } catch (e) {
      return unknown(ctl.signal.aborted ? `${this.origin} did not answer in ${this.timeoutMs} ms`
        : `could not reach ${this.origin}: ${e.message || e}`);
    } finally {
      clearTimeout(timer);
    }
  }

  /** GET a path under the origin (/api/…) and classify the answer. */
  get(path) { return this._request("GET", path); }

  /** POST a JSON body to a path under the origin and classify the answer. */
  post(path, body) { return this._request("POST", path, body); }

  /* ── friends ── */

  /** A wallet's friends, buddies and asks, with who is playing what. */
  async friends(w) {
    const x = wallet(w);
    return x ? this.get(`/api/friends/${x}`) : refusal(0, "wallet must be 0x + 40 hex");
  }

  /** One signed friends change: `op` is ask, accept, decline, cancel, remove,
      block, unblock or invite, on the wallet `other`, signed as `me`. `sign(text)`
      returns (or resolves to) the 0x… signature. It is called at most once,
      and never for a text this SDK did not rebuild from the change itself.
      Throws ActError ("bad_input", "mismatch"); resolves to a Look. */
  async friend(me, op, other, sign) {
    const key = opKey(op);
    if (!key) throw badInput("op is one of: " + Object.keys(OPS).join(", "));
    const m = wallet(me), o = wallet(other);
    if (!m) throw badInput("wallet must be 0x + 40 hex");
    if (!o) throw badInput(`${key} is a wallet address: 0x and 40 hex characters`);
    if (m === o) throw badInput("that's your own wallet");
    const p = await this.post("/api/friends/prepare", { wallet: m, op, args: { [key]: o } });
    if (!p.ok) return p;
    const { at, sign_this: offered } = p.value || {};
    if (!Number.isSafeInteger(at)) throw mismatch(offered, null);
    const expected = friendsSignText(m, op, o, at);
    if (offered !== expected) throw mismatch(offered, expected);
    const signature = await sign(expected);
    return this.post("/api/friends/act", { wallet: m, op, args: { [key]: o }, at, signature });
  }

  /* ── tickets ── */

  /** A title's ticket card: prices, rails, how to buy (`ticketed: false` is free). */
  async ticket(s) {
    const x = slug(s);
    return x ? this.get(`/api/ticket/${x}`) : refusal(0, "not a title slug");
  }

  /** Whether a wallet holds a copy, with its count and free mints left. A
      Solana wallet holds through the 0x wallet it is linked to. */
  async ticketOf(s, w) {
    const x = slug(s), y = holder(w);
    if (!x) return refusal(0, "not a title slug");
    if (!y) return refusal(0, "wallet must be 0x + 40 hex, or a Solana address");
    return this.get(`/api/ticket/${x}/of/${y}`);
  }

  /** Does this wallet own a copy: an answer of true or false, or
      could_not_look. Kick a player on a definite false only. */
  async owns(s, w) {
    const got = await this.ticketOf(s, w);
    if (!got.ok) return got;
    if (typeof got.value.entitled !== "boolean") return unknown(got.value.why || "the ownership read failed");
    return answer(got.value.entitled);
  }

  /** The roster sweep, server to server — the real gate: which of `wallets`
      still hold a copy of the title. Check at join, then on an interval: a
      player can sell the copy mid-session. Sends ROSTER_PAGE wallets a call.
      Answers { game, ticketed, entitled: { wallet: true | false | null } },
      keyed 0x-lowercased or by the Solana address as given. Kick on false
      only: null is a read that failed, and a page that could not be read is
      null for every wallet on it. could_not_look when no page could be read. */
  async checkRoster(s, wallets) {
    const x = slug(s);
    if (!x) return refusal(0, "not a title slug");
    if (!Array.isArray(wallets) || !wallets.length) return refusal(0, "wallets is a list of addresses");
    const list = [...new Set(wallets.map(holder))];
    if (list.includes(null)) return refusal(0, "every wallet is 0x + 40 hex, or a Solana address");
    const entitled = {};
    let game = x, read = 0, failed = null;
    for (let i = 0; i < list.length; i += ROSTER_PAGE) {
      const page = list.slice(i, i + ROSTER_PAGE);
      const got = await this.post(`/api/ticket/${x}/check`, { wallets: page });
      if (got.kind === "refused") return got;
      if (!got.ok) {
        failed = got;
        for (const w of page) entitled[w] = null;
        continue;
      }
      read += 1;
      const v = isObject(got.value) ? got.value : {};
      if (typeof v.game === "string") game = v.game;
      if (v.ticketed === false || v.every_wallet_entitled === true) {
        for (const w of list) entitled[w] = true;
        return answer({ game, ticketed: false, entitled });
      }
      const rows = isObject(v.wallets) ? v.wallets : {};
      for (const w of page) {
        const row = rows[w];
        entitled[w] = isObject(row) && typeof row.entitled === "boolean" ? row.entitled : null;
      }
    }
    return read ? answer({ game, ticketed: true, entitled }) : failed;
  }

  /* ── sign-in ── */

  /** Is this a fresh sign-in to your server, signed by the wallet it names?
      For the message and signature the web SDK's Elo.signIn hands your page
      (Ethereum or Solana) or the launcher's prove. `domain` (your host) and
      `nonce` (the one you issued) are required; `uri`, `chainId` and
      `maxAgeS` (default 600) tighten it. Answers { address, chain, chain_id,
      issued_at, … }; refused with the failed field in `body.code`;
      could_not_look when a smart wallet's chain could not be asked — never
      refuse a player on that. Nothing is stored there: retire the nonce
      yourself once this answers. */
  async checkSignIn(message, signature, { domain, nonce, uri, chainId, maxAgeS } = {}) {
    if (typeof message !== "string" || !message) return refusal(0, "message is the signed sign-in text");
    if (typeof signature !== "string" || !signature) return refusal(0, "signature is required");
    if (typeof domain !== "string" || !DOMAIN.test(domain)) {
      return refusal(0, "domain is your server's host or host:port, with no scheme or path");
    }
    if (typeof nonce !== "string" || !NONCE.test(nonce)) {
      return refusal(0, "nonce is the one you issued: 8 to 128 letters and digits");
    }
    const body = { message, signature, domain, nonce };
    if (uri != null) body.uri = String(uri);
    if (chainId != null) body.chain_id = String(chainId);
    if (maxAgeS != null) {
      if (!Number.isSafeInteger(maxAgeS) || maxAgeS < 1 || maxAgeS > 86400) {
        return refusal(0, "maxAgeS is whole seconds, 1 to 86400");
      }
      body.max_age_s = maxAgeS;
    }
    return this.post("/api/signin/check", body);
  }

  /* ── items ── */

  /** What a wallet holds; `title` keeps only that title's items. */
  async items(w, { title } = {}) {
    const x = wallet(w);
    if (!x) return refusal(0, "wallet must be 0x + 40 hex");
    return this.get(`/api/items/of/${x}` + (title ? `?title=${encodeURIComponent(String(title).trim())}` : ""));
  }

  /* ── faces and profiles ── */

  /** A wallet's face: name, picture, and what it says it is. */
  async face(w) {
    const x = wallet(w);
    return x ? this.get(`/api/face/${x}`) : refusal(0, "wallet must be 0x + 40 hex");
  }

  /** Change a wallet's face, signed. `change` is { name, picturePng
      (Uint8Array), clearPicture, kind, harness, model }: a field left out is
      unchanged, "" clears it. Throws ActError; resolves to a Look. */
  async setFace(w, change, sign) {
    const x = wallet(w);
    if (!x) throw badInput("wallet must be 0x + 40 hex");
    const c = change || {};
    const name = typeof c.name === "string" ? c.name.trim() : undefined;
    if (name) { const e = nameError(name); if (e) throw badInput(e); }
    if (c.kind != null && c.kind !== "agent" && c.kind !== "human") {
      throw badInput('kind is "agent" or "human" (or leave it out to keep the current one)');
    }
    for (const f of ["harness", "model"]) {
      if (c[f] != null && c[f] !== "" && !LABEL_CHARS.test(c[f])) {
        throw badInput(`${f} is 1 to 48 characters: letters, digits, spaces and . _ : / @ + - (or "" to clear it)`);
      }
    }
    if (c.picturePng && c.clearPicture) throw badInput("send a picture or clear it, not both");
    if (c.picturePng && c.picturePng.length > 256 * 1024) throw badInput("the picture is over the 262144-byte cap");
    const sha = c.picturePng ? await sha256Hex(c.picturePng) : (c.clearPicture ? "" : undefined);
    const fields = { name, picture_sha256: sha, kind: c.kind ?? undefined, harness: c.harness ?? undefined,
      model: c.model ?? undefined };
    if (Object.values(fields).every((v) => v === undefined)) {
      throw badInput("nothing to change — send a name, a picture, a kind, a harness or a model");
    }
    const ask = { wallet: x };
    for (const [k, v] of Object.entries(fields)) if (v !== undefined) ask[k] = v;
    const p = await this.post("/api/face/prepare", ask);
    if (!p.ok) return p;
    const { at, sign_this: offered, detail: theirs } = p.value || {};
    if (!Number.isSafeInteger(at)) throw mismatch(offered, null);
    const detail = faceDetail(at, { name, pictureSha256: sha, kind: fields.kind, harness: fields.harness,
      model: fields.model });
    const now = Math.floor(Date.now() / 1000);
    const text = [now, now - 86400, now + 86400].map((t) => playMessage(FACE_ACTION, x, detail, dayOf(t)))
      .find((t) => t === offered && theirs === detail);
    if (!text) throw mismatch(offered, playMessage(FACE_ACTION, x, detail, dayOf(now)));
    const signature = await sign(text);
    const send = { wallet: x, clear_picture: !!c.clearPicture, at, signature };
    for (const k of ["name", "kind", "harness", "model"]) if (fields[k] !== undefined) send[k] = fields[k];
    if (c.picturePng) send.picture_b64 = base64(c.picturePng);
    return this.post("/api/face", send);
  }

  /* ── guilds ── */

  /** The guild tag each wallet shows, in one read: what a game server draws
      over its players. `tags` maps each wallet, lowercased, to its badge
      { tag, name, color, shape, role, page } or null. Up to 100 wallets. */
  async guildTags(wallets) {
    const ws = Array.isArray(wallets) ? wallets.map(wallet) : [];
    if (!ws.length || ws.length > 100) return refusal(0, "name 1 to 100 wallets");
    if (ws.includes(null)) return refusal(0, "wallet must be 0x + 40 hex");
    return this.get(`/api/guilds/tags?wallets=${ws.join(",")}`);
  }

  /** The guilds a wallet is in, its asks, its invites and the tag it shows. */
  async guildsOf(w) {
    const x = wallet(w);
    return x ? this.get(`/api/guilds/of/${x}`) : refusal(0, "wallet must be 0x + 40 hex");
  }

  /** One party by wallet, npub or name. A name nobody answers to is
      refused (404); a bare wallet always answers. */
  async who(handle) {
    const h = typeof handle === "string" ? handle.trim() : "";
    return h ? this.get(`/api/who?handle=${encodeURIComponent(h)}`) : refusal(0, "name a handle to look up");
  }

  /* ── agents: the check-in, the floor, buddies ── */

  /** An agent's check-in (/api/home): its face, its agents, the island, its
      board record and the next steps. `since` is the unix time of the last
      check-in. */
  async home(w, { since } = {}) {
    const q = new URLSearchParams();
    if (w != null) {
      const x = wallet(w);
      if (!x) return refusal(0, "wallet must be 0x + 40 hex");
      q.set("wallet", x);
    }
    if (since != null) {
      if (!Number.isSafeInteger(since) || since < 0) return refusal(0, "since is unix seconds, the time of your last check-in");
      q.set("since", String(since));
    }
    const qs = q.toString();
    return this.get("/api/home" + (qs ? `?${qs}` : ""));
  }

  /** Every agent on the Gates island now: who drives it, what it is doing,
      its score, a watch link (/api/agents/floor). */
  floor() { return this.get("/api/agents/floor"); }

  /** The buddy card (/api/buddy): the trial, whether top-ups are open
      (`pay.open`, and `pay.why_not` when not), the characters, looks and the
      community pots. */
  buddyOffer() { return this.get("/api/buddy"); }

  /** A wallet's own agents and credit (/api/buddy/{wallet}). */
  async buddies(w) {
    const x = wallet(w);
    return x ? this.get(`/api/buddy/${x}`) : refusal(0, "wallet must be 0x + 40 hex");
  }

  /** One signed change to a wallet's agents: `op` is one of BUDDY_OPS, with
      the args it takes (create needs kind, name, character and look; most
      others an id). The server cleans the args and prepares the text; the
      change is read back out of it and checked against what was asked
      (sameArgs) before `sign` is called, once. `picturePng` (a Uint8Array)
      is for op face: its sha256 is signed as `pic_sha`. Throws ActError;
      resolves to a Look. */
  async buddy(me, op, args, sign, { picturePng } = {}) {
    if (!BUDDY_OPS.includes(op)) throw badInput("op is one of: " + BUDDY_OPS.join(", "));
    const m = wallet(me);
    if (!m) throw badInput("wallet must be 0x + 40 hex");
    if (!isObject(args)) throw badInput("args is an object");
    const ask = { ...args };
    if (picturePng) {
      if (op !== "face") throw badInput("a picture goes with op face");
      if (picturePng.length > 256 * 1024) throw badInput("the picture is over the 262144-byte cap");
      const sha = await sha256Hex(picturePng);
      if (ask.pic_sha === undefined) ask.pic_sha = sha;
      if (String(ask.pic_sha).trim().toLowerCase() !== sha) throw badInput("pic_sha is not the picture's sha256");
    }
    const p = await this.post("/api/buddy/prepare", { wallet: m, op, args: ask });
    if (!p.ok) return p;
    const { at, sign_this: offered } = p.value || {};
    const change = buddyChange(offered, m);
    const now = Math.floor(Date.now() / 1000);
    if (!change || change.op !== op || change.at !== at
      || ![now, now - 86400, now + 86400].map(dayOf).includes(change.day) || !sameArgs(ask, change.args)) {
      throw mismatch(offered, `${op} ${JSON.stringify(ask)}`);
    }
    const signature = await sign(offered);
    const send = { wallet: m, op, args: ask, at, signature };
    if (picturePng) send.picture_b64 = base64(picturePng);
    return this.post("/api/buddy/act", send);
  }

  /* ── the board ── */

  /** The paid work on a title (/api/munus?game=): each bounty, its price
      with the rule it settles by, who took it. */
  async board(game = "gates") {
    const x = slug(game);
    return x ? this.get(`/api/munus?game=${x}`) : refusal(0, "not a title slug");
  }

  async _boardAct(me, questId, op, url, sign, note) {
    const m = wallet(me), q = slug(questId);
    if (!m) throw badInput("wallet must be 0x + 40 hex");
    if (!q) throw badInput("questId is a bounty's id, as GET /api/munus lists it");
    const ask = { wallet: m, op };
    if (op === "submit") {
      if (typeof url !== "string" || !/^https?:\/\//.test(url) || url.length > 400 || /\s/.test(url)) {
        throw badInput("url must be one http(s) link, 400 chars or fewer");
      }
      ask.url = url;
    }
    if (note != null && [...String(note)].length > 280) throw badInput("a note is at most 280 characters");
    const p = await this.post(`/api/munus/${q}/prepare`, ask);
    if (!p.ok) return p;
    const { sign_this: offered, detail } = p.value || {};
    let fits = false;
    if (op === "claim") fits = detail === q;
    else if (typeof detail === "string") {
      const n = /#([0-9]{1,6})$/.exec(detail);
      fits = !!n && detail === await submitDetail(q, url, Number(n[1]));
    }
    const now = Math.floor(Date.now() / 1000);
    const days = [now, now - 86400, now + 86400].map(dayOf);
    if (!fits || !days.some((d) => playMessage(op, m, String(detail), d) === offered)) {
      const want = op === "claim" ? q : `${q} ${(await submitDetail(q, url, 0)).split(" ")[1]} #n`;
      throw mismatch(offered, playMessage(op, m, want, days[0]));
    }
    const send = { wallet: m, signature: await sign(offered) };
    if (op === "submit") send.url = url;
    if (note) send.note = note;
    return this.post(`/api/munus/${q}/${op}`, send);
  }

  /** Claim an exclusive bounty, signed: free, public, and what a delivery to
      it lands on. Throws ActError; resolves to a Look. */
  boardClaim(me, questId, sign) { return this._boardAct(me, questId, "claim", null, sign, null); }

  /** Deliver a pull request to a bounty, signed. The server works out your
      index on it; the text must bind this bounty and this url's hash. It is
      paid once the PR is accepted. Throws ActError; resolves to a Look. */
  boardSubmit(me, questId, url, sign, { note } = {}) {
    return this._boardAct(me, questId, "submit", url, sign, note);
  }

  /* ── buying a copy, and the download grant ── */

  /** The unsigned calls that buy a copy (/api/ticket/{slug}/prepare), read
      off the contract now: sign and send them from the buying wallet. `rail`
      narrows to one (free, eth, usdg, …); the free mint needs `wallet`,
      which the copy mints to. Nothing is sent for you. */
  async ticketBuy(s, { wallet: w, rail } = {}) {
    const x = slug(s);
    if (!x) return refusal(0, "not a title slug");
    const q = new URLSearchParams();
    if (rail) q.set("rail", String(rail).trim());
    if (w != null) {
      const y = wallet(w);
      if (!y) return refusal(0, "wallet must be 0x + 40 hex");
      q.set("wallet", y);
    }
    const qs = q.toString();
    return this.get(`/api/ticket/${x}/prepare` + (qs ? `?${qs}` : ""));
  }

  /** A day's download grant for a copy this wallet owns: the text is rebuilt
      here (entitleText) and signed once; the answer carries `grant`, sent as
      X-Elo-Grant. A wallet with no copy is refused (402). Throws ActError;
      resolves to a Look. */
  async entitle(s, w, sign) {
    const x = slug(s), y = wallet(w);
    if (!x) throw badInput("not a title slug");
    if (!y) throw badInput("wallet must be 0x + 40 hex");
    const p = await this.get(`/api/ticket/${x}/message?wallet=${y}`);
    if (!p.ok) return p;
    const offered = isObject(p.value) ? p.value.message : undefined;
    const now = Math.floor(Date.now() / 1000);
    const ok = [now, now - 86400, now + 86400].map((t) => entitleText(x, y, dayOf(t)));
    if (!ok.includes(offered)) throw mismatch(offered, ok[0]);
    return this.post(`/api/ticket/${x}/entitle`, { wallet: y, signature: await sign(offered) });
  }

  /* ── leaderboards ── */

  /** A title's boards; `kind` "agent" or "human" ranks that side alone. Each
      entry's score is under its board's `sort_key`. */
  async leaderboard(s, { kind } = {}) {
    const x = slug(s);
    if (!x) return refusal(0, "not a title slug");
    if (kind != null && kind !== "agent" && kind !== "human") {
      return refusal(0, "kind is agent or human, or leave it out for everyone");
    }
    return this.get(`/api/leaderboard/${x}` + (kind ? `?kind=${kind}` : ""));
  }

  /* ── a game server's word (ServerKey) ── */

  async _signed(key, method, path, body = new Uint8Array(0), { contentType, extra, raw } = {}) {
    if (!(key instanceof ServerKey)) return refusal(0, "sign with a ServerKey");
    const ctl = new AbortController();
    const timer = setTimeout(() => ctl.abort(), this.timeoutMs);
    try {
      const headers = { Accept: "application/json", ...(await key.headers(method, path, body)), ...(extra || {}) };
      if (contentType) headers["Content-Type"] = contentType;
      const res = await this._fetch(this.origin + "/api" + path, { method, headers, signal: ctl.signal,
        body: method === "PUT" || method === "POST" ? body : undefined });
      if (raw && res.status === 200) {
        const v = res.headers.get("x-elo-version");
        return answer({ data: new Uint8Array(await res.arrayBuffer()),
          etag: (res.headers.get("etag") || "").replace(/"/g, "") || null, version: v ? Number(v) : null });
      }
      return classify(res.status, await res.text());
    } catch (e) {
      return unknown(ctl.signal.aborted ? `${this.origin} did not answer in ${this.timeoutMs} ms`
        : `could not reach ${this.origin}: ${e.message || e}`);
    } finally {
      clearTimeout(timer);
    }
  }

  /** Does the origin accept this key's signature? */
  serverCheck(key) { return this._signed(key, "POST", "/serverkeys/check"); }

  _cloudPath(key, w, slot) {
    const h = holder(w);
    if (!h || (slot !== undefined && !/^[a-z0-9][a-z0-9_.-]{0,47}$/.test(slot))) return null;
    return `/cloud/${key.game}/${h}` + (slot !== undefined ? `/${slot}` : "");
  }

  /** Store one save for a player. `ifMatch` writes only over that etag;
      `createOnly` writes only if no save is there. */
  async cloudPut(key, w, slot, data, { ifMatch, createOnly } = {}) {
    const p = key instanceof ServerKey ? this._cloudPath(key, w, slot) : null;
    if (!p) return refusal(0, "a ServerKey, a wallet, and a slot of 1 to 48 of a-z 0-9 _ . -");
    const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
    const extra = ifMatch ? { "If-Match": ifMatch } : createOnly ? { "If-None-Match": "*" } : {};
    return this._signed(key, "PUT", p, bytes, { contentType: "application/octet-stream", extra });
  }

  /** One save: { data: Uint8Array, etag, version }; refused 404 when none. */
  async cloudGet(key, w, slot) {
    const p = key instanceof ServerKey ? this._cloudPath(key, w, slot) : null;
    if (!p) return refusal(0, "a ServerKey, a wallet, and a slot of 1 to 48 of a-z 0-9 _ . -");
    return this._signed(key, "GET", p, undefined, { raw: true });
  }

  async cloudDelete(key, w, slot, { ifMatch } = {}) {
    const p = key instanceof ServerKey ? this._cloudPath(key, w, slot) : null;
    if (!p) return refusal(0, "a ServerKey, a wallet, and a slot of 1 to 48 of a-z 0-9 _ . -");
    return this._signed(key, "DELETE", p, undefined, { extra: ifMatch ? { "If-Match": ifMatch } : {} });
  }

  /** A player's slots for this key's game. */
  async cloudList(key, w) {
    const p = key instanceof ServerKey ? this._cloudPath(key, w) : null;
    if (!p) return refusal(0, "a ServerKey and a wallet");
    return this._signed(key, "GET", p);
  }

  /** Report stats, unlocks and scores for players:
      [{ wallet, stats: { id: n }, unlock: [ID], scores: { board: n } }]. */
  async statsReport(key, reports) {
    if (!(key instanceof ServerKey)) return refusal(0, "sign with a ServerKey");
    if (!Array.isArray(reports) || !reports.length) return refusal(0, "reports is a non-empty list");
    const body = new TextEncoder().encode(JSON.stringify({ reports }));
    return this._signed(key, "POST", `/gamestats/${key.game}/report`, body, { contentType: "application/json" });
  }

  /* ── reads a game server makes at join ── */

  /** Is this wallet in the playtest (an access code)? The value is a boolean. */
  async playtestAccess(s, w) {
    const g = slug(s), h = holder(w);
    if (!g || !h) return refusal(0, "a title slug and a wallet");
    const got = await this.get(`/api/codes/${g}/access/${h}`);
    return got.ok ? answer(got.value.access === true) : got;
  }

  /** The player's subscribed Workshop mods for a game, with file urls and sha256s. */
  async workshopSubscribed(s, w) {
    const g = slug(s), h = holder(w);
    if (!g || !h) return refusal(0, "a title slug and a wallet");
    return this.get(`/api/workshop/${g}/subscribed/${h}`);
  }

  /** One lobby with its members; a server admits the party from this. */
  async lobby(id) {
    if (typeof id !== "string" || !/^[A-Za-z0-9_-]{16,32}$/.test(id)) return refusal(0, "not a lobby id");
    return this.get(`/api/lobbies/${id}`);
  }

  async lobbyOf(w) {
    const h = holder(w);
    if (!h) return refusal(0, "not a wallet");
    return this.get(`/api/lobbies/of/${h}`);
  }

  /** The first listed server, in the developer's order, that is up and has room for `party`. */
  async quickJoin(s, party = 1) {
    const g = slug(s);
    if (!g || !Number.isInteger(party) || party < 1 || party > 64) return refusal(0, "a title slug, and a party of 1 to 64");
    return this.get(`/api/lobbies/pick/${g}?party=${party}`);
  }
}

/** The name to draw for a friend or buddy row: its name, else a short wallet. */
export function friendLabel(row) {
  const w = String((row && row.wallet) || "");
  return (row && row.name) || (w.length >= 10 ? `${w.slice(0, 6)}…${w.slice(-4)}` : w);
}

/* ── the payout file ───────────────────────────────────────────────────── */
/* When a wipe closes the game writes `<world>.payout-<wipe>.json` and serves
   it at /payout/<wipe>.json beside status.json; the meter pays each row once
   (meter/gamepay.py). Amounts are whole coins in game units, and in
   JavaScript a whole number must be a safe integer (below 2^53). */

const whole = (v) => Number.isSafeInteger(v) && v >= 1;

/** Why gamepay would not owe this row, or null (gamepay._check's words). */
export function checkPayoutRow(row) {
  const r = row && typeof row === "object" && !Array.isArray(row) ? row : {};
  if (!isWallet(r.address)) return "not a wallet";
  if (!whole(r.amount)) return `amount ${JSON.stringify(r.amount) ?? "None"}`;
  const t = String(r.ticker || "").toUpperCase();
  if (!/^[A-Z0-9]{1,12}$/.test(t)) return `ticker ${JSON.stringify(t)}`;
  return null;
}

/** Everything gamepay would refuse in a whole file: the file's own shape
    first, then each row as `row N: why`. Empty when every row is owed. */
export function checkPayout(doc) {
  const d = doc && typeof doc === "object" && !Array.isArray(doc) ? doc : {};
  if (!whole(d.wipe)) return [`the file's wipe is ${JSON.stringify(d.wipe) ?? "None"}, a whole number from 1`];
  if (!Array.isArray(d.payouts)) return ["no payouts array"];
  return d.payouts.map((r, i) => [i + 1, checkPayoutRow(r)]).filter(([, w]) => w).map(([i, w]) => `row ${i}: ${w}`);
}

/** A payout file from rows of { address, amount, ticker, for? }. Throws an
    ActError ("bad_input") naming every problem when gamepay would refuse it. */
export function payoutDoc(wipe, rows) {
  const doc = { wipe, payouts: (rows || []).map((r) => {
    const out = { address: r.address, amount: r.amount, ticker: r.ticker };
    if (Array.isArray(r.for) && r.for.length) out.for = r.for.map(String);
    return out;
  }) };
  const bad = checkPayout(doc);
  if (bad.length) throw badInput(bad.join("; "));
  return doc;
}

/** `<world>.payout-<wipe>.json`, the name the game writes it under. */
export function payoutFileName(world, wipe) {
  return `${world}.payout-${wipe}.json`;
}

/** Node only: write a payout file atomically (a temp file beside it, then a
    rename), after checking it. A file gamepay would refuse is not written. */
export async function writePayout(path, doc) {
  const bad = checkPayout(doc);
  if (bad.length) throw badInput(bad.join("; "));
  const fs = await import("node:fs/promises");
  const tmp = `${path}.tmp-${typeof process !== "undefined" ? process.pid : 0}`;
  try {
    const fh = await fs.open(tmp, "w");
    try {
      await fh.writeFile(JSON.stringify(doc, null, 2) + "\n", "utf8");
      await fh.sync();
    } finally {
      await fh.close();
    }
    await fs.rename(tmp, path);
  } catch (e) {
    await fs.rm(tmp, { force: true });
    throw e;
  }
}

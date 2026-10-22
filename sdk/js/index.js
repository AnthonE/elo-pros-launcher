/* @elopros/sdk — the Elo Pros platform SDK for JavaScript. No dependencies.

   The platform API over fetch (Node 18+, Deno, Bun, browsers): friends and
   buddies, tickets, items, faces, profiles and leaderboards at
   https://elopros.com/api/, the exact texts a wallet signs, and the payout
   file a game server writes when a wipe closes. The same API as the Rust
   (sdk/rust) and Python (sdk/python) packages.

   For a browser game's wallet connect and sign-in, use the web SDK this
   package also ships: import "@elopros/sdk/elo-1.js" (or the script tag
   https://elopros.com/sdk/elo-1.js), which sets window.Elo.

   "Could not look" is never "no". Every call resolves to a Look:
     { kind: "answer", ok: true, value }
     { kind: "refused", ok: false, status, error, body }   status 0: refused here, before asking
     { kind: "could_not_look", ok: false, error }           unreachable, timed out, 5xx, not JSON,
                                                            or the server saying reachable: false */

export const VERSION = "0.1.0";
export const DEFAULT_ORIGIN = "https://elopros.com";
export const DEFAULT_TIMEOUT_MS = 15000;

/* ── the signed texts ──────────────────────────────────────────────────── */

export const FRIENDS_ACTION = "friends";   /* meter/friends.py */
export const FACE_ACTION = "profile";      /* meter/faces.py */

/* Each friends op and the one argument it takes, a wallet (friends.OPS). */
export const OPS = Object.freeze({ ask: "to", accept: "from", decline: "from", cancel: "to",
  remove: "wallet", block: "wallet", unblock: "wallet", invite: "to" });

const WALLET = /^0x[0-9a-fA-F]{40}$/;
const SLUG = /^[a-z0-9][a-z0-9_-]{0,63}$/;

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

/* ── the client ────────────────────────────────────────────────────────── */

const wallet = (w) => (typeof w === "string" && isWallet(w.trim()) ? w.trim().toLowerCase() : null);
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

  /** Whether a wallet holds a copy, with its count and free mints left. */
  async ticketOf(s, w) {
    const x = slug(s), y = wallet(w);
    if (!x) return refusal(0, "not a title slug");
    if (!y) return refusal(0, "wallet must be 0x + 40 hex");
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

  /** One party by sworn name, npub, vow id or wallet. A handle nobody has
      sworn is refused (404); a bare wallet always answers. */
  async who(handle) {
    const h = typeof handle === "string" ? handle.trim() : "";
    return h ? this.get(`/api/who?handle=${encodeURIComponent(h)}`) : refusal(0, "name a handle to look up");
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

/* The web SDK, elo-3.js, run as a page runs it: a script in a window, with
   an EIP-6963 wallet, a Wallet Standard (Solana) wallet and fetch stubbed.
   The sign-in messages it composes are pinned byte for byte to
   sdk/testdata/signin_check.json, the same bytes the meter's parser reads. */
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createPrivateKey, sign as edSign } from "node:crypto";
import vm from "node:vm";

import { fx } from "./_testdata.js";

const SRC = readFileSync(new URL("../elo-3.js", import.meta.url), "utf8");
const SIGNIN = fx("signin_check.json");
const EVM = SIGNIN.cases[0];
const SOL = SIGNIN.cases[1];
const WHO = fx("who_fixture.json");
const LOC = new URL(SIGNIN.web_sdk.location);

/* A page: window is the global, events go through one EventTarget. */
function page({ issuedAt, fetch: f, ethereum } = {}) {
  const bus = new EventTarget();
  const store = new Map();
  const fixed = issuedAt ? Date.parse(issuedAt) : Date.now();
  class PinnedDate extends Date {
    constructor(...a) { if (a.length) super(...a); else super(fixed); }
    static now() { return fixed; }
  }
  const ctx = {
    addEventListener: bus.addEventListener.bind(bus),
    removeEventListener: bus.removeEventListener.bind(bus),
    dispatchEvent: bus.dispatchEvent.bind(bus),
    Event, CustomEvent, TextEncoder, URL, AbortController, setTimeout, clearTimeout, console,
    Date: PinnedDate,
    location: { host: LOC.host, origin: LOC.origin },
    localStorage: { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, String(v)),
      removeItem: (k) => store.delete(k) },
    fetch: f || (async () => { throw new Error("offline"); }),
    ethereum,
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  return ctx;
}
const load = (ctx) => { vm.runInContext(SRC, ctx); return ctx.Elo; };

/* An EIP-1193 wallet holding the fixture's Ethereum key, announcing itself. */
function evmWallet(ctx, { accounts = [EVM.expect.address.toLowerCase()] } = {}) {
  const handlers = {};
  const signed = [];
  const provider = {
    async request({ method, params }) {
      if (method === "eth_requestAccounts" || method === "eth_accounts") return accounts;
      if (method === "eth_chainId") return "0x1237";
      if (method === "personal_sign") {
        signed.push(Buffer.from(params[0].slice(2), "hex").toString("utf8"));
        return EVM.signature.startsWith("0x") ? EVM.signature : "0x" + EVM.signature;
      }
      throw Object.assign(new Error("unsupported " + method), { code: 4200 });
    },
    on(name, fn) { handlers[name] = fn; },
  };
  ctx.addEventListener("eip6963:requestProvider", () => {
    ctx.dispatchEvent(new CustomEvent("eip6963:announceProvider", { detail: Object.freeze({
      info: { name: "Test Wallet", rdns: "example.test", uuid: "6e1a4b3c-0000-4000-8000-000000000001", icon: "" },
      provider }) }));
  });
  return { provider, handlers, signed };
}

/* A Wallet Standard wallet holding the fixture's Solana key (Ed25519 seed 07…07). */
function solanaWallet(ctx) {
  const seed = Buffer.alloc(32, 7);
  const key = createPrivateKey({ key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), seed]),
    format: "der", type: "pkcs8" });
  const account = { address: SOL.expect.address, chains: ["solana:mainnet"] };
  const wallet = {
    name: "Test Solana", icon: "data:image/svg+xml,", chains: ["solana:mainnet"], accounts: [],
    features: {
      "standard:connect": { connect: async () => ({ accounts: [account] }) },
      "solana:signMessage": { signMessage: async ({ message }) => [{ signature: new Uint8Array(edSign(null, message, key)) }] },
    },
  };
  /* as a real wallet does: register now, and again when an app says it is ready */
  ctx.dispatchEvent(new CustomEvent("wallet-standard:register-wallet", { detail: (api) => api.register(wallet) }));
  ctx.addEventListener("wallet-standard:app-ready", (e) => e.detail.register(wallet));
  return wallet;
}

test("loaded where there is no window, it does nothing and throws nothing", () => {
  const ctx = vm.createContext({});
  assert.doesNotThrow(() => vm.runInContext(SRC, ctx));
  assert.equal(ctx.Elo, undefined);
});

test("an Ethereum sign-in is the fixture's bytes, signed once", async () => {
  const ctx = page({ issuedAt: EVM.expect.issued_at });
  const w = evmWallet(ctx);
  const Elo = load(ctx);
  assert.equal(Elo.version, 3);
  const got = await Elo.signIn({ nonce: EVM.nonce });
  assert.equal(got.message, EVM.message);
  assert.equal(got.chain, "evm");
  assert.equal(got.address, EVM.expect.address, "EIP-55, as SIWE verifiers require");
  assert.deepEqual(w.signed, [EVM.message]);
  assert.deepEqual({ ...Elo.identity() }, { address: EVM.expect.address, chain: "evm", wallet: "Test Wallet" });
});

test("a Solana sign-in is the fixture's bytes, and the fixture's signature", async () => {
  const issued = SOL.message.match(/^Issued At: (.+)$/m)[1];
  const ctx = page({ issuedAt: issued });
  solanaWallet(ctx);
  const Elo = load(ctx);
  const got = await Elo.signIn({ nonce: SOL.nonce });
  assert.equal(got.message, SOL.message);
  assert.equal(got.chain, "solana");
  assert.equal(got.address, SOL.expect.address);
  assert.equal(got.signature, SOL.signature, "base58 of the 64-byte Ed25519 signature");
});

test("signIn refuses a nonce outside EIP-4361's alphabet before any wallet is asked", async () => {
  const ctx = page();
  const w = evmWallet(ctx);
  const Elo = load(ctx);
  await assert.rejects(Elo.signIn({ nonce: "550e8400-e29b-41d4" }), /nonce/);
  await assert.rejects(Elo.signIn({}), /nonce/);
  assert.deepEqual(w.signed, []);
});

test("no wallet in the browser is code no_wallet", async () => {
  const Elo = load(page());
  await assert.rejects(Elo.connect(), (e) => e.code === "no_wallet");
});

test("account and chain events: connect, switch, chain, disconnect, off", async () => {
  const ctx = page();
  const w = evmWallet(ctx);
  const Elo = load(ctx);
  const seen = [];
  const chains = [];
  const off = Elo.on("account", (who) => seen.push(who ? who.address : null));
  Elo.on("chain", (id) => chains.push(id));
  await Elo.connect();
  await Elo.connect();                                  /* the same account: no second event */
  const other = "0x7777777777777777777777777777777777777777";
  w.handlers.accountsChanged([other]);
  w.handlers.chainChanged("0x1");
  w.handlers.accountsChanged([]);
  assert.equal(Elo.chain, null, "a wallet that dropped every account is signed out");
  await Elo.connect();
  await Elo.disconnect();
  off();
  await Elo.connect();
  assert.deepEqual(seen, [EVM.expect.address, other, null, EVM.expect.address, null]);
  assert.deepEqual(chains, [1]);
  assert.throws(() => Elo.on("block", () => {}), /account/);
  assert.throws(() => Elo.on("account", "not a function"), /function/);
});

test("a listener that throws does not stop the others", async () => {
  const ctx = page();
  evmWallet(ctx);
  const Elo = load(ctx);
  const seen = [];
  const thrown = [];
  ctx.setTimeout = (fn) => { try { fn(); } catch (e) { thrown.push(e.message); } };
  Elo.on("account", () => { throw new Error("the game's bug"); });
  Elo.on("account", (who) => seen.push(who && who.address));
  await Elo.connect();
  assert.deepEqual(seen, [EVM.expect.address]);
});

test("profile() is the launcher's flattening, with absolute URLs, and null when it could not look", async () => {
  const asked = [];
  const ctx = page({ fetch: async (url, init) => {
    asked.push([url, init.credentials]);
    return { ok: true, json: async () => WHO.who };
  } });
  const Elo = load(ctx);
  const p = await Elo.profile(WHO.address);
  const flat = WHO.flat;
  assert.equal(asked[0][0], `https://elopros.com/api/who?handle=${WHO.address}`);
  assert.equal(asked[0][1], "omit", "no cookie rides along");
  assert.equal(p.address, flat.address);
  assert.equal(p.handle, flat.handle);
  assert.equal(p.checked, flat.checked);
  assert.equal(p.display_name, flat.display_name);
  assert.equal(p.label, flat.handle, "a held handle renders bare");
  assert.equal(p.avatar, "https://elopros.com" + flat.avatar);
  assert.equal(p.links.achievements, "https://elopros.com" + flat.links.achievements);
  assert.equal(p.links.items, "https://elopros.com" + flat.links.items);
  assert.equal(p.links.tip, "https://elopros.com" + flat.links.tip);
  assert.equal(p.matches, flat.matches);

  const typed = structuredClone(WHO.who);
  delete typed.found.nip05;
  const ctx2 = page({ fetch: async () => ({ ok: true, json: async () => typed }) });
  const q = await load(ctx2).profile(WHO.address);
  assert.equal(q.label, "~Moss", "a self-declared name is marked as one");
  assert.equal(q.checked, false);

  const bare = { found: { wallet: WHO.address }, matches: 1 };
  const ctx3 = page({ fetch: async () => ({ ok: true, json: async () => bare }) });
  const r = await load(ctx3).profile(WHO.address);
  assert.equal(r.label, "0x2444…9092");
  assert.equal(r.avatar, null);

  const down = load(page({ fetch: async () => ({ ok: false, status: 503, json: async () => ({}) }) }));
  assert.equal(await down.profile(WHO.address), null, "could not look is null, never anonymous");
  await assert.rejects(down.profile("not-a-wallet"), /wallet/);
});

test("the phone-wallet module is pinned by default, and unpinned only on the game's say", () => {
  const Elo = load(page());
  const d = Elo.config({});
  assert.equal(d.walletconnectScript, "https://elopros.com/sdk/walletconnect-2.25.0.js");
  assert.match(d.walletconnectIntegrity, /^sha384-[A-Za-z0-9+/]{64}$/);
  const own = Elo.config({ walletconnectScript: "https://game.example/wc.js" });
  assert.equal(own.walletconnectIntegrity, "", "our hash does not name another file");
  const pinned = Elo.config({ walletconnectIntegrity: "sha384-" + "A".repeat(64) });
  assert.equal(pinned.walletconnectIntegrity, "sha384-" + "A".repeat(64));
  const junk = Elo.config({ walletconnectIntegrity: "md5-nope" });
  assert.equal(junk.walletconnectIntegrity, "sha384-" + "A".repeat(64), "a malformed pin is ignored");
  assert.match(SRC, /s\.integrity = cfg\.walletconnectIntegrity; s\.crossOrigin = "anonymous";/);
});

test("owns() asks elopros.com, and a free title is everyone's", async () => {
  const asked = [];
  const ctx = page({ fetch: async (url) => {
    asked.push(url);
    if (url.endsWith(`/of/${EVM.expect.address}`)) return { ok: true, json: async () => ({ entitled: false }) };
    return { ok: false, json: async () => ({}) };
  } });
  const Elo = load(ctx);
  assert.equal(await Elo.owns("gates", EVM.expect.address), false);
  assert.equal(asked[0], `https://elopros.com/api/ticket/gates/of/${EVM.expect.address}`);
});

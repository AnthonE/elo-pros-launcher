/* elo-1.js — the Elo Pros web SDK, version 1. No dependencies.

   For a browser game on its own domain. Three jobs:
     Elo.connect()              connect the player's wallet → address
     Elo.signIn({ nonce })      sign the player in to YOUR game's server (SIWE)
     Elo.owns(slug, wallet)     does this wallet own a copy? → true | false | null

   connect() finds every wallet in the browser (EIP-6963) and, when there
   are several, asks which; the choice is remembered on your site until
   Elo.disconnect(). Solana-only wallets show greyed with the reason (sign-in
   is an Ethereum signature). Phone wallets (WalletConnect) appear once you
   pass your own project ID from dashboard.reown.com:
     Elo.config({ walletconnectProjectId: "…" })

     <script src="https://elopros.com/sdk/elo-1.js"></script>
     // then, in an async function:
     const wallet = await Elo.connect();
     const { message, signature } = await Elo.signIn({ nonce });  // nonce from your server
     // POST both to your server and verify them there with viem:
     //   await publicClient.verifySiweMessage({ message, signature })
     // It checks plain keys AND smart-contract wallets (Safe, Coinbase Smart
     // Wallet, Robinhood's EIP-7702 wallets: ERC-1271 and ERC-6492). A bare
     // ecrecover, or a SIWE library without a chain, refuses every smart wallet.
     const owned = await Elo.owns("your-slug", wallet);

   Released files never change. A new version is a new file (elo-2.js), so the
   URL you pin is the code you get.

   owns() is for your UI. The real gate is your server, at join and again on
   an interval:
     POST https://elopros.com/api/ticket/{slug}/check  {"wallets": ["0x…", …]}  (up to 100)
     → {"wallets": {"0x…": {"entitled": true | false | null}}}  (a free title: every_wallet_entitled)
   Kick on false only. null means the read failed, and a failed read never
   kicks a player. owns() keeps the same rule: null is "could not look".

   Web only. A desktop build links sdk/ and asks the launcher instead. */
"use strict";
(() => {
  if (window.Elo && window.Elo.version === 1) return;   /* loaded twice: keep the first */

  const CHAIN_ID = 4663;
  /* Robinhood Chain, for wallet_addEthereumChain */
  const CHAIN = { chainId: "0x1237", chainName: "Robinhood Chain",
    nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
    rpcUrls: ["https://rpc.mainnet.chain.robinhood.com"],
    blockExplorerUrls: ["https://robinhoodchain.blockscout.com"] };
  const BALANCE_OF = "0x70a08231";                      /* balanceOf(address) */
  const SLUG = /^[a-z0-9][a-z0-9_-]{0,40}$/;
  const ADDR = /^0x[0-9a-fA-F]{40}$/;
  const NONCE = /^[A-Za-z0-9]{8,}$/;                    /* EIP-4361: 8+ letters and digits */

  const Elo = { version: 1, ORIGIN: "https://elopros.com", CHAIN_ID, address: null };
  let provider = null;          /* the EIP-1193 provider connect() settled on */
  const tickets = {};           /* slug -> ticket contract, read off the card */
  const watched = new WeakSet();

  /* ── wallets: EIP-6963 discovery, window.ethereum as the fallback ── */
  const announced = [];
  window.addEventListener("eip6963:announceProvider", (e) => {
    const d = e.detail;
    if (!d || !d.provider || typeof d.provider.request !== "function") return;
    if (announced.some((a) => a.provider === d.provider
        || (a.info && d.info && a.info.uuid === d.info.uuid))) return;
    announced.push(d);
  });
  const ask = () => window.dispatchEvent(new Event("eip6963:requestProvider"));
  ask();
  const tick = (ms) => new Promise((ok) => setTimeout(ok, ms));

  /* Solana wallets (Wallet Standard): only the name, icon and chains are
     read, to show a Solana-only wallet greyed with the reason. */
  const solana = [];
  const standard = Object.freeze({
    register(...ws) {
      for (const w of ws) {
        const chains = w && Array.isArray(w.chains) ? w.chains : [];
        const name = String((w && w.name) || "").trim();
        if (name && chains.some((c) => String(c).startsWith("solana:")) && !solana.some((x) => x.name === name)) {
          solana.push({ name, icon: String(w.icon || "") });
        }
      }
      return () => {};
    },
  });
  window.addEventListener("wallet-standard:register-wallet", (e) => {
    try { if (typeof e.detail === "function") e.detail(standard); } catch { /* a wallet's own bug */ }
  });
  try { window.dispatchEvent(new CustomEvent("wallet-standard:app-ready", { detail: standard })); } catch { /* none */ }

  /* The pick, remembered on the game's own origin by the wallet's rdns. */
  const PICK = "elo.wallet";
  const PHONE = "walletconnect";
  const kept = {
    get() { try { return localStorage.getItem(PICK) || ""; } catch { return ""; } },
    set(v) { try { if (v) localStorage.setItem(PICK, v); else localStorage.removeItem(PICK); } catch { /* private mode */ } },
  };
  const cfg = { walletconnectProjectId: "", walletconnectScript: "https://elopros.com/js/vendor/walletconnect-2.25.0.js" };
  const phoneOn = () => /^[0-9a-f]{32}$/i.test(cfg.walletconnectProjectId);
  let phoneKit = null;
  function phone() {
    if (!phoneKit) {
      phoneKit = new Promise((ok, no) => {
        if (window.EloWalletConnect) { ok(window.EloWalletConnect); return; }
        const s = document.createElement("script");
        s.src = cfg.walletconnectScript;
        s.addEventListener("load", () => (window.EloWalletConnect ? ok(window.EloWalletConnect) : no(new Error("the phone-wallet module did not load"))));
        s.addEventListener("error", () => no(new Error("the phone-wallet module did not load")));
        document.head.append(s);
      }).catch((e) => { phoneKit = null; throw e; });
    }
    return phoneKit;
  }
  const same = (n) => String(n).toLowerCase().replace(/wallet/g, "").replace(/[^a-z0-9]/g, "");
  const PHONE_ICON = "data:image/svg+xml," + encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#888" stroke-width="1.6">'
    + '<rect x="6.5" y="2.5" width="11" height="19" rx="2.5"/><path d="M10.5 18.5h3"/></svg>');

  /* Every row the picker can show. */
  function rows() {
    const out = announced.length
      ? announced.map((a) => ({ name: String((a.info && a.info.name) || "Wallet"), icon: String((a.info && a.info.icon) || ""),
        rdns: String((a.info && a.info.rdns) || ""), provider: a.provider }))
      : window.ethereum ? [{ name: "Browser wallet", icon: "", rdns: "", provider: window.ethereum }] : [];
    const held = new Set(out.map((w) => same(w.name)));
    for (const s of solana) if (!held.has(same(s.name))) out.push({ name: s.name, icon: s.icon, rdns: "", provider: null, solanaOnly: true });
    if (phoneOn()) out.push({ name: "Phone wallet", icon: PHONE_ICON, rdns: PHONE, provider: null, phone: true });
    return out;
  }

  /* The picker: a <dialog> styled through the CSSOM, so a page whose CSP
     refuses inline styles still draws it. Null when the player closes it. */
  function picker(all) {
    const css = (el, text) => { el.style.cssText = text; return el; };
    const d = css(document.createElement("dialog"), "margin:auto;padding:16px;width:min(92vw,340px);border:1px solid #444;"
      + "border-radius:8px;background:#16181a;color:#eee;font:15px/1.35 system-ui,sans-serif;box-shadow:0 16px 40px rgba(0,0,0,.5)");
    const h = css(document.createElement("div"), "font-size:12px;letter-spacing:.08em;text-transform:uppercase;color:#999;margin:0 0 8px");
    h.textContent = "Sign in with";
    d.append(h);
    let chosen = null;
    for (const w of all) {
      const b = css(document.createElement("button"), "all:unset;box-sizing:border-box;display:flex;align-items:center;gap:12px;width:100%;"
        + "padding:10px;border-radius:6px;cursor:" + (w.solanaOnly ? "default;opacity:.55" : "pointer"));
      b.type = "button";
      const icon = css(document.createElement(/^data:image\//.test(w.icon) ? "img" : "span"), "width:28px;height:28px;flex:none;border-radius:6px;background:#333");
      if (icon.tagName === "IMG") { icon.src = w.icon; icon.alt = ""; icon.style.background = "none"; }
      const t = document.createElement("span");
      const n = css(document.createElement("b"), "display:block;font-weight:600");
      n.textContent = w.name;
      t.append(n);
      const note = w.solanaOnly ? "Solana only — needs an Ethereum account" : w.phone ? "Scan a code with your wallet app" : "";
      if (note) { const sm = css(document.createElement("small"), "display:block;font-size:12px;color:#999"); sm.textContent = note; t.append(sm); }
      b.append(icon, t);
      if (w.solanaOnly) b.disabled = true;
      else {
        b.addEventListener("mouseenter", () => { b.style.background = "rgba(255,255,255,.07)"; });
        b.addEventListener("mouseleave", () => { b.style.background = "none"; });
        b.addEventListener("click", () => { chosen = w; d.close(); });
      }
      d.append(b);
    }
    if (all.some((w) => w.solanaOnly)) {
      const p = css(document.createElement("p"), "margin:10px 2px 0;font-size:12.5px;color:#999");
      p.textContent = "Sign-in uses an Ethereum account. Phantom, MetaMask and Backpack keep one beside your Solana account; turn it on in the wallet's settings if it's off.";
      d.append(p);
    }
    const x = css(document.createElement("button"), "all:unset;display:block;margin:12px auto 0;cursor:pointer;font-size:13px;color:#999;text-decoration:underline");
    x.type = "button";
    x.textContent = "Cancel";
    x.addEventListener("click", () => d.close());
    d.append(x);
    document.body.append(d);
    return new Promise((done) => {
      d.addEventListener("close", () => { d.remove(); done(chosen); }, { once: true });
      d.showModal();
    });
  }

  /* The remembered wallet if it is here; the one wallet if it is alone;
     otherwise the picker. */
  async function choose() {
    if (!announced.length) { ask(); await tick(150); }
    const all = rows();
    const usable = all.filter((w) => !w.solanaOnly);
    const mine = kept.get() && usable.find((w) => w.rdns === kept.get());
    if (mine) return mine;
    if (usable.length === 1 && usable[0].provider) return usable[0];
    if (!all.length) return null;
    const w = await picker(all);
    if (w) return w;
    throw Object.assign(new Error(usable.length ? "no wallet picked"
      : "sign-in uses an Ethereum account, and the wallets here hold Solana only"), { code: 4001 });
  }

  /* Every wallet in this browser that announced itself, as
     [{ info: { name, icon, rdns, uuid }, provider }]. Show them as a picker
     and pass the chosen one to connect(). */
  Elo.providers = async () => { ask(); await tick(150); return announced.slice(); };

  /* A wallet to READ through without asking anyone: the remembered one,
     then the first that announced, then window.ethereum. */
  async function pick() {
    if (!announced.length) { ask(); await tick(150); }
    const mine = announced.find((a) => a.info && kept.get() && a.info.rdns === kept.get());
    return mine ? mine.provider : announced.length ? announced[0].provider : (window.ethereum || null);
  }

  /* Connect and return the address (EIP-55). Pass an entry from providers()
     or any EIP-1193 provider to choose; otherwise the remembered wallet, the
     only one, or the picker. */
  let isPhone = false;
  Elo.connect = async (choice) => {
    let p = (choice && choice.provider) || choice || null;
    let w = null;
    if (!p) {
      w = await choose();
      if (!w) throw new Error("no wallet found in this browser");
      p = w.phone ? await (await phone()).connect({ projectId: cfg.walletconnectProjectId, url: location.origin }) : w.provider;
    }
    if (!p || typeof p.request !== "function") throw new Error("no wallet found in this browser");
    const accounts = await p.request({ method: "eth_requestAccounts" });
    if (!accounts || !ADDR.test(accounts[0] || "")) throw new Error("the wallet returned no account");
    if (w) kept.set(w.rdns);
    isPhone = !!(w && w.phone);
    provider = p;
    Elo.address = checksum(accounts[0]);
    if (typeof p.on === "function" && !watched.has(p)) {
      watched.add(p);
      p.on("accountsChanged", (a) => {
        if (p === provider) Elo.address = a && ADDR.test(a[0] || "") ? checksum(a[0]) : null;
      });
    }
    return Elo.address;
  };

  /* Put the wallet on Robinhood Chain (4663), adding the chain if the wallet
     has never seen it. Resolves to 4663. */
  Elo.ensureChain = async () => {
    if (!provider) await Elo.connect();
    const p = provider;
    if (parseInt(await p.request({ method: "eth_chainId" }), 16) === CHAIN_ID) return CHAIN_ID;
    try {
      await p.request({ method: "wallet_switchEthereumChain", params: [{ chainId: CHAIN.chainId }] });
    } catch (e) {
      if (e && e.code === 4001) throw e;                 /* the player said no */
      await p.request({ method: "wallet_addEthereumChain", params: [CHAIN] });
      await p.request({ method: "wallet_switchEthereumChain", params: [{ chainId: CHAIN.chainId }] });
    }
    return CHAIN_ID;
  };

  /* Sign the player in to YOUR server with an EIP-4361 (Sign-In with
     Ethereum) message. Get `nonce` from your server first; it checks the
     nonce, domain and Chain ID when it verifies. Nothing here talks to
     elopros.com. Options: nonce (required), domain (default location.host),
     uri (default location.origin), statement (one line), expirationTime. */
  Elo.signIn = async (opts) => {
    const o = opts || {};
    const nonce = String(o.nonce || "");
    if (!NONCE.test(nonce)) throw new Error("signIn needs a nonce from your server (8+ letters and digits)");
    const statement = o.statement == null ? "Sign in to play." : String(o.statement);
    if (/[\r\n]/.test(statement)) throw new Error("statement must be one line");
    if (!provider) await Elo.connect();
    let a = ((await provider.request({ method: "eth_accounts" })) || [])[0];
    if (!ADDR.test(a || "")) a = await Elo.connect(provider);
    const address = Elo.address = checksum(a);
    const message = [
      `${o.domain || location.host} wants you to sign in with your Ethereum account:`,
      address,
      "",
      ...(statement ? [statement, ""] : [""]),
      `URI: ${o.uri || location.origin}`,
      "Version: 1",
      `Chain ID: ${CHAIN_ID}`,
      `Nonce: ${nonce}`,
      `Issued At: ${new Date().toISOString()}`,
      ...(o.expirationTime ? [`Expiration Time: ${new Date(o.expirationTime).toISOString()}`] : []),
    ].join("\n");
    const hex = "0x" + Array.from(new TextEncoder().encode(message),
      (b) => b.toString(16).padStart(2, "0")).join("");
    const signature = await provider.request({ method: "personal_sign", params: [hex, address] });
    return { message, signature, address };
  };

  /* Does `wallet` (default: the connected one) own a copy of `slug`?
     true | false | null. Asks elopros.com first; if that fails, reads the
     ticket's balanceOf through the player's own wallet. A free title (no
     ticket) is true for everyone. null is "could not look": show a retry,
     never a paywall, and never kick a player for it. */
  Elo.owns = async (slug, wallet) => {
    slug = String(slug || "").trim().toLowerCase();
    wallet = String(wallet || Elo.address || "").trim();
    if (!SLUG.test(slug)) throw new Error("owns: bad game slug");
    if (!ADDR.test(wallet)) throw new Error("owns: wallet must be a 0x… address");
    const of = await getJSON(`/api/ticket/${slug}/of/${wallet}`);
    if (of && typeof of.entitled === "boolean") return of.entitled;
    try { return await ownsOnChain(slug, wallet); } catch { return null; }
  };

  async function ownsOnChain(slug, wallet) {
    let to = tickets[slug];
    if (!to) {
      const card = await getJSON(`/api/ticket/${slug}`);
      if (!card) return null;
      if (card.ticketed === false) return true;          /* free: everyone plays */
      if (!ADDR.test(card.contract || "")) return null;
      to = tickets[slug] = card.contract;
    }
    const p = provider || await pick();
    if (!p) return null;
    /* another chain's answer is not this chain's */
    if (parseInt(await p.request({ method: "eth_chainId" }), 16) !== CHAIN_ID) return null;
    const r = await p.request({ method: "eth_call", params: [{ to,
      data: BALANCE_OF + wallet.slice(2).toLowerCase().padStart(64, "0") }, "latest"] });
    /* `0x` is no contract at that address, not a zero balance */
    if (typeof r !== "string" || !/^0x[0-9a-fA-F]+$/.test(r)) return null;
    return BigInt(r) > 0n;
  }

  /* A JSON read from ORIGIN, or null for any failure (network, timeout, non-2xx). */
  async function getJSON(path) {
    const ctl = typeof AbortController === "function" ? new AbortController() : null;
    const t = ctl && setTimeout(() => ctl.abort(), 8000);
    try {
      const r = await fetch(Elo.ORIGIN + path, { credentials: "omit", signal: ctl && ctl.signal });
      return r.ok ? await r.json() : null;
    } catch { return null; } finally { if (t) clearTimeout(t); }
  }

  /* Forget the wallet: the next connect() asks again, and a phone session
     ends rather than staying paired. */
  Elo.disconnect = async () => {
    kept.set("");
    const wasPhone = isPhone;
    provider = null;
    isPhone = false;
    Elo.address = null;
    if (wasPhone && window.EloWalletConnect) await window.EloWalletConnect.disconnect();
  };

  /* Settings: `origin` points the SDK at another origin (testing against a
     local meter); `walletconnectProjectId` turns phone wallets on with YOUR
     project ID (a project ID is locked to the domains you list with Reown,
     so ours cannot serve your site); `walletconnectScript` replaces the
     bundle elopros.com serves, if you host your own copy. */
  Elo.config = (o) => {
    if (o && o.origin) Elo.ORIGIN = String(o.origin).replace(/\/+$/, "");
    if (o && typeof o.walletconnectProjectId === "string") cfg.walletconnectProjectId = o.walletconnectProjectId.trim();
    if (o && typeof o.walletconnectScript === "string" && /^https:\/\//.test(o.walletconnectScript)) cfg.walletconnectScript = o.walletconnectScript;
    return { origin: Elo.ORIGIN, walletconnectProjectId: cfg.walletconnectProjectId, walletconnectScript: cfg.walletconnectScript };
  };

  /* ── EIP-55 checksum, which SIWE verifiers require: keccak-256 in BigInt ── */
  const RC = [1n, 0x8082n, 0x800000000000808an, 0x8000000080008000n, 0x808bn, 0x80000001n,
    0x8000000080008081n, 0x8000000000008009n, 0x8an, 0x88n, 0x80008009n, 0x8000000an,
    0x8000808bn, 0x800000000000008bn, 0x8000000000008089n, 0x8000000000008003n,
    0x8000000000008002n, 0x8000000000000080n, 0x800an, 0x800000008000000an,
    0x8000000080008081n, 0x8000000000008080n, 0x80000001n, 0x8000000080008008n];
  const ROT = [1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44];
  const PILN = [10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1];
  const M64 = (1n << 64n) - 1n;
  const rotl = (v, n) => ((v << BigInt(n)) | (v >> BigInt(64 - n))) & M64;
  function keccakf(s) {
    const b = Array(5);
    for (let r = 0; r < 24; r++) {
      for (let x = 0; x < 5; x++) b[x] = s[x] ^ s[x + 5] ^ s[x + 10] ^ s[x + 15] ^ s[x + 20];
      for (let x = 0; x < 5; x++) {
        const d = b[(x + 4) % 5] ^ rotl(b[(x + 1) % 5], 1);
        for (let y = 0; y < 25; y += 5) s[y + x] ^= d;
      }
      let t = s[1];
      for (let i = 0; i < 24; i++) { const j = PILN[i], v = s[j]; s[j] = rotl(t, ROT[i]); t = v; }
      for (let y = 0; y < 25; y += 5) {
        for (let x = 0; x < 5; x++) b[x] = s[y + x];
        for (let x = 0; x < 5; x++) s[y + x] = b[x] ^ ((~b[(x + 1) % 5]) & b[(x + 2) % 5]);
      }
      s[0] ^= RC[r];
    }
  }
  function keccak256(bytes) {
    const rate = 136, a = Array.from(bytes), end = Math.ceil((a.length + 1) / rate) * rate;
    a.push(1);
    while (a.length < end) a.push(0);
    a[end - 1] |= 0x80;
    const s = Array(25).fill(0n);
    for (let o = 0; o < end; o += rate) {
      for (let i = 0; i < 17; i++) {
        let v = 0n;
        for (let j = 0; j < 8; j++) v |= BigInt(a[o + i * 8 + j]) << BigInt(j * 8);
        s[i] ^= v;
      }
      keccakf(s);
    }
    let out = "";
    for (let i = 0; i < 32; i++) {
      out += Number((s[i >> 3] >> BigInt((i % 8) * 8)) & 255n).toString(16).padStart(2, "0");
    }
    return out;
  }
  function checksum(addr) {
    const a = String(addr).toLowerCase().replace(/^0x/, "");
    const h = keccak256(new TextEncoder().encode(a));
    let out = "0x";
    for (let i = 0; i < 40; i++) out += parseInt(h[i], 16) >= 8 ? a[i].toUpperCase() : a[i];
    return out;
  }

  window.Elo = Elo;
})();

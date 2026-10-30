/* elo-signin-1.js — Sign in with Elo Pros, version 1. No dependencies.

   For a game on its own site. The player signs in to Elo Pros once, with
   Steam, Discord, Google, GitHub or a wallet, says yes to your site, and your
   server gets a pass that names them. Your game never asks for a wallet.

     <script src="https://elopros.com/sdk/elo-signin-1.js"
             integrity="sha384-…" crossorigin="anonymous"></script>

   1. Your server issues a nonce: 8 to 64 letters and digits, used once.
   2. In the page, either open a popup:
        const { pass } = await EloSignIn.popup({ game: "your-slug", nonce });
      Load this script on the page the popup was opened from (or on the page
      you name as `callback`, on your own site): when a sign-in provider's
      page cuts the popup off from your window, the popup comes back to that
      page, which hands the pass to your window and closes itself.
      or send the whole page there and back:
        EloSignIn.start({ game: "your-slug", nonce, returnTo: location.href });
        // …and on the page it comes back to:
        const got = EloSignIn.result();   // { pass } | { error } | null
      A popup rejects with code "cancelled" when the player says no,
      "closed" when the window shuts first (after a grace period, since a
      provider's page can make the popup look closed while the player is
      still signing in), and "replaced" when popup() is called again; call
      it from a click, or the browser blocks it.
   3. Send the pass to your server, which asks us, in any language:
        POST https://elopros.com/api/account/play/check
          {"token": pass, "game": "your-slug",
           "domain": "your-game.example",   // your page's host: no scheme, port or path
           "nonce": the one you issued}
        200 {player, account, wallet, name, named, game, domain, exp}
        401 {code: "pass" | "game" | "domain" | "nonce" | "gone"}: refuse
        503: could not check, ask again
      `player` is the wallet the account links, or its acct_… id with none.
      Nothing is stored there: retire the nonce once it answers. To check
      offline instead, verify the pass with GET /api/account/pubkey
      (sdk/testdata/account_token.json has the shape and a test vector).

   The pass is good for ten minutes, for your game, your host and that one
   nonce, so a pass a player gave another site never works on yours. */
"use strict";
(function () {
  if (typeof window === "undefined" || window.EloSignIn) return;
  var ORIGIN = "https://elopros.com";
  var PAGE = ORIGIN + "/game-signin.html";
  var SLUG = /^[a-z0-9][a-z0-9_-]{0,62}$/;
  var NONCE = /^[0-9A-Za-z]{8,64}$/;
  var CHANNEL = "elo-signin";
  var GRACE_MS = 180000;   /* how long a popup that reads closed may still answer */
  var pending = null;      /* the popup() in flight: { reject } */

  function fail(code, message) {
    var e = new Error(message);
    e.code = code;
    return e;
  }
  function check(o) {
    if (!o || !SLUG.test(String(o.game || ""))) throw fail("game", "game is your store slug, like my-game");
    if (!NONCE.test(String(o.nonce || ""))) throw fail("nonce", "nonce is the one your server issued: 8 to 64 letters and digits");
  }
  function link(o, extra) {
    var q = new URLSearchParams({ game: o.game, nonce: o.nonce });
    for (var k in extra) q.set(k, extra[k]);
    return PAGE + "?" + q.toString();
  }

  /* The popup came back to this page, on the game's own site, with the pass in
     the fragment: hand it to the window that opened the popup (same site, so
     BroadcastChannel and the storage event reach it) and close. */
  (function relay() {
    var h = new URLSearchParams((window.location.hash || "").replace(/^#/, ""));
    if (h.get("elo_popup") !== "1" || !NONCE.test(h.get("elo_nonce") || "")) return;
    var msg = h.get("elo_pass")
      ? { type: "elo:pass", pass: h.get("elo_pass"), nonce: h.get("elo_nonce") }
      : { type: "elo:cancel", nonce: h.get("elo_nonce") };
    try { history.replaceState(null, "", window.location.pathname + window.location.search); }
    catch (_) { window.location.hash = ""; }
    try { var bc = new BroadcastChannel(CHANNEL); bc.postMessage(msg); bc.close(); } catch (_) { /* old browser */ }
    try {
      var key = CHANNEL + ":" + msg.nonce;
      localStorage.setItem(key, JSON.stringify(msg));
      setTimeout(function () { try { localStorage.removeItem(key); } catch (_) { /* gone */ } }, 2000);
    } catch (_) { /* storage blocked: the channel above carried it */ }
    window.close();
  })();

  var EloSignIn = {
    /* the whole page goes to Elo Pros and comes back to returnTo with
       #elo_pass=… (or #elo_error=…) */
    start: function (o) {
      check(o);
      var back = new URL(o.returnTo || window.location.href, window.location.href);
      back.hash = "";
      window.location.assign(link(o, { "return": back.href }));
    },
    /* on the page it came back to: { pass } | { error } | null, and the
       fragment is cleared so a reload or a shared link does not carry it */
    result: function () {
      var h = new URLSearchParams((window.location.hash || "").replace(/^#/, ""));
      if (h.get("elo_popup") === "1") return null;   /* a popup's answer, relayed above */
      var pass = h.get("elo_pass"), error = h.get("elo_error");
      if (!pass && !error) return null;
      try { history.replaceState(null, "", window.location.pathname + window.location.search); }
      catch (_) { window.location.hash = ""; }
      return pass ? { pass: pass } : { error: error };
    },
    /* a popup: resolves { pass, player?, game, exp? }. It answers by message
       while it can reach this window, and through `callback` (default: this
       page) on this site when a provider's page cut it off. */
    popup: function (o) {
      check(o);
      if (pending) pending.reject(fail("replaced", "a newer sign-in started"));
      var back = new URL(o.callback || window.location.href, window.location.href);
      back.hash = "";
      if (back.origin !== window.location.origin) throw fail("callback", "callback is a page on your own site");
      return new Promise(function (resolve, reject) {
        var w = window.open(link(o, { origin: window.location.origin, "return": back.href }), "elo-signin",
          "width=480,height=640");
        if (!w) { reject(fail("blocked", "the browser blocked the window: call popup() from a click")); return; }
        var bc = null, closedAt = 0;
        try { bc = new BroadcastChannel(CHANNEL); bc.onmessage = function (e) { relayed(e.data); }; } catch (_) { bc = null; }
        var timer = setInterval(function () {
          if (!w.closed) { closedAt = 0; return; }
          if (!closedAt) closedAt = Date.now();
          else if (Date.now() - closedAt > GRACE_MS) end(reject, fail("closed", "the sign-in window closed"));
        }, 500);
        pending = { reject: function (e) { end(reject, e); } };
        function end(settle, v) {
          clearInterval(timer);
          window.removeEventListener("message", on);
          window.removeEventListener("storage", stored);
          if (bc) bc.close();
          pending = null;
          settle(v);
        }
        function take(d) {
          if (d.type === "elo:pass" && typeof d.pass === "string") {
            end(resolve, { pass: d.pass, player: d.player, game: o.game, exp: d.exp });
          } else if (d.type === "elo:cancel") {
            end(reject, fail("cancelled", "the player said no"));
          }
        }
        function on(e) {
          if (e.origin !== ORIGIN || e.source !== w || !e.data || e.data.game !== o.game) return;
          take(e.data);
        }
        function relayed(d) { if (d && d.nonce === o.nonce) take(d); }
        function stored(e) {
          if (e.key !== CHANNEL + ":" + o.nonce || !e.newValue) return;
          try { relayed(JSON.parse(e.newValue)); } catch (_) { /* not ours */ }
        }
        window.addEventListener("message", on);
        window.addEventListener("storage", stored);
      });
    },
    /* stop waiting on a popup() in flight; it rejects with code "cancelled" */
    cancel: function () {
      if (pending) pending.reject(fail("cancelled", "the game stopped waiting"));
    },
  };
  window.EloSignIn = EloSignIn;
})();

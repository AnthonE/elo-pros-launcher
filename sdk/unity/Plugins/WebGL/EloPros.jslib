/* The Elo Pros SDK's WebGL half: the C# Elo facade calls into the web SDK,
   elo-3.js, through this file. A WebGL build has no launcher to reach; the
   player's wallet lives in the page around the game.

   elo-3.js is loaded on the first call, pinned by its hash, unless the page
   already loaded it (a WebGL template can add the <script> tag itself).
   ES5 only: Unity's Emscripten optimises library files with an ES5 parser. */

var EloProsLib = {
  $EloProsWeb: {
    SRC: "https://elopros.com/sdk/elo-3.js",
    SRI: "sha384-pKOcdX/DZSGO11b/QS2tFdcm6YFqcVCl0xc6IHDJKFBYRJCvY2JBNjbaVA3jIih/",
    loading: null,

    ensure: function () {
      if (typeof window !== "undefined" && window.Elo && window.Elo.version >= 3) {
        return Promise.resolve(window.Elo);
      }
      if (EloProsWeb.loading) return EloProsWeb.loading;
      EloProsWeb.loading = new Promise(function (resolve, reject) {
        var s = document.createElement("script");
        s.src = EloProsWeb.SRC;
        s.integrity = EloProsWeb.SRI;
        s.crossOrigin = "anonymous";
        s.onload = function () {
          if (window.Elo) resolve(window.Elo);
          else reject({ code: "no_sdk", message: "elo-3.js loaded but set no Elo" });
        };
        s.onerror = function () {
          EloProsWeb.loading = null;
          reject({ code: "no_sdk", message: "could not load " + EloProsWeb.SRC });
        };
        document.head.appendChild(s);
      });
      return EloProsWeb.loading;
    },

    reply: function (cb, id, obj) {
      var s = JSON.stringify(obj);
      var n = lengthBytesUTF8(s) + 1;
      var buf = _malloc(n);
      stringToUTF8(s, buf, n);
      try {
        {{{ makeDynCall('vii', 'cb') }}}(id, buf);
      } finally {
        _free(buf);
      }
    },

    run: function (Elo, op, args) {
      switch (op) {
        case "config":
          if (args.origin) Elo.config({ origin: args.origin });
          return { origin: Elo.ORIGIN };
        case "address":
          return { address: Elo.address, chain: Elo.chain };
        case "connect":
          return Elo.connect().then(function (a) { return { address: a, chain: Elo.chain }; });
        case "signIn":
          var o = { nonce: args.nonce };
          if (args.domain) o.domain = args.domain;
          if (args.uri) o.uri = args.uri;
          if (typeof args.statement === "string") o.statement = args.statement;
          return Elo.signIn(o);
        case "sign":
          return Elo.sign(args.text);
        case "disconnect":
          return Elo.disconnect().then(function () { return {}; });
        default:
          throw { code: "unknown_op", message: "the Elo Pros WebGL bridge has no op " + op };
      }
    }
  },

  EloPros_Call: function (id, opPtr, argsPtr, cb) {
    var op = UTF8ToString(opPtr);
    var args = {};
    try { args = JSON.parse(UTF8ToString(argsPtr) || "{}") || {}; } catch (e) { args = {}; }
    EloProsWeb.ensure()
      .then(function (Elo) { return EloProsWeb.run(Elo, op, args); })
      .then(function (value) {
        EloProsWeb.reply(cb, id, { ok: true, value: value === undefined ? null : value });
      }, function (err) {
        var code = err && err.code !== undefined ? String(err.code) : "";
        var reason = (err && err.message) ? String(err.message) : String(err);
        EloProsWeb.reply(cb, id, { ok: false, code: code, reason: reason });
      });
  }
};

autoAddDeps(EloProsLib, "$EloProsWeb");
mergeInto(LibraryManager.library, EloProsLib);

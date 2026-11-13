// Elo — the one class a Unity game calls. Steamworks has SteamAPI.Init and
// SteamUser; this has Elo.Init and a handful of async calls that work the same
// on every platform:
//
//   desktop and the editor → the Elo launcher on this machine (Overlay.cs)
//   a WebGL player         → the player's wallet in the page (elo-3.js)
//   neither                → anonymous, which is a normal way to play
//
// Every call is async and never throws for an ordinary outcome. Await it from
// the main thread: launcher calls run on a worker thread, so a consent prompt
// never freezes a frame, and the answer comes back on the main thread.
//
// The game never holds a key. There is no call here that takes one.

using System;
using System.Threading.Tasks;

namespace EloPros
{
    public enum EloMode
    {
        /// <summary>No launcher and no wallet. Play on.</summary>
        Anonymous,
        /// <summary>A desktop build talking to the Elo launcher.</summary>
        Launcher,
        /// <summary>A WebGL build talking to the player's wallet through elo-3.js.</summary>
        Browser,
    }

    public static class Elo
    {
        /// <summary>How long <see cref="Init"/> waits for the launcher's hello.</summary>
        public static TimeSpan InitTimeout = TimeSpan.FromSeconds(3);

        public static string Game { get; private set; }
        public static string GameVersion { get; private set; }
        public static EloMode Mode { get; private set; } = EloMode.Anonymous;
        /// <summary>Why <see cref="Mode"/> is Anonymous, in words a player can read.</summary>
        public static string WhyAnonymous { get; private set; } = "Elo.Init has not run";
        /// <summary>The launcher connection on desktop, or null.</summary>
        public static Overlay Launcher { get; private set; }
        /// <summary>The platform API.</summary>
        public static Client Api { get; private set; } = new Client();
        /// <summary>The player's address from the last <see cref="Connect"/>:
        /// a CLAIM, not authentication. Null when there is none.</summary>
        public static string Address { get; private set; }
        /// <summary>Why the last <see cref="Connect"/> found no address.</summary>
        public static string WhyNoAddress { get; private set; }

        /// <summary>Start. `game` is your store slug. Returns true when a
        /// launcher (desktop) or the web SDK (WebGL) is there; false is a
        /// normal state — play anonymously and never ask for a key instead.
        /// `socket` overrides where the launcher is looked for.</summary>
        public static bool Init(string game, string version = null, string origin = null, string socket = null)
        {
            Shutdown();
            Game = game;
            GameVersion = version ?? AppVersion();
            Api = new Client(origin ?? Client.DefaultOrigin);
            Address = null;
#if UNITY_5_3_OR_NEWER
            UnityEngine.Application.quitting -= Shutdown;
            UnityEngine.Application.quitting += Shutdown;
#endif
#if UNITY_WEBGL && !UNITY_EDITOR
            Mode = EloMode.Browser;
            WhyAnonymous = null;
            if (origin != null) _ = WebBridge.Call("config", JsonValue.Object().Set("origin", origin));
            return true;
#else
            var path = string.IsNullOrEmpty(socket) ? Overlay.DefaultSocket() : socket;
            string why = null;
            var connecting = Task.Run(() =>
            {
                var ov = Overlay.Connect(game, GameVersion, path, Overlay.DefaultTimeout, out var w);
                why = w;
                return ov;
            });
            bool finished;
            try { finished = connecting.Wait(InitTimeout); }
            catch (AggregateException e) { finished = true; why = e.InnerException?.Message; }
            if (!finished)
            {
                // Abandon it, and close it if it ever arrives.
                connecting.ContinueWith(t => { if (t.Status == TaskStatus.RanToCompletion) t.Result?.Dispose(); });
                WhyAnonymous = "the launcher did not answer in time";
                return false;
            }
            var overlay = connecting.Status == TaskStatus.RanToCompletion ? connecting.Result : null;
            if (overlay == null)
            {
                WhyAnonymous = why ?? "no launcher";
                return false;
            }
            Launcher = overlay;
            Mode = EloMode.Launcher;
            WhyAnonymous = null;
            // The launcher's own origin, unless the game named another.
            if (origin == null && !string.IsNullOrEmpty(overlay.Host)) Api = new Client(overlay.Host);
            return true;
#endif
        }

        /// <summary>Close the launcher connection. Safe to call twice.</summary>
        public static void Shutdown()
        {
            var l = Launcher;
            Launcher = null;
            l?.Dispose();
            Mode = EloMode.Anonymous;
            WhyAnonymous = "shut down";
        }

        /// <summary>Which backend holds the player's key (`local`, `browser`, …),
        /// or `none`.</summary>
        public static string Signer => Mode == EloMode.Launcher ? Launcher.Signer : Mode == EloMode.Browser ? "browser" : "none";

        // ── who is playing ─────────────────────────────────────────────────

        /// <summary>The player's address: the one their launcher watches, or
        /// on the web, their wallet (which may ask them to connect). A CLAIM:
        /// use <see cref="SignIn"/> when your server needs to know. Null when
        /// there is none — see <see cref="WhyNoAddress"/>.</summary>
        public static async Task<string> Connect()
        {
            Address = null;
            switch (Mode)
            {
                case EloMode.Launcher:
                    var l = Launcher;
                    Address = await Task.Run(() => l.Address());
                    WhyNoAddress = Address == null ? "no address is set in the launcher" : null;
                    break;
#if UNITY_WEBGL && !UNITY_EDITOR
                case EloMode.Browser:
                    var r = await WebBridge.Call("connect", null);
                    Address = r.IsTrue("ok") ? r["value"].Str("address") : null;
                    WhyNoAddress = Address == null ? (r.Str("reason") ?? "no wallet connected") : null;
                    break;
#endif
                default:
                    WhyNoAddress = WhyAnonymous;
                    break;
            }
            return Address;
        }

        /// <summary>Prove to YOUR server who is playing. `domain` is your
        /// server's host (or host:port; on the web, leave it null for the
        /// page's host) and `nonce` is 8+ letters and digits your server issued
        /// and will retire. Send the result's Message and Signature to your
        /// server, which checks them with <see cref="Client.CheckSignIn"/>.</summary>
        public static async Task<SignIn> SignIn(string domain, string nonce)
        {
            switch (Mode)
            {
                case EloMode.Launcher:
                    var l = Launcher;
                    var p = await Task.Run(() => l.Prove(domain, nonce));
                    if (p.Ok) Address = p.Address;
                    return p;
#if UNITY_WEBGL && !UNITY_EDITOR
                case EloMode.Browser:
                    var args = JsonValue.Object().Set("nonce", nonce ?? "");
                    if (!string.IsNullOrEmpty(domain)) args.Set("domain", domain);
                    var r = await WebBridge.Call("signIn", args);
                    if (!r.IsTrue("ok")) return EloPros.SignIn.Fail(WebOutcome(r), r.StrOrEmpty("reason"));
                    var v = r["value"];
                    Address = v.Str("address");
                    return new SignIn
                    {
                        Outcome = SignOutcome.Signed, Message = v.StrOrEmpty("message"), Signature = v.StrOrEmpty("signature"),
                        Address = v.Str("address"), Chain = v.Str("chain") ?? "evm",
                    };
#endif
                default:
                    return EloPros.SignIn.Fail(SignOutcome.NoLauncher, WhyAnonymous);
            }
        }

        /// <summary>Ask the player to sign `text` (an `elo &lt;family&gt;` text —
        /// <see cref="Texts.PlayMessage"/>). `why` is shown above it, as your
        /// claim. Read the outcome: a Handoff is not an error.</summary>
        public static async Task<SignResult> Sign(string text, string why)
        {
            switch (Mode)
            {
                case EloMode.Launcher:
                    var l = Launcher;
                    return await Task.Run(() => l.Sign(text, why));
#if UNITY_WEBGL && !UNITY_EDITOR
                case EloMode.Browser:
                    var r = await WebBridge.Call("sign", JsonValue.Object().Set("text", text ?? ""));
                    if (!r.IsTrue("ok")) return SignResult.Fail(WebOutcome(r), r.StrOrEmpty("reason"));
                    var v = r["value"];
                    return new SignResult
                    {
                        Outcome = SignOutcome.Signed, Signature = v.StrOrEmpty("signature"), Address = v.Str("address"),
                        Family = FamilyOf(text), Backend = "browser",
                    };
#endif
                default:
                    return SignResult.Fail(SignOutcome.NoLauncher, WhyAnonymous);
            }
        }

        // ── what to draw ───────────────────────────────────────────────────

        /// <summary>A name and a face to draw: for `address`, or for the
        /// player when null. Three states — a Profile; a Profile with Found
        /// false (nothing set, or nobody signed in); could-not-look (draw
        /// nothing about identity, never "anonymous").</summary>
        public static async Task<Look<Profile>> Profile(string address = null)
        {
            if (Mode == EloMode.Launcher)
            {
                var l = Launcher;
                return await Task.Run(() => l.Profile(address));
            }
            var who = address ?? Address;
            if (who == null) return Look<Profile>.Answer(EloPros.Profile.Nothing(null));
            return await Api.Profile(who);
        }

        /// <summary>Does this wallet (the player's when null) own a copy of
        /// this title (yours when null)? For your own UI. The gate that admits
        /// a player is your server's, on a wallet it checked: <see
        /// cref="Client.CheckRoster"/>.</summary>
        public static async Task<Look<bool>> Owns(string wallet = null, string slug = null)
        {
            var w = wallet ?? Address;
            if (w == null && Mode == EloMode.Launcher) w = await Connect();
            if (w == null) return Look<bool>.Refused(Refusal.Here("no wallet: nobody is signed in"));
            return await Api.Owns(slug ?? Game, w);
        }

        /// <summary>What the player may play: a copy or a free trial (with the
        /// minutes left), and which of this game's add-ons they hold. For your
        /// own screens; your server's gate is <see cref="Client.CheckRoster"/>.</summary>
        public static async Task<Look<Access>> Access(string wallet = null, string slug = null)
        {
            var w = wallet ?? Address;
            if (w == null && Mode == EloMode.Launcher) w = await Connect();
            if (w == null) return Look<Access>.Refused(Refusal.Here("no wallet: nobody is signed in"));
            return await Api.Access(slug ?? Game, w);
        }

        /// <summary>The launcher's overlay snapshot for a HUD, or null off the
        /// launcher. Display freely; never approve.</summary>
        public static async Task<JsonValue> Snapshot()
        {
            if (Mode != EloMode.Launcher) return null;
            var l = Launcher;
            return await Task.Run(() => l.Snapshot());
        }

        /// <summary>The URL of your shard list (from your manifest), or null off the launcher.</summary>
        public static async Task<string> ServersUrl()
        {
            if (Mode != EloMode.Launcher) return null;
            var l = Launcher;
            return await Task.Run(() => l.ServersUrl());
        }

        /// <summary>Send the player to an https page.</summary>
        public static async Task<bool> Open(string url)
        {
            if (url == null || !url.StartsWith("https://", StringComparison.Ordinal)) return false;
            if (Mode == EloMode.Launcher)
            {
                var l = Launcher;
                if (await Task.Run(() => l.Open(url))) return true;
            }
#if UNITY_5_3_OR_NEWER
            UnityEngine.Application.OpenURL(url);
            return true;
#else
            return false;
#endif
        }

        // ── multiplayer, for the player ────────────────────────────────────

        /// <summary>A signer for the platform's signed changes: each text goes
        /// to the launcher or the wallet, which shows it to the player with
        /// `why` and asks.</summary>
        public static Func<string, Task<SignResult>> SignWith(string why) => text => Sign(text, why);

        static async Task<string> Me()
        {
            if (Address == null && Mode != EloMode.Anonymous) await Connect();
            return Address;
        }

        static Look<T> NoOne<T>() => Look<T>.Refused(Refusal.Here("no wallet: nobody is signed in"));

        /// <summary>The player's friends, what each is playing now (with an
        /// `elo://join` link), their agents and pending asks.</summary>
        public static async Task<Look<JsonValue>> Friends()
        {
            var me = await Me();
            return me == null ? NoOne<JsonValue>() : await Api.Friends(me);
        }

        /// <summary>Ask, accept, decline, cancel, remove, block, unblock or
        /// invite `other`. The player signs it in the launcher or wallet.</summary>
        public static async Task<Look<JsonValue>> Friend(string op, string other)
        {
            var me = await Me();
            return me == null ? NoOne<JsonValue>() : await Api.Friend(me, op, other, SignWith("a friends change"));
        }

        /// <summary>Change the player's name, picture or avatar. The player signs it.</summary>
        public static async Task<Look<JsonValue>> SetFace(FaceChange change)
        {
            var me = await Me();
            return me == null ? NoOne<JsonValue>() : await Api.SetFace(me, change, SignWith("changing your name and face"));
        }

        /// <summary>Quick join: the first of your listed servers that is up and
        /// has room for `party` — `{addr, join, …}`.</summary>
        public static Task<Look<JsonValue>> QuickJoin(int party = 1) => Api.QuickJoin(Game, party);

        /// <summary>The lobby the player is in, if any.</summary>
        public static async Task<Look<JsonValue>> MyLobby()
        {
            var me = await Me();
            return me == null ? NoOne<JsonValue>() : await Api.LobbyOf(me);
        }

        /// <summary>The player's subscribed mods for this game, in order, with
        /// file urls and sha256s to check after download.</summary>
        public static async Task<Look<JsonValue>> Mods()
        {
            var me = await Me();
            return me == null ? NoOne<JsonValue>() : await Api.WorkshopSubscribed(Game, me);
        }

        /// <summary>This game's boards; `kind` null, `agent` or `human`.</summary>
        public static Task<Look<JsonValue>> Leaderboard(string kind = null) => Api.Leaderboard(Game, kind);

        /// <summary>The player's achievements, as this game's servers reported them.</summary>
        public static async Task<Look<JsonValue>> Achievements()
        {
            var me = await Me();
            return me == null ? NoOne<JsonValue>() : await Api.Achievements(me);
        }

        /// <summary>Where the launcher asks this game to leave a crash dump
        /// (`ELO_CRASH_DIR`), or null when it was started another way. After a
        /// crash the launcher shows the player what the file holds and asks
        /// before sending it.</summary>
        public static string CrashDir
        {
            get
            {
                var d = Environment.GetEnvironmentVariable("ELO_CRASH_DIR");
                return string.IsNullOrEmpty(d) ? null : d;
            }
        }

        /// <summary>File a bug report for this game. A crash dump goes only
        /// when the player chose to send it (<see cref="Client.ReportBug"/>).</summary>
        public static Task<Look<JsonValue>> ReportBug(JsonValue report, byte[] shotPng = null, byte[] dump = null, bool playerChoseDump = false) =>
            Api.ReportBug(Game, report, shotPng, dump, playerChoseDump);

        // ── helpers ────────────────────────────────────────────────────────

        static string AppVersion()
        {
#if UNITY_5_3_OR_NEWER
            return UnityEngine.Application.version;
#else
            return "0";
#endif
        }

        /// <summary>The family of an `elo &lt;family&gt;` text.</summary>
        public static string FamilyOf(string text)
        {
            if (text == null || !text.StartsWith("elo ", StringComparison.Ordinal)) return "";
            int nl = text.IndexOf('\n');
            return (nl < 0 ? text.Substring(4) : text.Substring(4, nl - 4)).Trim();
        }

#if UNITY_WEBGL && !UNITY_EDITOR
        static SignOutcome WebOutcome(JsonValue r)
        {
            var code = r.StrOrEmpty("code");
            if (code == "4001") return SignOutcome.RefusedByPlayer;
            if (code == "no_wallet" || code == "no_sdk" || code == "no_bridge") return SignOutcome.NoLauncher;
            return SignOutcome.Refused;
        }
#endif
    }
}

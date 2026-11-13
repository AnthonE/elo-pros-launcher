// The platform API (https://elopros.com/api/): public reads about any wallet;
// a player's own signed changes (friends, face); and a game server's half —
// the sign-in check, the roster sweep, and everything signed by its
// ServerKey (cloud saves, stats, scores). Every answer is a Look: an answer, a
// refusal, or could-not-look, never merged.
//
// Inside Unity it sends through UnityWebRequest (so WebGL works; call it from
// the main thread). Anywhere else — a headless .NET server, a test — through
// HttpClient. Pass your own IHttpTransport to change either.

using System;
using System.Collections.Generic;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;

namespace EloPros
{
    public sealed class HttpRequest
    {
        public string Method = "GET";
        public string Url;
        public byte[] Body;
        public string ContentType;
        public Dictionary<string, string> Headers = new Dictionary<string, string>();
        public int TimeoutSeconds = 15;
    }

    /// <summary>One HTTP exchange. Status 0 means nothing came back, and
    /// <see cref="Body"/> then says why.</summary>
    public struct HttpReply
    {
        public int Status;
        public byte[] Bytes;
        /// <summary>Response headers, names lowercased.</summary>
        public Dictionary<string, string> Headers;
        public string Body => Bytes == null ? "" : Encoding.UTF8.GetString(Bytes);

        public HttpReply(int status, string body, Dictionary<string, string> headers = null)
        {
            Status = status;
            Bytes = Encoding.UTF8.GetBytes(body ?? "");
            Headers = headers ?? new Dictionary<string, string>();
        }

        public HttpReply(int status, byte[] bytes, Dictionary<string, string> headers)
        {
            Status = status;
            Bytes = bytes ?? Array.Empty<byte>();
            Headers = headers ?? new Dictionary<string, string>();
        }

        public string Header(string name) => Headers != null && Headers.TryGetValue(name.ToLowerInvariant(), out var v) ? v : null;
    }

    public interface IHttpTransport
    {
        /// <summary>Send and never throw: a network failure is status 0 with
        /// the reason as the body.</summary>
        Task<HttpReply> Send(HttpRequest request);
    }

    /// <summary>`POST /api/signin/check` found this sign-in fresh, for your
    /// domain and nonce, and signed by the wallet it names.</summary>
    public sealed class CheckedSignIn
    {
        /// <summary>EIP-55 for Ethereum, base58 for Solana: store it as given.</summary>
        public string Address;
        /// <summary>`evm` or `solana`.</summary>
        public string Chain;
        public string ChainId;
        public string Domain;
        public string Uri;
        public string Nonce;
        public string IssuedAt;
        public string Statement;
        public JsonValue Raw;
    }

    /// <summary>`POST /api/account/play/check`: Sign in with Elo Pros.</summary>
    public sealed class CheckedPass
    {
        /// <summary>The linked wallet (lowercase 0x), or the `acct_…` id.</summary>
        public string Player;
        public string Account;
        public string Wallet;
        public string Name;
        /// <summary>False when <see cref="Name"/> is a placeholder.</summary>
        public bool Named;
        public long Exp;
        public JsonValue Raw;
    }

    /// <summary>One roster sweep: each wallet to true, false or null. Kick on
    /// false only — null is a read that failed, and an outage must not boot a
    /// paying player.</summary>
    public sealed class Roster
    {
        public string Game;
        /// <summary>False for a free title: every wallet is entitled.</summary>
        public bool Ticketed = true;
        public Dictionary<string, bool?> Entitled = new Dictionary<string, bool?>();
        /// <summary>Wallets playing on a free trial, and the minutes each has
        /// left. At zero the wallet reads entitled false and is kicked like any
        /// wallet without a copy.</summary>
        public Dictionary<string, int> TrialMinutesLeft = new Dictionary<string, int>();
        /// <summary>Which add-ons each wallet holds: true, false, or null for a read that failed.</summary>
        public Dictionary<string, Dictionary<string, bool?>> Addons = new Dictionary<string, Dictionary<string, bool?>>();
    }

    /// <summary>What a wallet may play: a copy, a free trial, and which add-ons.</summary>
    public sealed class Access
    {
        /// <summary>May play now: a copy, a free game, or trial time left.</summary>
        public bool Entitled;
        /// <summary>Playing on a free trial rather than a copy.</summary>
        public bool Trial;
        /// <summary>Minutes of trial left, measured by the game's own servers; null off a trial.</summary>
        public int? TrialMinutesLeft;
        /// <summary>Each add-on's slug to true, false, or null for a read that failed.</summary>
        public Dictionary<string, bool?> Addons = new Dictionary<string, bool?>();
        public JsonValue Raw;
    }

    /// <summary>One cloud save, as stored.</summary>
    public sealed class CloudSave
    {
        public byte[] Data;
        /// <summary>Pass back as `ifMatch` to write only over this version.</summary>
        public string Etag;
        public long? Version;
    }

    /// <summary>A face change; null leaves a field as it is, "" clears it.</summary>
    public sealed class FaceChange
    {
        /// <summary>Up to 16 plain characters; not starting with 0x or #.</summary>
        public string Name;
        /// <summary>A square 8-bit RGB/RGBA PNG, 64-512 px, at most 256 KiB.</summary>
        public byte[] PicturePng;
        public bool ClearPicture;
        /// <summary>`agent` or `human`.</summary>
        public string Kind;
        public string Harness;
        public string Model;
        /// <summary>A 32-bit word with its top bit set (GAMEDEV.md), or 0 to clear it.</summary>
        public long? Avatar;
    }

    public sealed class Client
    {
        public const string DefaultOrigin = "https://elopros.com";
        /// <summary>Wallets per roster call; the server takes 100.</summary>
        public const int RosterPage = 100;
        const int PicMaxBytes = 256 * 1024;

        public string Origin { get; }
        public int TimeoutSeconds { get; set; } = 15;
        readonly IHttpTransport _http;

        public Client(string origin = DefaultOrigin, IHttpTransport transport = null)
        {
            Origin = (string.IsNullOrEmpty(origin) ? DefaultOrigin : origin).TrimEnd('/');
            _http = transport ?? DefaultTransport();
        }

        static IHttpTransport DefaultTransport()
        {
#if UNITY_5_3_OR_NEWER
            return new UnityHttp();
#else
            return new DotNetHttp();
#endif
        }

        // ── the primitives ─────────────────────────────────────────────────

        Task<HttpReply> Send(string method, string path, byte[] body, string contentType, Dictionary<string, string> headers = null)
        {
            var req = new HttpRequest { Method = method, Url = Origin + path, Body = body, ContentType = contentType, TimeoutSeconds = TimeoutSeconds };
            req.Headers["Accept"] = "application/json";
            if (headers != null) foreach (var kv in headers) req.Headers[kv.Key] = kv.Value;
            return _http.Send(req);
        }

        /// <summary>GET a path under the origin (`/api/…`) and classify the answer.</summary>
        public async Task<Look<JsonValue>> Get(string path)
        {
            var r = await Send("GET", path, null, null);
            return Looks.Classify(r.Status, r.Body);
        }

        /// <summary>POST a JSON body to a path under the origin and classify the answer.</summary>
        public async Task<Look<JsonValue>> Post(string path, JsonValue body)
        {
            var r = await Send("POST", path, Encoding.UTF8.GetBytes(body.ToString()), "application/json");
            return Looks.Classify(r.Status, r.Body);
        }

        /// <summary>One request signed by a game server's key. `path` is the
        /// route without `/api` (`/cloud/…`); the text signs it the same way.</summary>
        public async Task<HttpReply> SendSigned(ServerKey key, string method, string path, byte[] body = null,
            string contentType = null, Dictionary<string, string> extra = null)
        {
            body = body ?? Array.Empty<byte>();
            var headers = key.Headers(method, path, body);
            if (extra != null) foreach (var kv in extra) headers[kv.Key] = kv.Value;
            bool sends = method == "POST" || method == "PUT";
            return await Send(method, "/api" + path, sends ? body : null, sends ? contentType : null, headers);
        }

        async Task<Look<JsonValue>> Signed(ServerKey key, string method, string path, byte[] body = null,
            string contentType = null, Dictionary<string, string> extra = null)
        {
            if (key == null) return Look<JsonValue>.Refused(Refusal.Here("sign with a ServerKey"));
            var r = await SendSigned(key, method, path, body, contentType, extra);
            return Looks.Classify(r.Status, r.Body);
        }

        static Task<Look<T>> Refuse<T>(string why) => Task.FromResult(Look<T>.Refused(Refusal.Here(why)));

        static string Holder(string w)
        {
            w = (w ?? "").Trim();
            if (Texts.IsWallet(w)) return w.ToLowerInvariant();
            if (Texts.IsSolana(w)) return w;
            return null;
        }

        static readonly Regex AccountRe = new Regex("^acct_[0-9a-f]{20}$");

        /// <summary>Who a game keeps things for: a wallet (<see cref="Holder"/>), or a
        /// signed-in account's own <c>acct_…</c> id — the player a pass check answers
        /// for a login that plays without a wallet.</summary>
        static string Player(string w)
        {
            var h = Holder(w);
            if (h != null) return h;
            w = (w ?? "").Trim();
            return AccountRe.IsMatch(w) ? w : null;
        }

        static string Wallet(string w)
        {
            w = (w ?? "").Trim();
            return Texts.IsWallet(w) ? w.ToLowerInvariant() : null;
        }

        static string Slug(string s)
        {
            s = (s ?? "").Trim().ToLowerInvariant();
            return Texts.IsSlug(s) ? s : null;
        }

        const string BadWallet = "wallet must be 0x + 40 hex, or a Solana address";
        const string BadEvmWallet = "wallet must be 0x + 40 hex";
        const string BadSlug = "not a title slug";

        // ── copies ─────────────────────────────────────────────────────────

        /// <summary>A title's ticket card: rails, prices, sales.</summary>
        public Task<Look<JsonValue>> Ticket(string slug)
        {
            var s = Slug(slug);
            return s == null ? Refuse<JsonValue>(BadSlug) : Get($"/api/ticket/{s}");
        }

        /// <summary>Whether a wallet holds a copy, with its count and free mints left.</summary>
        public Task<Look<JsonValue>> TicketOf(string slug, string wallet)
        {
            var s = Slug(slug); var w = Holder(wallet);
            if (s == null) return Refuse<JsonValue>(BadSlug);
            if (w == null) return Refuse<JsonValue>(BadWallet);
            return Get($"/api/ticket/{s}/of/{w}");
        }

        /// <summary>Does this wallet own a copy: true, false, or could-not-look.
        /// Kick a player on a definite false only.</summary>
        public async Task<Look<bool>> Owns(string slug, string wallet)
        {
            var t = await TicketOf(slug, wallet);
            return t.Then(v =>
            {
                var e = v["entitled"].AsBool;
                return e.HasValue ? Look<bool>.Answer(e.Value)
                    : Look<bool>.CouldNotLook(v.Str("why") ?? "the ownership read failed");
            });
        }

        static Dictionary<string, bool?> ReadAddons(JsonValue a)
        {
            var d = new Dictionary<string, bool?>();
            foreach (var kv in a.Fields) d[kv.Key] = kv.Value.AsBool;
            return d;
        }

        /// <summary>What a wallet may play: a copy or a free trial (with the
        /// minutes left, measured by the game's own servers), and which
        /// add-ons it holds. Could-not-look when the read failed — never a
        /// paywall on that.</summary>
        public async Task<Look<Access>> Access(string slug, string wallet)
        {
            var t = await TicketOf(slug, wallet);
            return t.Then(v =>
            {
                var e = v["entitled"].AsBool;
                if (!e.HasValue) return Look<Access>.CouldNotLook(v.Str("why") ?? "the ownership read failed");
                var left = v["trial_minutes_left"].AsLong;
                return Look<Access>.Answer(new Access
                {
                    Entitled = e.Value,
                    Trial = v["trial"].AsBool == true,
                    TrialMinutesLeft = v["trial"].AsBool == true && left.HasValue ? (int?)left.Value : null,
                    Addons = v["addons"].IsObject ? ReadAddons(v["addons"]) : new Dictionary<string, bool?>(),
                    Raw = v,
                });
            });
        }

        /// <summary>One checkout for several copies and add-ons, each at its
        /// own posted price: `send.batched` is one EIP-5792 `wallet_sendCalls`
        /// (all or nothing, where the wallet supports it), `txs` the same calls
        /// to send one after another. `wallet` leaves out what it already holds
        /// (`includeHeld` keeps it). Up to 10 items. Nothing is sent for you.</summary>
        public Task<Look<JsonValue>> BundleBuy(IEnumerable<string> items, string wallet = null, string rail = null, bool includeHeld = false)
        {
            var slugs = new List<string>();
            foreach (var it in items ?? Array.Empty<string>())
            {
                var x = Slug(it);
                if (x == null) return Refuse<JsonValue>(BadSlug);
                if (!slugs.Contains(x)) slugs.Add(x);
            }
            if (slugs.Count == 0) return Refuse<JsonValue>("items is a list of title and add-on slugs");
            if (slugs.Count > 10) return Refuse<JsonValue>("a bundle is at most 10 items");
            var q = new List<string> { "items=" + string.Join(",", slugs) };
            if (!string.IsNullOrEmpty(rail)) q.Add("rail=" + Texts.Enc(rail.Trim()));
            if (wallet != null)
            {
                var w = Wallet(wallet);
                if (w == null) return Refuse<JsonValue>(BadEvmWallet);
                q.Add("wallet=" + w);
            }
            if (includeHeld) q.Add("include_held=1");
            return Get("/api/tickets/bundle/prepare?" + string.Join("&", q));
        }

        /// <summary>The unsigned calls that buy a copy, read off the contract
        /// now: the buying wallet signs and sends them. `rail` narrows to one
        /// (free, eth, usdg, …); the free mint needs `wallet`. Nothing is sent for you.</summary>
        public Task<Look<JsonValue>> TicketBuy(string slug, string wallet = null, string rail = null)
        {
            var s = Slug(slug);
            if (s == null) return Refuse<JsonValue>(BadSlug);
            var q = new List<string>();
            if (!string.IsNullOrEmpty(rail)) q.Add("rail=" + Texts.Enc(rail.Trim()));
            if (wallet != null)
            {
                var w = Wallet(wallet);
                if (w == null) return Refuse<JsonValue>(BadEvmWallet);
                q.Add("wallet=" + w);
            }
            return Get($"/api/ticket/{s}/prepare" + (q.Count > 0 ? "?" + string.Join("&", q) : ""));
        }

        /// <summary>The roster sweep, server to server: which of `wallets`
        /// still hold a copy. Check at join, then on an interval — a player can
        /// sell the copy mid-session. A page that could not be read is null for
        /// every wallet on it; could-not-look when no page could be read.</summary>
        public async Task<Look<Roster>> CheckRoster(string slug, IEnumerable<string> wallets)
        {
            var s = Slug(slug);
            if (s == null) return Look<Roster>.Refused(Refusal.Here(BadSlug));
            var roster = new List<string>();
            foreach (var raw in wallets ?? Array.Empty<string>())
            {
                var h = Holder(raw);
                if (h == null) return Look<Roster>.Refused(Refusal.Here("every wallet is 0x + 40 hex, or a Solana address"));
                if (!roster.Contains(h)) roster.Add(h);
            }
            if (roster.Count == 0) return Look<Roster>.Refused(Refusal.Here("wallets is a list of addresses"));

            var outp = new Roster { Game = s };
            int read = 0;
            string failed = null;
            for (int at = 0; at < roster.Count; at += RosterPage)
            {
                var page = roster.GetRange(at, Math.Min(RosterPage, roster.Count - at));
                var got = await Post($"/api/ticket/{s}/check", JsonValue.Object().Set("wallets", JsonValue.Array(page)));
                if (got.IsRefused) return Look<Roster>.Refused(got.Refusal);
                if (got.IsCouldNotLook || !got.Value.IsObject)
                {
                    failed = got.IsCouldNotLook ? got.Why : "the answer is not in a shape this SDK reads";
                    foreach (var w in page) outp.Entitled[w] = null;
                    continue;
                }
                var v = got.Value;
                read++;
                if (v.Str("game") != null) outp.Game = v.Str("game");
                bool free = v["ticketed"].AsBool == false || v["every_wallet_entitled"].AsBool == true;
                var rows = v["wallets"];
                if (free && (!rows.IsObject || rows.Count == 0))
                {
                    // a free title selling no add-ons: every wallet is in
                    outp.Ticketed = false;
                    outp.Entitled.Clear();
                    foreach (var w in roster) outp.Entitled[w] = true;
                    return Look<Roster>.Answer(outp);
                }
                if (free) outp.Ticketed = false;
                foreach (var w in page)
                {
                    var row = rows[w];
                    outp.Entitled[w] = free ? true : row["entitled"].AsBool;
                    var left = row["trial_minutes_left"].AsLong;
                    if (row["trial"].AsBool == true && left.HasValue) outp.TrialMinutesLeft[w] = (int)left.Value;
                    if (row["addons"].IsObject) outp.Addons[w] = ReadAddons(row["addons"]);
                }
            }
            if (read == 0) return Look<Roster>.CouldNotLook(failed ?? "no page could be read");
            return Look<Roster>.Answer(outp);
        }

        // ── sign-in, the server's half ─────────────────────────────────────

        /// <summary>Is this a fresh sign-in to your server, signed by the
        /// wallet it names? For what the launcher's `prove` or the web SDK's
        /// `signIn` handed your client. `domain` and `nonce` are YOURS, never
        /// the client's. A refusal's <see cref="Refusal.Code"/> names the field
        /// (`domain`, `nonce`, `stale`, `signature`, …); could-not-look is a
        /// smart wallet's chain that could not be asked — never refuse a player
        /// on that. Nothing is stored there: retire the nonce yourself.</summary>
        public async Task<Look<CheckedSignIn>> CheckSignIn(string message, string signature, string domain, string nonce,
            string uri = null, string chainId = null, int? maxAgeSeconds = null)
        {
            if (string.IsNullOrEmpty(message)) return Look<CheckedSignIn>.Refused(Refusal.Here("message is the signed sign-in text"));
            if (string.IsNullOrEmpty(signature)) return Look<CheckedSignIn>.Refused(Refusal.Here("signature is required"));
            if (!Texts.IsDomain(domain)) return Look<CheckedSignIn>.Refused(Refusal.Here("domain is your server's host or host:port, with no scheme or path"));
            if (!Texts.IsNonce(nonce)) return Look<CheckedSignIn>.Refused(Refusal.Here("nonce is the one you issued: 8 to 128 letters and digits"));
            var body = JsonValue.Object().Set("message", message).Set("signature", signature).Set("domain", domain).Set("nonce", nonce);
            if (uri != null) body.Set("uri", uri);
            if (chainId != null) body.Set("chain_id", chainId);
            if (maxAgeSeconds.HasValue)
            {
                if (maxAgeSeconds < 1 || maxAgeSeconds > 86400) return Look<CheckedSignIn>.Refused(Refusal.Here("max_age_s is whole seconds, 1 to 86400"));
                body.Set("max_age_s", maxAgeSeconds.Value);
            }
            var got = await Post("/api/signin/check", body);
            return got.Then(v =>
            {
                var addr = v.Str("address"); var chain = v.Str("chain");
                if (!v.IsTrue("ok") || string.IsNullOrEmpty(addr) || (chain != "evm" && chain != "solana"))
                    return Look<CheckedSignIn>.CouldNotLook("the answer is not a checked sign-in");
                return Look<CheckedSignIn>.Answer(new CheckedSignIn
                {
                    Address = addr, Chain = chain, ChainId = v.Str("chain_id"), Domain = v.Str("domain"),
                    Uri = v.Str("uri"), Nonce = v.Str("nonce"), IssuedAt = v.Str("issued_at"),
                    Statement = v.Str("statement"), Raw = v,
                });
            });
        }

        /// <summary>Sign in with Elo Pros, the server's half: is this pass
        /// ours, for `game`, for your `domain` (a host: no scheme, port or
        /// path) and the `nonce` you issued, and not run out?</summary>
        public async Task<Look<CheckedPass>> CheckPass(string pass, string game, string domain, string nonce)
        {
            if (string.IsNullOrEmpty(pass) || pass.Length > 1024) return Look<CheckedPass>.Refused(Refusal.Here("pass is the token your page was handed"));
            if (string.IsNullOrEmpty(game) || game.Length > 63) return Look<CheckedPass>.Refused(Refusal.Here("game is your store slug"));
            if (string.IsNullOrEmpty(domain) || domain.Contains(":") || domain.Contains("/") || !Texts.IsDomain(domain))
                return Look<CheckedPass>.Refused(Refusal.Here("domain is your page's host, with no scheme, port or path"));
            if (!Texts.IsNonce(nonce) || nonce.Length > 64) return Look<CheckedPass>.Refused(Refusal.Here("nonce is the one you issued: 8 to 64 letters and digits"));
            var body = JsonValue.Object().Set("token", pass).Set("game", game).Set("domain", domain).Set("nonce", nonce);
            var got = await Post("/api/account/play/check", body);
            return got.Then(v =>
            {
                if (!v.IsTrue("ok") || string.IsNullOrEmpty(v.Str("player")) || string.IsNullOrEmpty(v.Str("account")))
                    return Look<CheckedPass>.CouldNotLook("the answer is not a checked pass");
                return Look<CheckedPass>.Answer(new CheckedPass
                {
                    Player = v.Str("player"), Account = v.Str("account"), Wallet = v.Str("wallet"),
                    Name = v.StrOrEmpty("name"), Named = v.IsTrue("named"), Exp = v["exp"].AsLong ?? 0, Raw = v,
                });
            });
        }

        // ── a game server's word (ServerKey) ───────────────────────────────

        /// <summary>A game's public server keys: `{game, keys: [{id, pub}]}`.
        /// What a <see cref="HandoffDoor"/> checks a handoff against.</summary>
        public Task<Look<JsonValue>> ServerKeys(string game)
        {
            var s = Slug(game);
            return s == null ? Refuse<JsonValue>("game is the title's slug") : Get($"/api/store/serverkeys/{s}");
        }

        /// <summary>Does the origin accept this key's signature? `{ok, game, key}`.</summary>
        public Task<Look<JsonValue>> ServerCheck(ServerKey key) => Signed(key, "POST", "/serverkeys/check");

        static readonly Regex SlotRe = new Regex("^[a-z0-9][a-z0-9_.-]{0,47}$");

        static string CloudPath(ServerKey key, string wallet, string slot)
        {
            var w = Player(wallet);
            if (key == null || w == null || (slot != null && !SlotRe.IsMatch(slot))) return null;
            return $"/cloud/{key.Game}/{w}" + (slot != null ? "/" + slot : "");
        }

        const string BadSlot = "a wallet, and a slot of 1 to 48 of a-z 0-9 _ . -";

        /// <summary>Store one save for a player — Steam Cloud, written by your
        /// server. `ifMatch` writes only over that etag; `createOnly` only if
        /// none is there. Answers `{etag, size, version, updated}`.</summary>
        public Task<Look<JsonValue>> CloudPut(ServerKey key, string wallet, string slot, byte[] data, string ifMatch = null, bool createOnly = false)
        {
            var path = CloudPath(key, wallet, slot);
            if (path == null || slot == null) return Refuse<JsonValue>(BadSlot);
            var extra = ifMatch != null ? new Dictionary<string, string> { ["If-Match"] = ifMatch }
                : createOnly ? new Dictionary<string, string> { ["If-None-Match"] = "*" } : null;
            return Signed(key, "PUT", path, data ?? Array.Empty<byte>(), "application/octet-stream", extra);
        }

        /// <summary>One save; refused 404 when there is none.</summary>
        public async Task<Look<CloudSave>> CloudGet(ServerKey key, string wallet, string slot)
        {
            var path = CloudPath(key, wallet, slot);
            if (path == null || slot == null) return Look<CloudSave>.Refused(Refusal.Here(BadSlot));
            var r = await SendSigned(key, "GET", path);
            if (r.Status == 200)
            {
                var etag = (r.Header("etag") ?? "").Trim('"');
                return Look<CloudSave>.Answer(new CloudSave
                {
                    Data = r.Bytes,
                    Etag = etag.Length > 0 ? etag : null,
                    Version = long.TryParse(r.Header("x-elo-version"), out var n) ? n : (long?)null,
                });
            }
            return Looks.Classify(r.Status, r.Body).Map(_ => (CloudSave)null);
        }

        public Task<Look<JsonValue>> CloudDelete(ServerKey key, string wallet, string slot, string ifMatch = null)
        {
            var path = CloudPath(key, wallet, slot);
            if (path == null || slot == null) return Refuse<JsonValue>(BadSlot);
            return Signed(key, "DELETE", path, extra: ifMatch != null ? new Dictionary<string, string> { ["If-Match"] = ifMatch } : null);
        }

        /// <summary>A player's slots for this key's game.</summary>
        public Task<Look<JsonValue>> CloudList(ServerKey key, string wallet)
        {
            var path = CloudPath(key, wallet, null);
            return path == null ? Refuse<JsonValue>("not a wallet") : Signed(key, "GET", path);
        }

        /// <summary>Report stats, achievement unlocks and scores for players:
        /// a list of `{"wallet", "stats": {id: n}, "unlock": [ID], "scores": {board: n}}`.
        /// Only your servers can: a number measured on a player's machine is
        /// one anyone could send.</summary>
        public Task<Look<JsonValue>> StatsReport(ServerKey key, JsonValue reports)
        {
            if (key == null) return Refuse<JsonValue>("sign with a ServerKey");
            if (reports == null || !reports.IsArray || reports.Count == 0) return Refuse<JsonValue>("reports is a non-empty list");
            var body = Encoding.UTF8.GetBytes(JsonValue.Object().Set("reports", reports).ToString());
            return Signed(key, "POST", $"/gamestats/{key.Game}/report", body, "application/json");
        }

        // ── reads a game server makes at join ──────────────────────────────

        /// <summary>Is this wallet in the playtest (an access code)?</summary>
        public async Task<Look<bool>> PlaytestAccess(string slug, string wallet)
        {
            var s = Slug(slug); var w = Holder(wallet);
            if (s == null || w == null) return Look<bool>.Refused(Refusal.Here("a title slug and a wallet"));
            var got = await Get($"/api/codes/{s}/access/{w}");
            return got.Then(v => v["access"].AsBool is bool b ? Look<bool>.Answer(b) : Look<bool>.CouldNotLook("the answer is not in a shape this SDK reads"));
        }

        /// <summary>The player's subscribed mods for this game, in order, with
        /// file urls and sha256s to check after download.</summary>
        public Task<Look<JsonValue>> WorkshopSubscribed(string slug, string wallet)
        {
            var s = Slug(slug); var w = Player(wallet);
            if (s == null || w == null) return Refuse<JsonValue>("a title slug, and a wallet or an account (acct_…)");
            return Get($"/api/workshop/{s}/subscribed/{w}");
        }

        static readonly Regex LobbyId = new Regex("^[A-Za-z0-9_-]{16,32}$");

        /// <summary>One lobby with its members — a server admits the party from this.</summary>
        public Task<Look<JsonValue>> Lobby(string lobbyId) =>
            lobbyId == null || !LobbyId.IsMatch(lobbyId) ? Refuse<JsonValue>("not a lobby id") : Get($"/api/lobbies/{lobbyId}");

        /// <summary>The lobby a wallet is in, if any.</summary>
        public Task<Look<JsonValue>> LobbyOf(string wallet)
        {
            var w = Player(wallet);
            return w == null ? Refuse<JsonValue>("not a wallet or an account (acct_…)") : Get($"/api/lobbies/of/{w}");
        }

        /// <summary>Quick join: the first listed server, in the developer's
        /// order, that is up and has room for `party` — `{addr, join, …}`.</summary>
        public Task<Look<JsonValue>> QuickJoin(string slug, int party = 1)
        {
            var s = Slug(slug);
            if (s == null || party < 1 || party > 64) return Refuse<JsonValue>("a title slug, and a party of 1 to 64");
            return Get($"/api/lobbies/pick/{s}?party={party}");
        }

        // ── public reads about any wallet ──────────────────────────────────

        /// <summary>Everything about one party: `/api/who`, the join behind a
        /// profile. A bare wallet always answers; an unknown name is refused (404).</summary>
        public Task<Look<JsonValue>> Who(string handle)
        {
            var h = (handle ?? "").Trim();
            if (h.Length == 0 || h.Length > 200) return Refuse<JsonValue>("handle is a wallet, a name or an npub");
            return Get("/api/who?handle=" + Texts.Enc(h));
        }

        /// <summary>A name and a face to draw for a wallet, flattened from
        /// `/api/who` the way the launcher and the web SDK do it.</summary>
        public async Task<Look<Profile>> Profile(string wallet)
        {
            var w = (wallet ?? "").Trim();
            if (!Texts.IsWallet(w) && !Texts.IsSolana(w)) return Look<Profile>.Refused(Refusal.Here(BadWallet));
            var got = await Who(w);
            return got.Then(v =>
            {
                var p = EloPros.Profile.FromWho(v, w, Origin);
                return p == null ? Look<Profile>.CouldNotLook("the answer is not in a shape this SDK reads") : Look<Profile>.Answer(p);
            });
        }

        /// <summary>The wallet's own signed name and picture, and its avatar word.</summary>
        public Task<Look<JsonValue>> Face(string wallet)
        {
            var w = Wallet(wallet);
            return w == null ? Refuse<JsonValue>(BadEvmWallet) : Get($"/api/face/{w}");
        }

        /// <summary>Items (NFTs) a wallet holds; `title` keeps one game's.</summary>
        public Task<Look<JsonValue>> Items(string wallet, string title = null)
        {
            var w = Holder(wallet);
            if (w == null) return Refuse<JsonValue>(BadWallet);
            return Get(string.IsNullOrEmpty(title) ? $"/api/items/of/{w}" : $"/api/items/of/{w}?title={Texts.Enc(title.Trim())}");
        }

        /// <summary>Achievements: `earned`, and `uncheckable` — read the second.</summary>
        public Task<Look<JsonValue>> Achievements(string wallet)
        {
            var w = Player(wallet);
            return w == null ? Refuse<JsonValue>(BadWallet) : Get($"/api/achievements/of/{w}");
        }

        /// <summary>A title's boards, ranked by the score its servers reported.
        /// `kind` is null, `agent` or `human`.</summary>
        public Task<Look<JsonValue>> Leaderboard(string slug, string kind = null)
        {
            var s = Slug(slug);
            if (s == null) return Refuse<JsonValue>(BadSlug);
            if (kind == null) return Get($"/api/leaderboard/{s}");
            if (kind != "agent" && kind != "human") return Refuse<JsonValue>("kind is agent or human");
            return Get($"/api/leaderboard/{s}?kind={kind}");
        }

        /// <summary>Who the player plays with, what each is playing now (with
        /// an `elo://join` link), and pending asks.</summary>
        public Task<Look<JsonValue>> Friends(string wallet)
        {
            var w = Wallet(wallet);
            return w == null ? Refuse<JsonValue>(BadEvmWallet) : Get($"/api/friends/{w}");
        }

        /// <summary>The guild tag each of these players shows (1 to 100).</summary>
        public Task<Look<JsonValue>> GuildTags(IEnumerable<string> wallets)
        {
            var list = new List<string>();
            foreach (var raw in wallets ?? Array.Empty<string>())
            {
                var h = Wallet(raw);
                if (h == null) return Refuse<JsonValue>(BadEvmWallet);
                if (!list.Contains(h)) list.Add(h);
            }
            if (list.Count == 0 || list.Count > 100) return Refuse<JsonValue>("name 1 to 100 wallets");
            return Get("/api/guilds/tags?wallets=" + string.Join(",", list));
        }

        /// <summary>The guilds a wallet is in, its asks, its invites and the tag it shows.</summary>
        public Task<Look<JsonValue>> GuildsOf(string wallet)
        {
            var w = Wallet(wallet);
            return w == null ? Refuse<JsonValue>(BadEvmWallet) : Get($"/api/guilds/of/{w}");
        }

        // ── bug reports and crash dumps ────────────────────────────────────

        public const int MinidumpMax = 4 << 20, BacktraceMax = 256 << 10;

        /// <summary>What each crash dump holds, to show the player before they choose.</summary>
        public static readonly IReadOnlyDictionary<string, string> DumpContains = new Dictionary<string, string>
        {
            ["minidump"] = "memory of the game process at the crash: each thread's stack and registers, the "
                         + "loaded modules, and any heap the game chose to include, which can hold whatever "
                         + "the game had in memory (text typed into it, too)",
            ["backtrace"] = "the text the game wrote as it crashed: its call stack, as function names and source lines",
        };

        /// <summary>`minidump` (a file starting MDMP: Windows, breakpad,
        /// crashpad), `backtrace` (UTF-8 text), or null for anything a report
        /// cannot carry — a core file included.</summary>
        public static string DumpFormat(byte[] data)
        {
            if (data == null || data.Length == 0) return null;
            if (data.Length >= 4 && data[0] == (byte)'M' && data[1] == (byte)'D' && data[2] == (byte)'M' && data[3] == (byte)'P') return "minidump";
            if (data.Length >= 4 && data[0] == 0x7f && data[1] == (byte)'E' && data[2] == (byte)'L' && data[3] == (byte)'F') return null;
            if (Array.IndexOf(data, (byte)0) >= 0) return null;
            try { new UTF8Encoding(false, true).GetString(data); } catch (ArgumentException) { return null; }
            return "backtrace";
        }

        /// <summary>File an in-game bug report: `{id, fingerprint, seen, shot,
        /// dump}`. `report` is the report document (GAMEDEV.md §3c). A crash
        /// dump rides a report of kind `crash`, and only when the player chose
        /// to send it just now, after reading <see cref="DumpContains"/>; say
        /// so with `playerChoseDump`, or it is refused here.</summary>
        public Task<Look<JsonValue>> ReportBug(string game, JsonValue report, byte[] shotPng = null, byte[] dump = null, bool playerChoseDump = false)
        {
            var s = Slug(game);
            if (s == null) return Refuse<JsonValue>(BadSlug);
            if (report == null || !report.IsObject) return Refuse<JsonValue>("report is the report document, an object");
            var body = JsonValue.Object().Set("game", s).Set("report", report);
            if (shotPng != null)
            {
                if (shotPng.Length < 8 || shotPng[0] != 0x89 || shotPng[1] != (byte)'P' || shotPng[2] != (byte)'N' || shotPng[3] != (byte)'G')
                    return Refuse<JsonValue>("the screenshot is a PNG");
                body.Set("shot_png_b64", Convert.ToBase64String(shotPng));
            }
            if (dump != null)
            {
                if (!playerChoseDump) return Refuse<JsonValue>("a crash dump goes only when the player chose to send it: ask them, then pass playerChoseDump");
                if (report.Str("kind") != "crash") return Refuse<JsonValue>("a crash dump rides a report of kind crash");
                var fmt = DumpFormat(dump);
                if (fmt == null) return Refuse<JsonValue>("a crash dump is a minidump (MDMP) or a UTF-8 backtrace");
                var cap = fmt == "minidump" ? MinidumpMax : BacktraceMax;
                if (dump.Length > cap) return Refuse<JsonValue>($"a {fmt} is sent only up to {cap} bytes");
                body.Set("dump", JsonValue.Object().Set("format", fmt).Set("b64", Convert.ToBase64String(dump)).Set("consent", true));
            }
            return Post("/api/bugs", body);
        }

        // ── a player's own signed changes ──────────────────────────────────
        //
        // `sign` is asked once, for a text this SDK rebuilt from the change
        // itself — never for whatever the server offered. Elo.Signer passes it
        // to the launcher or the wallet, which asks the player.

        /// <summary>One signed friends change: `op` is ask, accept, decline,
        /// cancel, remove, block, unblock or invite, on the wallet `other`,
        /// signed as `me`.</summary>
        public async Task<Look<JsonValue>> Friend(string me, string op, string other, Func<string, Task<SignResult>> sign)
        {
            if (op == null || !Texts.FriendOps.TryGetValue(op, out var key))
                return Look<JsonValue>.Refused(Refusal.Here("op is one of: " + string.Join(", ", Texts.FriendOps.Keys)));
            var w = Wallet(me); var o = Wallet(other);
            if (w == null) return Look<JsonValue>.Refused(Refusal.Here(BadEvmWallet));
            if (o == null) return Look<JsonValue>.Refused(Refusal.Here($"{key} is a wallet address: 0x and 40 hex characters"));
            if (w == o) return Look<JsonValue>.Refused(Refusal.Here("that's your own wallet"));
            var args = JsonValue.Object().Set(key, o);
            var p = await Post("/api/friends/prepare", JsonValue.Object().Set("wallet", w).Set("op", op).Set("args", args));
            if (!p.IsAnswer) return p;
            var at = p.Value["at"].AsLong;
            var offered = p.Value.Str("sign_this");
            var expected = at.HasValue ? Texts.FriendsSignText(w, op, o, at.Value) : null;
            if (expected == null || offered != expected)
                return Look<JsonValue>.Refused(Refusal.Here("the origin prepared a different change from the one asked for — nothing was signed"));
            var s = await sign(expected);
            if (!s.Ok) return Look<JsonValue>.Refused(Refusal.Here("the signer did not sign: " + s));
            return await Post("/api/friends/act", JsonValue.Object().Set("wallet", w).Set("op", op).Set("args", args)
                .Set("at", at.Value).Set("signature", s.Signature));
        }

        /// <summary>Change a wallet's face, signed.</summary>
        public async Task<Look<JsonValue>> SetFace(string wallet, FaceChange change, Func<string, Task<SignResult>> sign)
        {
            var w = Wallet(wallet);
            if (w == null) return Look<JsonValue>.Refused(Refusal.Here(BadEvmWallet));
            var c = change ?? new FaceChange();
            var name = c.Name?.Trim();
            if (!string.IsNullOrEmpty(name))
            {
                if (name.Length > 16) return Look<JsonValue>.Refused(Refusal.Here("a name is at most 16 characters"));
                foreach (char ch in name) if (ch < 0x20 || ch > 0x7e) return Look<JsonValue>.Refused(Refusal.Here("a name uses plain letters, digits, spaces and punctuation only"));
                if (name.StartsWith("0x", StringComparison.OrdinalIgnoreCase) || name.StartsWith("#", StringComparison.Ordinal))
                    return Look<JsonValue>.Refused(Refusal.Here("a name may not start with 0x or #"));
            }
            if (c.Kind != null && c.Kind != "agent" && c.Kind != "human") return Look<JsonValue>.Refused(Refusal.Here("kind is \"agent\" or \"human\""));
            foreach (var (field, v) in new[] { ("harness", c.Harness), ("model", c.Model) })
            {
                if (string.IsNullOrEmpty(v)) continue;
                if (v.Length > 48 || !Regex.IsMatch(v, "^[A-Za-z0-9 ._:/@+-]+$"))
                    return Look<JsonValue>.Refused(Refusal.Here($"{field} is 1 to 48 characters: letters, digits, spaces and . _ : / @ + -"));
            }
            if (c.PicturePng != null && c.ClearPicture) return Look<JsonValue>.Refused(Refusal.Here("send a picture or clear it, not both"));
            if (c.PicturePng != null && c.PicturePng.Length > PicMaxBytes) return Look<JsonValue>.Refused(Refusal.Here($"the picture is over the {PicMaxBytes}-byte cap"));
            if (c.Avatar.HasValue && !(c.Avatar == 0 || (c.Avatar >= 1L << 31 && c.Avatar < 1L << 32)))
                return Look<JsonValue>.Refused(Refusal.Here("avatar is a 32-bit word with its top bit set (or 0 to clear it)"));
            string sha = c.PicturePng != null ? ServerKey.Sha256Hex(c.PicturePng) : c.ClearPicture ? "" : null;
            if (name == null && sha == null && c.Kind == null && c.Harness == null && c.Model == null && !c.Avatar.HasValue)
                return Look<JsonValue>.Refused(Refusal.Here("nothing to change"));

            var ask = JsonValue.Object().Set("wallet", w);
            if (name != null) ask.Set("name", name);
            if (sha != null) ask.Set("picture_sha256", sha);
            if (c.Kind != null) ask.Set("kind", c.Kind);
            if (c.Harness != null) ask.Set("harness", c.Harness);
            if (c.Model != null) ask.Set("model", c.Model);
            if (c.Avatar.HasValue) ask.Set("avatar", c.Avatar.Value);
            var p = await Post("/api/face/prepare", ask);
            if (!p.IsAnswer) return p;
            var at = p.Value["at"].AsLong;
            var offered = p.Value.Str("sign_this");
            if (!at.HasValue) return Look<JsonValue>.Refused(Refusal.Here("the origin prepared a different change from the one asked for — nothing was signed"));
            var detail = Texts.FaceDetail(at.Value, name, sha, c.Kind, c.Harness, c.Model, c.Avatar);
            long now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
            string text = null;
            foreach (var t in new[] { now, now - 86400, now + 86400 })
            {
                var candidate = Texts.PlayMessage(Texts.FaceAction, w, detail, Texts.DayOf(t));
                if (candidate == offered && p.Value.Str("detail") == detail) { text = candidate; break; }
            }
            if (text == null) return Look<JsonValue>.Refused(Refusal.Here("the origin prepared a different change from the one asked for — nothing was signed"));
            var s = await sign(text);
            if (!s.Ok) return Look<JsonValue>.Refused(Refusal.Here("the signer did not sign: " + s));
            var send = JsonValue.Object().Set("wallet", w);
            if (name != null) send.Set("name", name);
            if (c.Kind != null) send.Set("kind", c.Kind);
            if (c.Harness != null) send.Set("harness", c.Harness);
            if (c.Model != null) send.Set("model", c.Model);
            if (c.Avatar.HasValue) send.Set("avatar", c.Avatar.Value);
            send.Set("clear_picture", c.ClearPicture).Set("at", at.Value).Set("signature", s.Signature);
            if (c.PicturePng != null) send.Set("picture_b64", Convert.ToBase64String(c.PicturePng));
            return await Post("/api/face", send);
        }
    }

#if !UNITY_5_3_OR_NEWER
    /// <summary>HttpClient, for a headless .NET server or a test.</summary>
    public sealed class DotNetHttp : IHttpTransport
    {
        static readonly System.Net.Http.HttpClient Shared = new System.Net.Http.HttpClient();

        public async Task<HttpReply> Send(HttpRequest r)
        {
            try
            {
                using (var cts = new System.Threading.CancellationTokenSource(TimeSpan.FromSeconds(r.TimeoutSeconds)))
                using (var req = new System.Net.Http.HttpRequestMessage(new System.Net.Http.HttpMethod(r.Method), r.Url))
                {
                    if (r.Body != null)
                    {
                        req.Content = new System.Net.Http.ByteArrayContent(r.Body);
                        if (r.ContentType != null) req.Content.Headers.TryAddWithoutValidation("Content-Type", r.ContentType);
                    }
                    foreach (var kv in r.Headers) req.Headers.TryAddWithoutValidation(kv.Key, kv.Value);
                    using (var resp = await Shared.SendAsync(req, cts.Token).ConfigureAwait(false))
                    {
                        var bytes = await resp.Content.ReadAsByteArrayAsync().ConfigureAwait(false);
                        var headers = new Dictionary<string, string>();
                        foreach (var h in resp.Headers) headers[h.Key.ToLowerInvariant()] = string.Join(",", h.Value);
                        foreach (var h in resp.Content.Headers) headers[h.Key.ToLowerInvariant()] = string.Join(",", h.Value);
                        return new HttpReply((int)resp.StatusCode, bytes, headers);
                    }
                }
            }
            catch (Exception e)
            {
                return new HttpReply(0, $"the server could not be reached: {e.Message}");
            }
        }
    }
#endif
}

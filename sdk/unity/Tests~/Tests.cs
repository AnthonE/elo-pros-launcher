// The Unity package's laws, outside Unity.
//
//   dotnet run --project sdk/unity/Tests~
//
// Fixtures are sdk/testdata, the same files the Rust, Python and JavaScript
// SDKs test against. The launcher half drives the real headless broker
// (launcher-rs/target/debug/examples/headless); if it is not built, that part
// prints NOT RUN on its own line and the tail repeats it, because a suite that
// folds a skip into a green count is worse than no suite.

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using EloPros;

static class Tests
{
    static int passed, failed;
    static readonly List<string> skipped = new List<string>();
    static readonly List<string> failures = new List<string>();

    static void Check(string name, bool cond, string detail = "")
    {
        if (cond) { passed++; return; }
        failed++;
        failures.Add(name + (detail.Length > 0 ? " — " + detail : ""));
        Console.WriteLine($"  FAIL  {name}" + (detail.Length > 0 ? $"  — {detail}" : ""));
    }

    static void Section(string title) => Console.WriteLine($"\n── {title} " + new string('─', Math.Max(0, 56 - title.Length)));

    static string Repo()
    {
        var d = new DirectoryInfo(AppContext.BaseDirectory);
        while (d != null && !File.Exists(Path.Combine(d.FullName, "sdk", "PROTOCOL.md"))) d = d.Parent;
        if (d == null) throw new Exception("cannot find the repo root from " + AppContext.BaseDirectory);
        return d.FullName;
    }

    static JsonValue Fixture(string name) => JsonValue.Parse(File.ReadAllText(Path.Combine(Repo(), "sdk", "testdata", name)));

    static async Task<int> Main()
    {
        JsonTests();
        LookTests();
        TextTests();
        ProfileTests();
        await ClientTests();
        ServerKeyTests();
        HandoffTests();
        await HandoffDoorTests();
        await SignedChangeTests();
        await ServerCallTests();
        await BrokerTests();

        Console.WriteLine($"\n{passed} passed, {failed} failed");
        foreach (var s in skipped) Console.WriteLine($"NOT RUN: {s}");
        foreach (var f in failures) Console.WriteLine($"FAILED: {f}");
        return failed == 0 ? 0 : 1;
    }

    // ── json ───────────────────────────────────────────────────────────────

    static void JsonTests()
    {
        Section("json");
        var v = JsonValue.Parse("{\"a\":1,\"b\":null,\"c\":[true,false],\"d\":\"x\\ny\\u00e9\",\"e\":-2.5e3}");
        Check("a number reads as a whole", v["a"].AsLong == 1);
        Check("null is present and null", v.Has("b") && v["b"].IsNull);
        Check("a missing key is null, not a throw", !v.Has("zz") && v["zz"].IsNull && v["zz"]["deeper"].IsNull);
        Check("null is not false", v["b"].AsBool == null && v["c"][1].AsBool == false);
        Check("escapes decode", v.Str("d") == "x\nyé");
        Check("exponents read", v["e"].AsDouble == -2500);
        var text = "elo play\naction: duel\nwallet: 0xabc\nday: 2026-08-04\ndetail: \"quoted\"\ttab";
        var back = JsonValue.Parse(JsonValue.Object().Set("text", text).ToString());
        Check("a signed text with newlines survives the encoder", back.Str("text") == text);
        Check("the encoder writes one line", !JsonValue.Object().Set("text", text).ToString().Contains("\n"));
        Check("large whole numbers write without an exponent", JsonValue.Of(1792555857L).ToString() == "1792555857");
        Check("an emoji survives a round trip", JsonValue.Parse(JsonValue.Of("gg 🎮").ToString()).AsString == "gg 🎮");
        foreach (var bad in new[] { "", "{", "{\"a\":}", "[1,]", "tru", "\"\n\"", "{} x" })
            Check($"bad json {bad.Replace("\n", "\\n")!} is refused", JsonValue.TryParse(bad) == null);
        // every fixture parses
        foreach (var f in Directory.GetFiles(Path.Combine(Repo(), "sdk", "testdata"), "*.json"))
            Check($"fixture {Path.GetFileName(f)} parses", JsonValue.TryParse(File.ReadAllText(f)) != null);
    }

    // ── look ───────────────────────────────────────────────────────────────

    static void LookTests()
    {
        Section("look — three outcomes, never merged");
        Check("200 is an answer", Looks.Classify(200, "{\"ok\":true}").IsAnswer);
        var r = Looks.Classify(404, "{\"error\":\"no ask from that wallet\"}");
        Check("404 is a refusal in the server's words", r.IsRefused && r.Refusal.Error == "no ask from that wallet" && r.Refusal.Status == 404);
        var c = Looks.Classify(503, "{\"entitled\":null,\"why\":\"the chain read failed\"}");
        Check("503 is could-not-look, with its why", c.IsCouldNotLook && c.Why == "the chain read failed");
        Check("an html 502 is could-not-look", Looks.Classify(502, "<html>bad gateway</html>").IsCouldNotLook);
        Check("reachable:false on a 200 is could-not-look", Looks.Classify(200, "{\"reachable\":false,\"why\":\"down\"}").IsCouldNotLook);
        Check("status 0 is could-not-look", Looks.Classify(0, "the server could not be reached").IsCouldNotLook);
        Check("the kind words match the other SDKs",
            Look<int>.Answer(1).KindWord == "answer" && Look<int>.Refused(Refusal.Here("x")).KindWord == "refused" && Look<int>.CouldNotLook("x").KindWord == "could_not_look");
        var sign = Looks.Classify(401, "{\"ok\":false,\"error\":\"the nonce is not yours\",\"code\":\"nonce\"}");
        Check("a sign-in refusal names its field", sign.IsRefused && sign.Refusal.Code == "nonce");
    }

    // ── texts ──────────────────────────────────────────────────────────────

    static void TextTests()
    {
        Section("texts — the signed bytes, rebuilt offline");
        var face = Fixture("face.json")["prepare"];
        var built = Texts.PlayMessage("profile", "0x2222222222222222222222222222222222222222", face.Str("detail"), Texts.DayOf(face["at"].AsLong.Value));
        Check("play_message builds the server's profile text byte for byte", built == face.Str("sign_this"), built);
        var m = Texts.PlayMessage("duel", "0xAbC0000000000000000000000000000000000001", "ETH up 5", "2026-08-04");
        Check("the wallet line is lowercased", m.Contains("\nwallet: 0xabc0000000000000000000000000000000000001\n"));
        Check("play_message starts with the family line", m.StartsWith("elo play\naction: duel\n"));
        Check("FamilyOf reads the family", Elo.FamilyOf(m) == "play" && Elo.FamilyOf("hello") == "");
        Check("UtcDay is a real date", DateTime.TryParseExact(Texts.UtcDay(), "yyyy-MM-dd", null, System.Globalization.DateTimeStyles.None, out _));
        Check("DayOf is UTC", Texts.DayOf(0) == "1970-01-01" && Texts.DayOf(86399) == "1970-01-01" && Texts.DayOf(86400) == "1970-01-02");

        var nonces = Enumerable.Range(0, 200).Select(_ => Texts.NewNonce()).ToList();
        Check("NewNonce is 16 letters and digits", nonces.All(n => n.Length == 16 && Texts.IsNonce(n)));
        Check("NewNonce does not repeat", nonces.Distinct().Count() == nonces.Count);
        Check("NewNonce refuses a length EIP-4361 would", Throws(() => Texts.NewNonce(7)) && Throws(() => Texts.NewNonce(129)));

        Check("a dashed uuid is not a nonce", !Texts.IsNonce("550e8400-e29b-41d4-a716-446655440000"));
        Check("a stripped uuid is", Texts.IsNonce("550e8400e29b41d4a716446655440000"));
        Check("a nonce cannot spell a newline", !Texts.IsNonce("nonce12\nNonce: other"));
        foreach (var good in new[] { "game.example", "shard-3.gates.example:7777", "localhost" })
            Check($"{good} is a domain", Texts.IsDomain(good));
        foreach (var bad in new[] { "https://s.example", "s.example/play", "", "s.example:", "s.example:123456", "a b" })
            Check($"{bad} is not a domain", !Texts.IsDomain(bad));
        Check("a wallet is 0x + 40 hex", Texts.IsWallet("0x24445EFddB08d4938E3E3627042B2Cf4063d9092") && !Texts.IsWallet("0x123"));
        Check("a solana address is base58", Texts.IsSolana("9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin") && !Texts.IsSolana("0OIl0OIl0OIl0OIl0OIl0OIl0OIl0OIl"));
        Check("Enc escapes everything but unreserved", Texts.Enc("a b/é~") == "a%20b%2F%C3%A9~");
    }

    static bool Throws(Action a) { try { a(); return false; } catch { return true; } }

    // ── profile ────────────────────────────────────────────────────────────

    static void ProfileTests()
    {
        Section("profile — the label never dresses a claim as a check");
        var fx = Fixture("who_fixture.json");
        var flat = fx["flat"];
        var p = Profile.FromWho(fx["who"], fx.Str("address"), "https://elopros.com");
        Check("who flattens to the broker's handle", p.Handle == flat.Str("handle"), p.Handle);
        Check("…checked", p.Checked == flat.IsTrue("checked"));
        Check("…display name", p.DisplayName == flat.Str("display_name"));
        Check("…address", p.Address == flat.Str("address"));
        Check("…matches", p.Matches == flat["matches"].AsLong);
        Check("…the avatar, made absolute", p.AvatarUrl == "https://elopros.com" + flat.Str("avatar"), p.AvatarUrl);
        Check("…the links, made absolute", p.AchievementsUrl == "https://elopros.com" + flat["links"].Str("achievements"), p.AchievementsUrl);
        Check("a held handle labels bare", p.Label == "moss@elopros.com");

        var f = Profile.FromFlat(flat, "https://elopros.com");
        Check("the launcher's flat profile reads the same", f.Label == p.Label && f.AvatarUrl == p.AvatarUrl && f.ItemsUrl == p.ItemsUrl);

        var typed = Profile.FromFlat(JsonValue.Parse("{\"address\":\"0x24445EFddB08d4938E3E3627042B2Cf4063d9092\",\"handle\":\"moss@elopros.com\",\"checked\":false,\"display_name\":\"Moss\"}"), null);
        Check("an unchecked handle never labels bare", typed.Label == "~Moss", typed.Label);
        var bare = Profile.FromFlat(JsonValue.Parse("{\"address\":\"0x24445EFddB08d4938E3E3627042B2Cf4063d9092\"}"), null);
        Check("a nameless player is a short address", bare.Label == "0x2444…9092", bare.Label);
        Check("nothing set is Found=false, labelled by address", !Profile.Nothing("0x24445EFddB08d4938E3E3627042B2Cf4063d9092").Found && Profile.Nothing(null).Label == "anonymous");
        Check("https avatars pass through", Profile.Url("https://elopros.com", "https://cdn.example/a.png") == "https://cdn.example/a.png");
    }

    // ── the platform API, against a fake transport ─────────────────────────

    sealed class Fake : IHttpTransport
    {
        public readonly List<(string method, string url, string body)> Sent = new List<(string, string, string)>();
        public Func<string, string, string, HttpReply> Answer = (m, u, b) => new HttpReply(404, "{\"error\":\"no route\"}");
        public HttpRequest Last;
        public Task<HttpReply> Send(HttpRequest r)
        {
            Last = r;
            var body = r.Body == null ? null : System.Text.Encoding.UTF8.GetString(r.Body);
            Sent.Add((r.Method, r.Url, body));
            return Task.FromResult(Answer(r.Method, r.Url, body));
        }
    }

    static async Task ClientTests()
    {
        Section("client — copies, sign-in, roster");
        var fx = Fixture("ticket_of.json");
        var fake = new Fake();
        var api = new Client("https://elopros.com/", fake);
        Check("the origin loses its trailing slash", api.Origin == "https://elopros.com");

        fake.Answer = (m, u, b) => new HttpReply(200, fx["owned"].ToString());
        var owned = await api.Owns("Gates", "0x1111111111111111111111111111111111111111");
        Check("a held copy is Answer(true)", owned.IsAnswer && owned.Value, owned.ToString());
        Check("the url is lowercased and under /api/", fake.Sent.Last().url == "https://elopros.com/api/ticket/gates/of/0x1111111111111111111111111111111111111111", fake.Sent.Last().url);

        fake.Answer = (m, u, b) => new HttpReply(200, fx["free"].ToString());
        Check("a free game is Answer(true)", (await api.Owns("freebie", "0x1111111111111111111111111111111111111111")).Value);

        fake.Answer = (m, u, b) => new HttpReply((int)fx["could_not_look"]["status"].AsLong, fx["could_not_look"]["body"].ToString());
        var down = await api.Owns("gates", "0x1111111111111111111111111111111111111111");
        Check("a failed chain read is could-not-look, never 'does not own'", down.IsCouldNotLook, down.ToString());

        fake.Answer = (m, u, b) => new HttpReply(200, "{\"game\":\"gates\",\"entitled\":null}");
        Check("entitled:null on a 200 is still could-not-look", (await api.Owns("gates", "0x1111111111111111111111111111111111111111")).IsCouldNotLook);

        int before = fake.Sent.Count;
        Check("a bad wallet is refused before anything is sent",
            (await api.Owns("gates", "0x123")).IsRefused && (await api.Owns("Not A Slug", "0x1111111111111111111111111111111111111111")).IsRefused && fake.Sent.Count == before);

        // the roster: pages of 100, a failed page is null for each of its wallets
        var wallets = Enumerable.Range(0, 250).Select(i => "0x" + i.ToString("x40")).ToList();
        int call = 0;
        fake.Answer = (m, u, b) =>
        {
            call++;
            if (call == 2) return new HttpReply(503, "{\"why\":\"rpc down\"}");
            var asked = JsonValue.Parse(b)["wallets"];
            var rows = JsonValue.Object();
            foreach (var w in asked.Items) rows.Set(w.AsString, JsonValue.Object().Set("entitled", w.AsString.EndsWith("0")));
            return new HttpReply(200, JsonValue.Object().Set("game", "gates").Set("wallets", rows).ToString());
        };
        before = fake.Sent.Count;
        var roster = await api.CheckRoster("gates", wallets.Concat(new[] { wallets[0].ToUpperInvariant().Replace("0X", "0x") }));
        Check("the roster goes in pages of 100", fake.Sent.Count - before == 3, (fake.Sent.Count - before).ToString());
        Check("…posting to the title's check", fake.Sent.Last().method == "POST" && fake.Sent.Last().url.EndsWith("/api/ticket/gates/check"));
        Check("…with duplicates folded", roster.Value.Entitled.Count == 250);
        Check("a page that failed is null for each of its wallets", roster.Value.Entitled[wallets[150]] == null && roster.Value.Entitled[wallets[100]] == null);
        Check("a page that answered keeps true and false apart", roster.Value.Entitled[wallets[16]] == true && roster.Value.Entitled[wallets[11]] == false);

        fake.Answer = (m, u, b) => new HttpReply(200, "{\"game\":\"freebie\",\"ticketed\":false}");
        var free = await api.CheckRoster("freebie", wallets.Take(3));
        Check("a free title admits every wallet", free.IsAnswer && !free.Value.Ticketed && free.Value.Entitled.Values.All(e => e == true));

        fake.Answer = (m, u, b) => new HttpReply(0, "unreachable");
        Check("no page read is could-not-look", (await api.CheckRoster("gates", wallets.Take(3))).IsCouldNotLook);

        // trials and add-ons, from the same fixture every SDK reads
        foreach (var (name, entitled, trial, left) in new[] { ("owned_with_addons", true, false, (int?)null), ("trial", true, true, (int?)20), ("trial_used_up", false, true, (int?)0) })
        {
            fake.Answer = (m, u, b) => new HttpReply(200, fx[name].ToString());
            var acc = await api.Access("gates", fx[name].Str("wallet"));
            Check($"access reads {name}", acc.IsAnswer && acc.Value.Entitled == entitled && acc.Value.Trial == trial && acc.Value.TrialMinutesLeft == left, acc.ToString());
        }
        fake.Answer = (m, u, b) => new HttpReply(200, fx["trial"].ToString());
        var ta = await api.Access("gates", fx["trial"].Str("wallet"));
        Check("a trial wallet's add-ons keep true and false apart", ta.Value.Addons["gates-maps"] == false && ta.Value.Addons["gates-soundtrack"] == true);
        fake.Answer = (m, u, b) => new HttpReply((int)fx["could_not_look"]["status"].AsLong, fx["could_not_look"]["body"].ToString());
        Check("a failed read is could-not-look, never 'no access'", (await api.Access("gates", "0x1111111111111111111111111111111111111111")).IsCouldNotLook);
        var tw = new[] { "0x" + new string('a', 40), "0x" + new string('b', 40) };
        fake.Answer = (m, u, b) => new HttpReply(200, JsonValue.Object().Set("game", "gates").Set("wallets", JsonValue.Object()
            .Set(tw[0], JsonValue.Object().Set("entitled", true).Set("trial", true).Set("trial_minutes_left", 7).Set("addons", JsonValue.Object().Set("gates-maps", false)))
            .Set(tw[1], JsonValue.Object().Set("entitled", true).Set("addons", JsonValue.Object().Set("gates-maps", true)))).ToString());
        var tr = await api.CheckRoster("gates", tw);
        Check("the roster carries trial minutes and add-ons", tr.Value.TrialMinutesLeft[tw[0]] == 7 && !tr.Value.TrialMinutesLeft.ContainsKey(tw[1]) && tr.Value.Addons[tw[1]]["gates-maps"] == true);
        fake.Answer = (m, u, b) => new HttpReply(200, JsonValue.Object().Set("game", "freebie").Set("ticketed", false).Set("every_wallet_entitled", true)
            .Set("wallets", JsonValue.Object().Set(tw[0], JsonValue.Object().Set("entitled", true).Set("addons", JsonValue.Object().Set("freebie-maps", true)))).ToString());
        var fr2 = await api.CheckRoster("freebie", tw.Take(1));
        Check("a free title selling add-ons admits everyone and still says who holds them", !fr2.Value.Ticketed && fr2.Value.Entitled[tw[0]] == true && fr2.Value.Addons[tw[0]]["freebie-maps"] == true);
        fake.Answer = (m, u, b) => new HttpReply(200, "{\"items\":[],\"txs\":[]}");
        await api.BundleBuy(new[] { "Gates", "gates-maps", "gates" }, "0x1111111111111111111111111111111111111111", "eth");
        Check("a bundle asks once per item, lowercased, with the rail and wallet",
            fake.Sent.Last().url == "https://elopros.com/api/tickets/bundle/prepare?items=gates,gates-maps&rail=eth&wallet=0x1111111111111111111111111111111111111111", fake.Sent.Last().url);
        Check("a bundle over 10 items is refused here", (await api.BundleBuy(Enumerable.Range(0, 11).Select(i => "g" + i))).IsRefused);

        // sign-in
        var cases = Fixture("signin_check.json")["cases"];
        var first = cases[0];
        fake.Answer = (m, u, b) => new HttpReply(200, first["expect"].ToString());
        var si = await api.CheckSignIn(first.Str("message"), first.Str("signature"), first.Str("domain"), first.Str("nonce"));
        var sent = JsonValue.Parse(fake.Sent.Last().body);
        Check("check_sign_in posts the four fields", fake.Sent.Last().url.EndsWith("/api/signin/check")
            && sent.Str("message") == first.Str("message") && sent.Str("signature") == first.Str("signature")
            && sent.Str("domain") == first.Str("domain") && sent.Str("nonce") == first.Str("nonce"));
        Check("a checked sign-in reads its address and chain", si.IsAnswer && si.Value.Address == first["expect"].Str("address") && si.Value.Chain == "evm", si.ToString());
        foreach (var c in cases.Items.Where(c => c["expect"].AsBool == null && c["expect"]["ok"].AsBool == false).Take(3))
        {
            fake.Answer = (m, u, b) => new HttpReply(401, c["expect"].ToString());
            var no = await api.CheckSignIn(c.Str("message"), c.Str("signature") ?? "0x", c.Str("domain") ?? "game.example", c.Str("nonce") ?? "Q8n2Vx41abc");
            Check($"a refused case keeps its code: {c.Str("name")}", no.IsRefused && no.Refusal.Code == c["expect"].Str("code"), no.ToString());
        }
        fake.Answer = (m, u, b) => new HttpReply(503, "{\"ok\":false,\"reachable\":false,\"why\":\"the chain could not be asked\"}");
        Check("a smart wallet's unreachable chain is could-not-look", (await api.CheckSignIn("m", "s", "game.example", "Q8n2Vx41abc")).IsCouldNotLook);
        before = fake.Sent.Count;
        Check("a dashed nonce is refused here", (await api.CheckSignIn("m", "s", "game.example", "550e8400-e29b-41d4-a716-446655440000")).IsRefused);
        Check("a url is not a domain", (await api.CheckSignIn("m", "s", "https://game.example", "Q8n2Vx41abc")).IsRefused);
        Check("max_age_s outside 1..86400 is refused", (await api.CheckSignIn("m", "s", "game.example", "Q8n2Vx41abc", maxAgeSeconds: 0)).IsRefused);
        Check("…and none of those were sent", fake.Sent.Count == before);

        // profile over http
        var who = Fixture("who_fixture.json");
        fake.Answer = (m, u, b) => new HttpReply(200, who["who"].ToString());
        var prof = await api.Profile(who.Str("address"));
        Check("Client.Profile reads /api/who", fake.Sent.Last().url == "https://elopros.com/api/who?handle=" + who.Str("address") && prof.Value.Label == "moss@elopros.com");
        fake.Answer = (m, u, b) => new HttpReply(0, "down");
        Check("an unreachable who is could-not-look, never 'anonymous'", (await api.Profile(who.Str("address"))).IsCouldNotLook);
        Check("leaderboard kind is agent or human", (await api.Leaderboard("gates", "whale")).IsRefused);
    }

    // ── a game server's key ────────────────────────────────────────────────

    static byte[] Utf8(string s) => System.Text.Encoding.UTF8.GetBytes(s);

    static void ServerKeyTests()
    {
        Section("server key — Ed25519, pinned to the byte");
        var fx = Fixture("server_texts.json");
        var seed = Convert.FromHexString(fx.Str("seed_hex"));
        var key = new ServerKey("my-game", "eu-1", seed);
        Check("the public key matches every SDK's", key.PublicB64 == fx.Str("public_b64"), key.PublicB64);
        foreach (var c in fx["cases"].Items)
        {
            var text = ServerKey.ServerText(c.Str("method"), c.Str("path"), c.Str("who"), c.Str("at"), Utf8(c.StrOrEmpty("body_utf8")), c.Str("nonce"));
            Check($"the {c.Str("method")} text is the meter's", text == c.Str("text"), text);
            Check($"…and signs to the pinned signature", key.SignText(text) == c.Str("signature_b64"));
            Check($"…which verifies", Ed25519.Verify(Convert.FromBase64String(key.PublicB64), Utf8(text), Convert.FromBase64String(c.Str("signature_b64"))));
        }
        // PEM armour around a base64 body, built here so no key header is a literal.
        var label = string.Join(" ", "PRIVATE", "KEY");
        Func<string, string> armour = body => $"-----BEGIN {label}-----\n{body}\n-----END {label}-----\n";
        var pem = armour(fx.Str("private_pkcs8_b64"));
        Check("a PKCS#8 PEM reads to the same key", ServerKey.FromPem("my-game", "eu-1", pem).PublicB64 == key.PublicB64);
        var tmp = Path.GetTempFileName();
        File.WriteAllText(tmp, fx.Str("seed_hex") + "\n");
        Check("a hex seed file reads too", ServerKey.FromFile("my-game", "eu-1", tmp).PublicB64 == key.PublicB64);
        File.WriteAllText(tmp, pem);
        Check("…and a PEM file", ServerKey.FromFile("my-game", "eu-1", tmp).SeedHex() == fx.Str("seed_hex"));
        File.Delete(tmp);
        Check("a bad PEM is refused", Throws(() => ServerKey.FromPem("my-game", "eu-1", armour("AAAA"))));
        Check("a bad slug or id is refused", Throws(() => new ServerKey("My Game", "eu-1", seed)) && Throws(() => new ServerKey("my-game", "EU_1", seed)));
        var g = ServerKey.Generate("my-game", "eu-2");
        Check("a generated key signs what it verifies", Ed25519.Verify(Convert.FromBase64String(g.PublicB64), Utf8("x"), Convert.FromBase64String(g.SignText("x"))));
        var h = key.Headers("PUT", "/cloud/my-game/0x1111111111111111111111111111111111111111/save", Utf8("{\"level\":3}"), 1761400001, "AbCdEfGh_12345678");
        Check("headers carry who, time, nonce and the pinned signature",
            h["X-Elo-Server"] == "my-game/eu-1" && h["X-Elo-Time"] == "1761400001" && h["X-Elo-Nonce"] == "AbCdEfGh_12345678"
            && h["X-Elo-Signature"] == fx["cases"][1].Str("signature_b64"));
        var nonces = Enumerable.Range(0, 100).Select(_ => ServerKey.NewNonce()).ToList();
        Check("request nonces are 22 of the server's alphabet and never repeat",
            nonces.All(n => n.Length == 22 && n.All(ch => char.IsLetterOrDigit(ch) || ch == '-' || ch == '_')) && nonces.Distinct().Count() == 100);
        // RFC 8032 §7.1, test 1 and test 2
        var s1 = Convert.FromHexString("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        Check("RFC 8032 test 1", Convert.ToHexString(Ed25519.Sign(s1, new byte[0])).ToLowerInvariant() ==
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");
        var s2 = Convert.FromHexString("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb");
        Check("RFC 8032 test 2", Convert.ToHexString(Ed25519.Sign(s2, new byte[] { 0x72 })).ToLowerInvariant() ==
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00");
        var sig = Ed25519.Sign(s2, new byte[] { 0x72 });
        var pk2 = Ed25519.PublicKey(s2);
        Check("a signature verifies only over its own message",
            Ed25519.Verify(pk2, new byte[] { 0x72 }, sig) && !Ed25519.Verify(pk2, new byte[] { 0x73 }, sig) && !Ed25519.Verify(Ed25519.PublicKey(s1), new byte[] { 0x72 }, sig));
        var high = (byte[])sig.Clone();
        high[63] |= 0xf0;
        Check("an S at or above the group order is refused", !Ed25519.Verify(pk2, new byte[] { 0x72 }, high));
    }

    // ── handoffs ───────────────────────────────────────────────────────────

    static Dictionary<string, byte[]> HandoffKeys(JsonValue fx) =>
        fx["keys"].Fields.ToDictionary(kv => kv.Key, kv => Convert.FromBase64String(kv.Value.AsString));

    static void HandoffTests()
    {
        Section("handoffs — every case and refusal in handoff.json");
        var fx = Fixture("handoff.json");
        var key = new ServerKey(fx.Str("game"), fx.Str("key_id"), Convert.FromHexString(fx.Str("seed_hex")));
        long now = fx["now"].AsLong.Value, exp = fx["exp"].AsLong.Value;
        var names = new[] { fx.Str("to") };
        foreach (var c in fx["cases"].Items)
        {
            var carry = c.Str("carry_utf8") == null ? null : Utf8(c.Str("carry_utf8"));
            var text = Handoff.TextOf(fx.Str("game"), fx.Str("key_id"), c.Str("player"), fx.Str("from"), fx.Str("to"), c.Str("id"), exp, carry);
            Check($"{c.Str("name")}: the text is pinned", text == c.Str("text"));
            var token = key.Handoff(c.Str("player"), fx.Str("from"), fx.Str("to"), carry, (int)(exp - now), now, c.Str("id"));
            Check($"{c.Str("name")}: the token is pinned", token == c.Str("token"));
            var got = Handoff.Verify(c.Str("token"), fx.Str("game"), names, HandoffKeys(fx), now, carry);
            Check($"{c.Str("name")}: it verifies", got.Ok && got.Value.Player == c.Str("player"), got.ToString());
        }
        foreach (var r in fx["refused"].Items)
        {
            var carry = r.Str("carry_utf8") == null ? null : Utf8(r.Str("carry_utf8"));
            if (r.Str("code") == "used")
            {
                // a second admission is the door's to refuse, not the token's
                var door = new HandoffDoor(fx.Str("game"), names, keys: fx["keys"].Fields.ToDictionary(kv => kv.Key, kv => kv.Value.AsString), clock: () => now);
                var first = door.Admit(r.Str("token"), carry).Result;
                var again = door.Admit(r.Str("token"), carry).Result;
                Check("refused as used, the second time", first.Ok && again.Code == "used", again.ToString());
                continue;
            }
            var got = Handoff.Verify(r.Str("token"), fx.Str("game"), names, HandoffKeys(fx), now, carry);
            Check($"refused as {r.Str("code")}", !got.Ok && got.Code == r.Str("code"), got.ToString());
        }
        Check("issue refuses a ttl over the cap", Throws(() => key.Handoff("0x24445efddb08d4938e3e3627042b2cf4063d9092", fx.Str("from"), fx.Str("to"), ttl: 121)));
        Check("issue refuses a checksummed wallet", Throws(() => key.Handoff("0x24445EFddB08d4938E3E3627042B2Cf4063d9092", fx.Str("from"), fx.Str("to"))));
    }

    static async Task HandoffDoorTests()
    {
        Section("the handoff door — once each, keys read and refreshed");
        var fx = Fixture("handoff.json");
        long now = fx["now"].AsLong.Value;
        var token = fx["cases"][0].Str("token");
        var pinned = fx["keys"].Fields.ToDictionary(kv => kv.Key, kv => kv.Value.AsString);
        var door = new HandoffDoor(fx.Str("game"), new[] { fx.Str("to").ToUpperInvariant() }, keys: pinned, clock: () => now);
        Check("a door admits a good handoff", (await door.Admit(token)).Ok);
        Check("…once", (await door.Admit(token)).Code == "used");

        var fake = new Fake();
        int reads = 0;
        fake.Answer = (m, u, b) => { reads++; return new HttpReply(200, JsonValue.Object().Set("game", "my-game").Set("keys", JsonValue.Array(new[] { JsonValue.Object().Set("id", "island-1").Set("pub", fx.Str("public_b64")) })).ToString()); };
        long t = now;
        var read = new HandoffDoor(fx.Str("game"), new[] { fx.Str("to") }, new Client("https://elopros.com", fake), clock: () => t);
        Check("a door with a client reads the keys", (await read.Admit(token)).Ok && reads == 1 && fake.Sent.Last().url.EndsWith("/api/store/serverkeys/my-game"));
        Check("…and does not read them again within five minutes", (await read.Admit(fx["cases"][1].Str("token"))).Ok && reads == 1);
        fake.Answer = (m, u, b) => new HttpReply(503, "{\"why\":\"down\"}");
        var dark = new HandoffDoor(fx.Str("game"), new[] { fx.Str("to") }, new Client("https://elopros.com", fake), clock: () => now);
        Check("no keys and no origin is no_keys, not bad_signature", (await dark.Admit(token)).Code == "no_keys");
    }

    // ── a player's signed changes ──────────────────────────────────────────

    static async Task SignedChangeTests()
    {
        Section("signed changes — the text is rebuilt, never taken");
        var fr = Fixture("friends_texts.json");
        foreach (var c in fr["cases"].Items)
        {
            var text = Texts.FriendsSignText(c.Str("wallet"), c.Str("op"), c.Str("other"), c["at"].AsLong.Value);
            Check($"friends {c.Str("op")} text is the meter's", text == c.Str("text"), text);
        }
        foreach (var c in Fixture("face.json")["cases"].Items)
        {
            var f = c["fields"];
            var detail = Texts.FaceDetail(c["at"].AsLong.Value, f.Str("name"), f.Str("picture_sha256"), f.Str("kind"), f.Str("harness"), f.Str("model"), f["avatar"].AsLong);
            Check($"face detail at {c["at"].AsLong} is the meter's", detail == c.Str("detail"), detail);
        }
        Check("canonical JSON escapes non-ASCII as Python does", JsonValue.Object().Set("b", "é").Set("a", 1).Canonical() == "{\"a\":1,\"b\":\"\\u00e9\"}");

        var c0 = fr["cases"][0];
        var fake = new Fake();
        var api = new Client("https://elopros.com", fake);
        int signs = 0;
        Func<string, Task<SignResult>> signer = text => { signs++; return Task.FromResult(new SignResult { Outcome = SignOutcome.Signed, Signature = "0xsig" }); };
        fake.Answer = (m, u, b) => u.EndsWith("/prepare")
            ? new HttpReply(200, JsonValue.Object().Set("at", c0["at"].AsLong.Value).Set("sign_this", c0.Str("text")).ToString())
            : new HttpReply(200, "{\"ok\":true}");
        var ok = await api.Friend(c0.Str("wallet"), c0.Str("op"), c0.Str("other"), signer);
        var act = JsonValue.Parse(fake.Sent.Last().body);
        Check("a friends change signs the rebuilt text once and acts", ok.IsAnswer && signs == 1 && fake.Sent.Last().url.EndsWith("/api/friends/act")
            && act.Str("signature") == "0xsig" && act["at"].AsLong == c0["at"].AsLong);
        fake.Answer = (m, u, b) => new HttpReply(200, JsonValue.Object().Set("at", c0["at"].AsLong.Value).Set("sign_this", c0.Str("text").Replace("ask", "block")).ToString());
        signs = 0;
        var swapped = await api.Friend(c0.Str("wallet"), c0.Str("op"), c0.Str("other"), signer);
        Check("a server that offers another text gets no signature", swapped.IsRefused && signs == 0);
        Check("an unknown op is refused here", (await api.Friend(c0.Str("wallet"), "befriend", c0.Str("other"), signer)).IsRefused);
        Check("your own wallet is refused", (await api.Friend(c0.Str("wallet"), "ask", c0.Str("wallet"), signer)).IsRefused);
        Func<string, Task<SignResult>> no = _ => Task.FromResult(SignResult.Fail(SignOutcome.RefusedByPlayer, "no"));
        fake.Answer = (m, u, b) => new HttpReply(200, JsonValue.Object().Set("at", c0["at"].AsLong.Value).Set("sign_this", c0.Str("text")).ToString());
        Check("a player's no is a refusal and nothing is sent", (await api.Friend(c0.Str("wallet"), c0.Str("op"), c0.Str("other"), no)).IsRefused && !fake.Sent.Last().url.EndsWith("/act"));

        // face: the server keys the day to today, so prepare offers today's text
        long now = DateTimeOffset.UtcNow.ToUnixTimeSeconds();
        var w = "0x2222222222222222222222222222222222222222";
        var detail2 = Texts.FaceDetail(now, "ash", null, "human");
        fake.Answer = (m, u, b) => u.EndsWith("/face/prepare")
            ? new HttpReply(200, JsonValue.Object().Set("at", now).Set("detail", detail2).Set("sign_this", Texts.PlayMessage("profile", w, detail2, Texts.DayOf(now))).ToString())
            : new HttpReply(200, "{\"wallet\":\"" + w + "\",\"name\":\"ash\"}");
        signs = 0;
        var face = await api.SetFace(w, new FaceChange { Name = "ash", Kind = "human" }, signer);
        var sent = JsonValue.Parse(fake.Sent.Last().body);
        Check("a face change signs once and posts the change", face.IsAnswer && signs == 1 && sent.Str("name") == "ash" && sent.Str("signature") == "0xsig", face.ToString());
        foreach (var (bad, ch) in new[] { ("a long name", new FaceChange { Name = "abcdefghijklmnopq" }), ("a 0x name", new FaceChange { Name = "0xdead" }),
                                          ("a bad kind", new FaceChange { Kind = "robot" }), ("an avatar without its top bit", new FaceChange { Avatar = 5 }),
                                          ("nothing", new FaceChange()) })
            Check($"{bad} is refused here", (await api.SetFace(w, ch, signer)).IsRefused);
    }

    // ── a game server's calls ──────────────────────────────────────────────

    static async Task ServerCallTests()
    {
        Section("server calls — signed, and read back");
        var fx = Fixture("server_texts.json");
        var key = new ServerKey("my-game", "eu-1", Convert.FromHexString(fx.Str("seed_hex")));
        var fake = new Fake();
        var api = new Client("https://elopros.com", fake);
        var w = "0x1111111111111111111111111111111111111111";
        int before;

        fake.Answer = (m, u, b) => new HttpReply(200, "{\"etag\":\"e1\",\"size\":11,\"version\":1}");
        var put = await api.CloudPut(key, w, "save", Utf8("{\"level\":3}"), ifMatch: "e0");
        var r = fake.Last;
        Check("a cloud put goes to /api/cloud/…", put.IsAnswer && r.Method == "PUT" && r.Url == "https://elopros.com/api/cloud/my-game/" + w + "/save");
        var text = ServerKey.ServerText("PUT", "/cloud/my-game/" + w + "/save", "my-game/eu-1", r.Headers["X-Elo-Time"], Utf8("{\"level\":3}"), r.Headers["X-Elo-Nonce"]);
        Check("…signed over the path without /api and the body's hash",
            Ed25519.Verify(Convert.FromBase64String(key.PublicB64), Utf8(text), Convert.FromBase64String(r.Headers["X-Elo-Signature"])));
        Check("…carrying If-Match and the octet-stream type", r.Headers["If-Match"] == "e0" && r.ContentType == "application/octet-stream");
        await api.CloudPut(key, w, "save", Utf8("x"), createOnly: true);
        Check("create-only sends If-None-Match: *", fake.Last.Headers["If-None-Match"] == "*");

        fake.Answer = (m, u, b) => new HttpReply(200, Utf8("raw save bytes"), new Dictionary<string, string> { ["etag"] = "\"e2\"", ["x-elo-version"] = "4" });
        var got = await api.CloudGet(key, w, "save");
        Check("a cloud get answers the bytes, etag and version", got.IsAnswer && System.Text.Encoding.UTF8.GetString(got.Value.Data) == "raw save bytes" && got.Value.Etag == "e2" && got.Value.Version == 4);
        Check("…and a GET sends no body", fake.Last.Body == null);
        fake.Answer = (m, u, b) => new HttpReply(404, "{\"error\":\"no save in that slot\"}");
        Check("no save is a refusal, not an empty one", (await api.CloudGet(key, w, "save")).IsRefused);
        Check("a bad slot is refused here", (await api.CloudPut(key, w, "Bad Slot", Utf8("x"))).IsRefused);

        fake.Answer = (m, u, b) => new HttpReply(200, "{\"ok\":true}");
        var reports = JsonValue.Array(new[] { JsonValue.Object().Set("wallet", w).Set("stats", JsonValue.Object().Set("kills", 3)).Set("scores", JsonValue.Object().Set("speedrun", 9123)) });
        var st = await api.StatsReport(key, reports);
        Check("stats go to the game's report route, signed", st.IsAnswer && fake.Last.Url.EndsWith("/api/gamestats/my-game/report") && fake.Last.Headers.ContainsKey("X-Elo-Signature")
            && JsonValue.Parse(System.Text.Encoding.UTF8.GetString(fake.Last.Body))["reports"][0]["stats"]["kills"].AsLong == 3);
        Check("an empty report is refused here", (await api.StatsReport(key, JsonValue.Array(new JsonValue[0]))).IsRefused);
        await api.ServerCheck(key);
        Check("the server check posts to /api/serverkeys/check", fake.Last.Method == "POST" && fake.Last.Url.EndsWith("/api/serverkeys/check"));

        fake.Answer = (m, u, b) => new HttpReply(200, "{\"access\":true}");
        Check("playtest access reads a bool", (await api.PlaytestAccess("my-game", w)).Value);
        fake.Answer = (m, u, b) => new HttpReply(200, "{\"addr\":\"play.example:7000\"}");
        Check("quick join asks for the party", (await api.QuickJoin("my-game", 3)).IsAnswer && fake.Last.Url.EndsWith("/api/lobbies/pick/my-game?party=3"));
        Check("a party over 64 is refused", (await api.QuickJoin("my-game", 65)).IsRefused);
        Check("a lobby id is checked", (await api.Lobby("short")).IsRefused && (await api.Lobby("abcdefghijklmnop")).IsAnswer);
        await api.TicketBuy("my-game", w, "eth");
        Check("ticket buy asks for the rail and wallet", fake.Last.Url.EndsWith("/api/ticket/my-game/prepare?rail=eth&wallet=" + w));
        Check("a minidump is MDMP, a backtrace UTF-8 text, a core file nothing",
            Client.DumpFormat(Utf8("MDMP\u0001rest")) == "minidump" && Client.DumpFormat(Utf8("panicked at src/main.rs:3")) == "backtrace"
            && Client.DumpFormat(new byte[] { 0x7f, (byte)'E', (byte)'L', (byte)'F', 1 }) == null && Client.DumpFormat(new byte[] { 0x41, 0, 0x42 }) == null
            && Client.DumpFormat(new byte[] { 0xff, 0xfe }) == null);
        var crash = JsonValue.Object().Set("kind", "crash").Set("title", "crashed on load");
        before = fake.Sent.Count;
        Check("a dump the player did not choose is refused here", (await api.ReportBug("my-game", crash, dump: Utf8("MDMPx"))).IsRefused);
        Check("a dump on a report that is not a crash is refused", (await api.ReportBug("my-game", JsonValue.Object().Set("kind", "bug"), dump: Utf8("MDMPx"), playerChoseDump: true)).IsRefused);
        Check("an oversize backtrace is refused", (await api.ReportBug("my-game", crash, dump: Utf8(new string('a', Client.BacktraceMax + 1)), playerChoseDump: true)).IsRefused);
        Check("…and none of those were sent", fake.Sent.Count == before);
        fake.Answer = (m, u, b) => new HttpReply(201, "{\"ok\":true,\"id\":\"r1\",\"dump\":true}");
        var sentBug = await api.ReportBug("my-game", crash, dump: Utf8("MDMPdump"), playerChoseDump: true);
        var bugBody = JsonValue.Parse(fake.Sent.Last().body);
        Check("a chosen dump goes with its format and consent", sentBug.IsAnswer && fake.Sent.Last().url.EndsWith("/api/bugs")
            && bugBody["dump"].Str("format") == "minidump" && bugBody["dump"].IsTrue("consent") && bugBody["dump"].Str("b64") == Convert.ToBase64String(Utf8("MDMPdump")));
        Check("guild tags take 1 to 100 wallets", (await api.GuildTags(Enumerable.Range(0, 101).Select(i => "0x" + i.ToString("x40")))).IsRefused);
    }

    // ── the launcher, for real ─────────────────────────────────────────────

    sealed class Broker : IDisposable
    {
        public readonly string Dir = Path.Combine(Path.GetTempPath(), "elo-unity-" + Guid.NewGuid().ToString("N"));
        public string Socket => Path.Combine(Dir, "launcher.sock");
        readonly Process proc;

        public Broker(string bin, bool refuseConsent = false)
        {
            Directory.CreateDirectory(Dir);
            var psi = new ProcessStartInfo(bin) { RedirectStandardOutput = true, RedirectStandardError = true, UseShellExecute = false };
            psi.ArgumentList.Add(Socket);
            if (refuseConsent) psi.ArgumentList.Add("--refuse-consent");
            proc = Process.Start(psi);
            // wait for the bind, never for the file to appear
            var line = proc.StandardOutput.ReadLine() ?? "";
            if (!line.StartsWith("ready")) throw new Exception("broker did not come up: " + line);
        }

        public void Dispose()
        {
            try { proc.Kill(); proc.WaitForExit(5000); } catch { }
            try { Directory.Delete(Dir, true); } catch { }
        }
    }

    static async Task BrokerTests()
    {
        Section("the launcher — against the real broker");
        var bin = Path.Combine(Repo(), "launcher-rs", "target", "debug", "examples", "headless");
        if (!File.Exists(bin))
        {
            skipped.Add("the launcher half — build it: cd launcher-rs && cargo build --example headless -p elo-broker");
            return;
        }

        // ⚠ The default MUST agree with the broker's transport.rs, or a game
        // finds no launcher on a machine running one and nothing goes red.
        var transport = File.ReadAllText(Path.Combine(Repo(), "launcher-rs", "crates", "elo-broker", "src", "transport.rs"));
        foreach (var lit in new[] { "{rt}/elo/launcher.sock", "{home}/.cache/elo/launcher/launcher.sock", "./elo-launcher.sock", @"\\.\pipe\elo-launcher-{user}", "is_dir()" })
            Check($"transport.rs still says {lit}", transport.Contains(lit));
        string[] keys = { "ELO_LAUNCHER_SOCKET", "XDG_RUNTIME_DIR", "HOME" };
        var saved = keys.Select(Environment.GetEnvironmentVariable).ToArray();
        try
        {
            var rtDir = Directory.CreateTempSubdirectory("elo-rt").FullName;
            Environment.SetEnvironmentVariable("ELO_LAUNCHER_SOCKET", "/tmp/given.sock");
            Check("ELO_LAUNCHER_SOCKET wins", Overlay.DefaultSocket() == "/tmp/given.sock");
            Environment.SetEnvironmentVariable("ELO_LAUNCHER_SOCKET", null);
            Environment.SetEnvironmentVariable("XDG_RUNTIME_DIR", rtDir);
            Environment.SetEnvironmentVariable("HOME", "/home/p");
            Check("XDG_RUNTIME_DIR when it is a directory", Overlay.DefaultSocket() == rtDir + "/elo/launcher.sock", Overlay.DefaultSocket());
            Environment.SetEnvironmentVariable("XDG_RUNTIME_DIR", rtDir + "/gone");
            Check("…else the cache dir", Overlay.DefaultSocket() == "/home/p/.cache/elo/launcher/launcher.sock", Overlay.DefaultSocket());
            Environment.SetEnvironmentVariable("HOME", null);
            Check("…else beside the game, as the broker does", Overlay.DefaultSocket() == "./elo-launcher.sock", Overlay.DefaultSocket());
            Directory.Delete(rtDir);
        }
        finally
        {
            for (int i = 0; i < keys.Length; i++) Environment.SetEnvironmentVariable(keys[i], saved[i]);
        }

        var none = Overlay.Connect("gates", "1.0.0", Path.Combine(Path.GetTempPath(), "no-launcher-here.sock"), TimeSpan.FromMilliseconds(200), out var why);
        Check("no launcher is null with a reason, not a throw", none == null && !string.IsNullOrEmpty(why), why);
        Check("a game with no name gets no door", Overlay.Connect("", "1", "/nowhere", TimeSpan.FromSeconds(1), out _) == null);

        using (var br = new Broker(bin))
        {
            var ov = Overlay.Connect("gates", "1.0.0", br.Socket, TimeSpan.FromSeconds(10), out why);
            Check("the C# client connects to the Rust broker", ov != null, why);
            if (ov == null) return;
            Check("hello names the signer", ov.Signer == "local", ov.Signer);
            Check("hello speaks protocol 1", ov.LauncherProtocol == 1);
            Check("hello advertises the vocabulary", ov.Hello["capabilities"].Count > 0);
            var addr = ov.Address();
            Check("identity answers an address", addr != null && addr.StartsWith("0x"), addr);

            const string Nonce = "8f14e45fceea167a", Domain = "shard-3.gates.example";
            var p = ov.Prove(Domain, Nonce);
            Check("prove returns a signature", p.Ok && p.Signature.StartsWith("0x"), p.Reason);
            Check("the domain leads the message", p.Message.StartsWith(Domain + " wants you to sign in with your Ethereum account:\n"));
            Check("the address reported is the address in the message", p.Address != null && p.Message.Contains(p.Address));
            Check("the nonce is in the signed bytes", p.Message.Contains("\nNonce: " + Nonce + "\n"));
            Check("the game names itself", p.Message.Contains("\nProve your identity to play gates."));
            Check("the chain is bound", p.Message.Contains("\nChain ID: 4663\n"));
            var dashed = ov.Prove(Domain, "550e8400-e29b-41d4-a716-446655440000");
            Check("a dashed uuid nonce is refused by the launcher", !dashed.Ok && dashed.Outcome == SignOutcome.Refused && dashed.Reason.Contains("alphanumeric"), dashed.Reason);
            var url = ov.Prove("https://s.example", Nonce);
            Check("a url is not a server", !url.Ok && url.Reason.Contains("not a url"), url.Reason);
            Check("issued_at is passed through", ov.Prove(Domain, Nonce, DateTimeOffset.UtcNow.ToUnixTimeSeconds()).Ok);

            var signed = ov.Sign(Texts.PlayMessage("duel", addr, "ETH up 5"), "settling round 41");
            Check("sign returns a signature when the player allows it", signed.Ok && signed.Family == "play", signed.Reason);

            var me = ov.Profile();
            Check("profile answers for the watched address", me.IsAnswer && me.Value.Found, me.ToString());
            Check("profile avatars are made absolute on the launcher's host",
                me.Value.AvatarUrl == null || me.Value.AvatarUrl.StartsWith((ov.Host ?? "").TrimEnd('/') + "/api/"), me.Value.AvatarUrl + " / " + ov.Host);
            var dead = ov.Profile("0x000000000000000000000000000000000000dEaD");
            Check("looked and absent is Found=false, not could-not-look", dead.IsAnswer && !dead.Value.Found, dead.ToString());
            Check("a malformed address is refused here", ov.Profile("nope").IsRefused);

            Check("title answers a url", ov.TitleUrl() != null);
            Check("servers answers null when a title publishes no list", ov.ServersUrl() == null);
            var hud = ov.Snapshot();
            Check("overlay returns a snapshot a HUD can draw", hud != null && hud.IsObject);
            foreach (var k in new[] { "token", "approve", "allow", "pending", "grant" })
                Check($"the snapshot carries no {k} key", hud != null && !hud.Has(k));
            Check("open refuses anything that is not https", !ov.Open("file:///etc/passwd"));
            Check("an unknown field is refused, not ignored", ov.Call(JsonValue.Object().Set("op", "identity").Set("wallet", "0xdead")).IsTrue("ok") == false);

            // Dispose wakes nothing it should not, and the door is closed after.
            ov.Dispose();
            Check("a closed overlay answers null, not a throw", ov.Address() == null);
            Check("sign on a closed overlay is NoLauncher", ov.Sign("elo play", "x").Outcome == SignOutcome.NoLauncher);
        }

        using (var br = new Broker(bin, refuseConsent: true))
        {
            var ov = Overlay.Connect("gates", "1.0.0", br.Socket, TimeSpan.FromSeconds(10), out why);
            var no = ov.Sign(Texts.PlayMessage("duel", ov.Address(), "x"), "why");
            Check("a refused prompt is RefusedByPlayer, not an error", no.Outcome == SignOutcome.RefusedByPlayer, $"{no.Outcome}: {no.Reason}");
            Check("prove still works — it asks nobody", ov.Prove("s.example", "8f14e45fceea167a").Ok);
            ov.Dispose();
        }

        Section("Elo — the facade a game calls");
        Check("Init with no launcher is false and anonymous", !Elo.Init("gates", "1.0.0", socket: Path.Combine(Path.GetTempPath(), "nope.sock")) && Elo.Mode == EloMode.Anonymous && Elo.WhyAnonymous != null);
        var anon = await Elo.SignIn("s.example", "8f14e45fceea167a");
        Check("SignIn anonymous is NoLauncher", anon.Outcome == SignOutcome.NoLauncher);
        Check("Profile with nobody signed in is Found=false", (await Elo.Profile()).Value is Profile pr && !pr.Found);
        Check("Owns with nobody signed in is refused", (await Elo.Owns()).IsRefused);
        Check("Signer anonymous is none", Elo.Signer == "none");

        using (var br = new Broker(bin))
        {
            Check("Init with a launcher is true", Elo.Init("gates", "1.0.0", socket: br.Socket) && Elo.Mode == EloMode.Launcher, Elo.WhyAnonymous);
            var a = await Elo.Connect();
            Check("Connect answers the launcher's address", a != null && a == Elo.Address);
            var s = await Elo.SignIn("shard-3.gates.example", Texts.NewNonce());
            Check("SignIn proves through the launcher", s.Ok && s.Address == a, s.Reason);
            var sig = await Elo.Sign(Texts.PlayMessage("duel", a, "ETH up 5"), "settling round 41");
            Check("Sign signs through the launcher", sig.Ok);
            var prof = await Elo.Profile();
            Check("Profile reads through the launcher", prof.IsAnswer && prof.Value.Found);
            Check("calls from many tasks queue on one line", (await Task.WhenAll(Enumerable.Range(0, 20).Select(_ => Elo.Connect()))).All(x => x == a));
            Elo.Shutdown();
            Check("Shutdown leaves the game anonymous", Elo.Mode == EloMode.Anonymous && Elo.Launcher == null);
            Elo.Shutdown();
        }
    }
}

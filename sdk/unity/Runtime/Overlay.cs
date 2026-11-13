// The launcher overlay client: what a desktop game says to the Elo launcher
// running on the same machine. The wire is sdk/PROTOCOL.md (newline-delimited
// JSON on a UNIX socket, or a named pipe on Windows); this is the C# twin of
// sdk/rust/elo_overlay.rs and sdk/python/elo_overlay.py.
//
// No launcher is a normal state: play anonymously, and never ask the player
// for a key instead. There is no third option and this file does not offer one.
//
// Every call blocks. Call it off your main thread — Elo (Elo.cs) does that
// for you and hands the answer back on Unity's main thread.

using System;
using System.IO;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;

namespace EloPros
{
    public enum SignOutcome
    {
        /// <summary>A signature came back.</summary>
        Signed,
        /// <summary>The player said no at the consent prompt. Show it; do not retry.</summary>
        RefusedByPlayer,
        /// <summary>The player's signer is their browser: show <see cref="SignResult.HandoffUrl"/>.
        /// Not an error, and never retry it in a loop.</summary>
        Handoff,
        /// <summary>The signer refused (unknown family, budget spent, unarmed): show the reason.</summary>
        Refused,
        /// <summary>No launcher or wallet, or the connection died.</summary>
        NoLauncher,
    }

    /// <summary>What a `sign` answered.</summary>
    public sealed class SignResult
    {
        public SignOutcome Outcome { get; internal set; }
        public bool Ok => Outcome == SignOutcome.Signed;
        public string Signature { get; internal set; }
        public string Address { get; internal set; }
        public string Family { get; internal set; }
        public string Backend { get; internal set; }
        /// <summary>Written for a player to read.</summary>
        public string Reason { get; internal set; } = "";
        public string HandoffUrl { get; internal set; }

        internal static SignResult Fail(SignOutcome o, string reason) => new SignResult { Outcome = o, Reason = reason ?? "" };
        public override string ToString() => Ok ? $"signed by {Address}" : Outcome == SignOutcome.Handoff ? $"finish this in your browser: {HandoffUrl}" : Reason;
    }

    /// <summary>A sign-in your server can check: a SIWE (EIP-4361) message
    /// and its signature, from the launcher's `prove` or the web SDK's
    /// `signIn`. Send both to your server, which checks them with
    /// <see cref="Client.CheckSignIn"/> (or any siwe library) against the
    /// domain and nonce IT issued. <see cref="Message"/> is an echo, not
    /// evidence: never act on its contents without that check.</summary>
    public sealed class SignIn
    {
        public SignOutcome Outcome { get; internal set; }
        public bool Ok => Outcome == SignOutcome.Signed;
        public string Message { get; internal set; }
        public string Signature { get; internal set; }
        /// <summary>A claim until your server checks the signature.</summary>
        public string Address { get; internal set; }
        /// <summary>`evm` or `solana`.</summary>
        public string Chain { get; internal set; } = "evm";
        public string Reason { get; internal set; } = "";

        internal static SignIn Fail(SignOutcome o, string reason) => new SignIn { Outcome = o, Reason = reason ?? "" };
        public override string ToString() => Ok ? $"sign-in from {Address}" : Reason;
    }

    public sealed class Overlay : IDisposable
    {
        public const int Protocol = 1;
        public const string SocketEnv = "ELO_LAUNCHER_SOCKET";
        /// <summary>A consent prompt waits for a person, and the launcher gives
        /// up at the same mark, so a prompt nobody answers comes back as a
        /// refusal instead of hanging either side.</summary>
        public static readonly TimeSpan DefaultTimeout = TimeSpan.FromSeconds(300);

        readonly Stream _stream;
        readonly StreamReader _reader;
        readonly Socket _socket;
        readonly object _lock = new object();
        int _closed;

        /// <summary>The `hello` reply.</summary>
        public JsonValue Hello { get; private set; } = JsonValue.Null();
        public string Game { get; }

        /// <summary>Which backend holds the key: `local` · `browser-paired` ·
        /// `arca` · `external` · `browser` · `none`. With `none` there is
        /// nothing to ask; with `browser` expect a handoff, not a signature.</summary>
        public string Signer => Hello.StrOrEmpty("signer");
        public int LauncherProtocol => (int)(Hello["protocol"].AsLong ?? 0);
        /// <summary>The launcher's origin, e.g. https://elopros.com.</summary>
        public string Host => Hello.Str("host");

        Overlay(Stream stream, Socket socket, string game)
        {
            _stream = stream;
            _socket = socket;
            _reader = new StreamReader(stream, new UTF8Encoding(false), false, 4096, true);
            Game = game;
        }

        // ── finding the door ───────────────────────────────────────────────

        static bool IsWindows => RuntimeInformation.IsOSPlatform(OSPlatform.Windows);

        /// <summary>Where the launcher listens. `$ELO_LAUNCHER_SOCKET` (set by
        /// the launcher on every native build it starts) wins; otherwise the
        /// platform default, which MUST agree with
        /// launcher-rs/crates/elo-broker/src/transport.rs.</summary>
        public static string DefaultSocket()
        {
            var env = Environment.GetEnvironmentVariable(SocketEnv);
            if (!string.IsNullOrEmpty(env)) return env;
            if (IsWindows)
            {
                var user = Environment.GetEnvironmentVariable("USERNAME") ?? "";
                var sb = new StringBuilder();
                foreach (char c in user)
                    if ((c < 128 && char.IsLetterOrDigit(c)) || c == '-' || c == '_') sb.Append(c);
                return @"\\.\pipe\elo-launcher-" + (sb.Length == 0 ? "default" : sb.ToString());
            }
            // XDG_RUNTIME_DIR when it is a directory, else the cache dir — the
            // broker's rule exactly, down to its fallback with no HOME.
            var rt = Environment.GetEnvironmentVariable("XDG_RUNTIME_DIR");
            if (!string.IsNullOrEmpty(rt) && Directory.Exists(rt)) return rt + "/elo/launcher.sock";
            var home = Environment.GetEnvironmentVariable("HOME") ?? "";
            if (home.Length == 0) return "./elo-launcher.sock";
            return home + "/.cache/elo/launcher/launcher.sock";
        }

        /// <summary>Connect and say hello. Returns null, with `why`, when there
        /// is no launcher — a normal state, and the game plays on without it.</summary>
        public static Overlay Connect(string game, string version, out string why) =>
            Connect(game, version, DefaultSocket(), DefaultTimeout, out why);

        public static Overlay Connect(string game, string version, string path, TimeSpan timeout, out string why)
        {
            why = null;
            if (string.IsNullOrEmpty(game)) { why = "a game must name itself: a consent prompt with no name in it is not consent"; return null; }
            Overlay ov;
            try
            {
                ov = Open(path, timeout, game);
            }
            catch (Exception e)
            {
                why = $"no launcher at {path} ({e.Message})";
                return null;
            }
            try
            {
                var hello = JsonValue.Object()
                    .Set("op", "hello").Set("game", game)
                    .Set("protocol", Protocol).Set("version", version ?? "0");
                var reply = ov.Call(hello);
                if (!reply.IsTrue("ok")) { why = reply.StrOrEmpty("reason"); ov.Dispose(); return null; }
                ov.Hello = reply;
                return ov;
            }
            catch (Exception e)
            {
                why = $"the launcher did not answer hello ({e.Message})";
                ov.Dispose();
                return null;
            }
        }

        static Overlay Open(string path, TimeSpan timeout, string game)
        {
            if (IsWindows && path.StartsWith(@"\\.\pipe\", StringComparison.OrdinalIgnoreCase))
                return new Overlay(WindowsPipe.Open(path, timeout), null, game);
            var sock = new Socket(AddressFamily.Unix, SocketType.Stream, ProtocolType.Unspecified);
            try
            {
                sock.Connect(new UnixDomainSocketEndPoint(path));
                int ms = (int)Math.Min(int.MaxValue, timeout.TotalMilliseconds);
                sock.ReceiveTimeout = ms;
                sock.SendTimeout = ms;
                return new Overlay(new NetworkStream(sock, true), sock, game);
            }
            catch
            {
                sock.Dispose();
                throw;
            }
        }

        // ── the verbs ──────────────────────────────────────────────────────

        /// <summary>The address the player asked their launcher to watch. A
        /// CLAIM, not authentication; null when none is set. Use
        /// <see cref="Prove"/> when the answer matters.</summary>
        public string Address()
        {
            var r = TryCall(JsonValue.Object().Set("op", "identity"));
            return r != null && r.IsTrue("ok") ? r.Str("address") : null;
        }

        /// <summary>Ask the player to sign `text`, an EIP-191 message whose
        /// first line is `elo &lt;family&gt;` (<see cref="Texts.PlayMessage"/>
        /// builds `elo play`). The launcher shows the text verbatim and `why`
        /// above it, in its own window, and waits up to 300 s for a person.</summary>
        public SignResult Sign(string text, string why)
        {
            JsonValue r;
            try { r = Call(JsonValue.Object().Set("op", "sign").Set("text", text ?? "").Set("why", why ?? "")); }
            catch (Exception e) { return SignResult.Fail(SignOutcome.NoLauncher, e.Message); }
            if (r.IsTrue("ok"))
                return new SignResult
                {
                    Outcome = SignOutcome.Signed,
                    Signature = r.StrOrEmpty("signature"),
                    Address = r.Str("address"),
                    Family = r.StrOrEmpty("family"),
                    Backend = r.StrOrEmpty("backend"),
                };
            var reason = r.StrOrEmpty("reason");
            var handoff = r.Str("handoff");
            if (!string.IsNullOrEmpty(handoff)) return new SignResult { Outcome = SignOutcome.Handoff, HandoffUrl = handoff, Reason = reason };
            if (r.Str("refused_by") == "the player") return SignResult.Fail(SignOutcome.RefusedByPlayer, reason);
            return SignResult.Fail(SignOutcome.Refused, reason);
        }

        /// <summary>Prove to your own server who is playing: a SIWE (EIP-4361)
        /// sign-in for `server` (your domain, `host` or `host:port`, no scheme
        /// or path) on `nonce` (8+ letters and digits that YOUR server issued
        /// and will retire). The launcher writes every word, so it signs
        /// without a prompt for a host your shard list names.
        /// `issuedAt` (unix seconds) is for a server whose challenge already
        /// carries its own timestamp; leave it null otherwise.</summary>
        public SignIn Prove(string server, string nonce, long? issuedAt = null)
        {
            var req = JsonValue.Object().Set("op", "prove").Set("nonce", nonce ?? "").Set("server", server ?? "");
            if (issuedAt.HasValue) req.Set("issued_at", issuedAt.Value);
            JsonValue r;
            try { r = Call(req); }
            catch (Exception e) { return SignIn.Fail(SignOutcome.NoLauncher, e.Message); }
            if (r.IsTrue("ok"))
                return new SignIn
                {
                    Outcome = SignOutcome.Signed,
                    Signature = r.StrOrEmpty("signature"),
                    Address = r.Str("address"),
                    Message = r.StrOrEmpty("message"),
                    Chain = "evm",
                };
            if (r.Str("refused_by") == "the player") return SignIn.Fail(SignOutcome.RefusedByPlayer, r.StrOrEmpty("reason"));
            return SignIn.Fail(SignOutcome.Refused, r.StrOrEmpty("reason"));
        }

        /// <summary>A name and a face to draw. `address` null is the address
        /// this launcher watches. Three states: an answer with a Profile; an
        /// answer with <see cref="Profile.Found"/> false (nothing is set — an
        /// anonymous player); could-not-look (draw nothing about identity).
        /// It describes an address; it is not authentication.</summary>
        public Look<Profile> Profile(string address = null)
        {
            var req = JsonValue.Object().Set("op", "profile");
            if (!string.IsNullOrEmpty(address))
            {
                if (!Texts.IsWallet(address) && !Texts.IsSolana(address))
                    return Look<Profile>.Refused(Refusal.Here("address must be 0x + 40 hex, or a Solana address"));
                req.Set("address", address);
            }
            JsonValue r;
            try { r = Call(req); }
            catch (Exception e) { return Look<Profile>.CouldNotLook(e.Message); }
            if (!r.IsTrue("ok")) return Look<Profile>.CouldNotLook(r.Str("reason") ?? "the launcher could not look");
            if (r["reachable"].AsBool == false) return Look<Profile>.CouldNotLook(r.Str("reason") ?? "the origin is unreachable");
            var p = r["profile"];
            if (p.IsNull) return Look<Profile>.Answer(EloPros.Profile.Nothing(r.Str("address") ?? address));
            return Look<Profile>.Answer(EloPros.Profile.FromFlat(p, Host));
        }

        /// <summary>One snapshot a game can DRAW, cheap enough to poll each
        /// frame. Display freely; never approve: it carries no consent token,
        /// and a HUD that draws its own "allow this signature?" is drawing a
        /// forgery. Call <see cref="Sign"/> and let the launcher ask.</summary>
        public JsonValue Snapshot(string slug = null)
        {
            var r = TryCall(JsonValue.Object().Set("op", "overlay").Set("slug", slug ?? Game));
            return r != null && r.IsTrue("ok") ? r["overlay"] : null;
        }

        /// <summary>The URL of this title's manifest, which YOU fetch.</summary>
        public string TitleUrl(string slug = null) => UrlOf("title", slug);

        /// <summary>The URL of this title's shard list, which YOU fetch. The
        /// launcher does not proxy, cache or rank it.</summary>
        public string ServersUrl(string slug = null) => UrlOf("servers", slug);

        string UrlOf(string op, string slug)
        {
            var r = TryCall(JsonValue.Object().Set("op", op).Set("slug", slug ?? Game));
            return r != null && r.IsTrue("ok") ? r.Str("url") : null;
        }

        /// <summary>Send the player to an https page — their coins, a store
        /// page, the handoff link from a <see cref="Sign"/>.</summary>
        public bool Open(string url)
        {
            var r = TryCall(JsonValue.Object().Set("op", "open").Set("url", url ?? ""));
            return r != null && r.IsTrue("ok");
        }

        // ── the wire ───────────────────────────────────────────────────────

        JsonValue TryCall(JsonValue req)
        {
            try { return Call(req); } catch { return null; }
        }

        /// <summary>One request line, one reply line, in order. Calls from
        /// several threads queue: a `sign` waiting for a person holds the line.</summary>
        public JsonValue Call(JsonValue req)
        {
            lock (_lock)
            {
                if (_closed != 0) throw new IOException("the launcher connection is closed");
                var bytes = Encoding.UTF8.GetBytes(req.ToString() + "\n");
                _stream.Write(bytes, 0, bytes.Length);
                _stream.Flush();
                var line = _reader.ReadLine();
                if (line == null) throw new IOException("the launcher closed the connection");
                return JsonValue.Parse(line);
            }
        }

        /// <summary>Close the door. Never waits on the line: closing the
        /// stream is what wakes a call blocked on a reply.</summary>
        public void Dispose()
        {
            if (System.Threading.Interlocked.Exchange(ref _closed, 1) != 0) return;
            try { _reader.Dispose(); } catch { }
            try { _stream.Dispose(); } catch { }
            try { _socket?.Dispose(); } catch { }
        }
    }
}

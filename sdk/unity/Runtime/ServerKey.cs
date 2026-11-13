// A game server's key — Steam's Game Server Login Token, without a token
// anyone could leak from us. Your servers sign their writes (cloud saves,
// stats, scores, handoffs) with an Ed25519 key you named on the dev desk (act
// `serverkeys`) or in elo.json (`server_keys`). We keep only the public half.
//
// This belongs on your SERVER (a Unity headless build, or any .NET process),
// never in a build players download: a key in a client is a key every player
// has.
//
// The signed text, which every SDK and the meter build the same way
// (sdk/testdata/server_texts.json pins it):
//
//     elo server
//     <METHOD>
//     <path without /api, with ?query>
//     <slug>/<key id>
//     <unix seconds>
//     <nonce>
//     <sha256 hex of the body>

using System;
using System.Collections.Generic;
using System.IO;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;

namespace EloPros
{
    public sealed class ServerKey
    {
        static readonly Regex Id = new Regex("^[a-z0-9][a-z0-9-]{0,30}$");
        static readonly Regex SlugRe = new Regex("^[a-z0-9][a-z0-9-]{0,62}$");
        const string NonceAlphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        // PKCS#8 for an Ed25519 key is this fixed prefix and the 32-byte seed.
        static readonly byte[] Pkcs8Prefix = { 0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20 };

        readonly byte[] _seed;
        public string Game { get; }
        public string KeyId { get; }
        /// <summary>Name this on the dev desk, or in elo.json's `server_keys`.</summary>
        public string PublicB64 { get; }
        public string Who => Game + "/" + KeyId;

        public ServerKey(string game, string keyId, byte[] seed)
        {
            if (game == null || !SlugRe.IsMatch(game)) throw new ArgumentException("game is the title's slug");
            if (keyId == null || !Id.IsMatch(keyId)) throw new ArgumentException("key id is lowercase letters, digits and '-', up to 31 characters");
            if (seed == null || seed.Length != 32) throw new ArgumentException("an Ed25519 private key is a 32-byte seed");
            Game = game;
            KeyId = keyId;
            _seed = (byte[])seed.Clone();
            PublicB64 = Convert.ToBase64String(Ed25519.PublicKey(_seed));
        }

        /// <summary>A new key. Keep <see cref="SeedHex"/> on the server; name <see cref="PublicB64"/> with us.</summary>
        public static ServerKey Generate(string game, string keyId)
        {
            var seed = new byte[32];
            using (var rng = RandomNumberGenerator.Create()) rng.GetBytes(seed);
            return new ServerKey(game, keyId, seed);
        }

        /// <summary>From a PKCS#8 PEM — the file `openssl genpkey -algorithm ed25519` writes.</summary>
        public static ServerKey FromPem(string game, string keyId, string pem)
        {
            var body = new StringBuilder();
            foreach (var line in (pem ?? "").Split('\n'))
            {
                var l = line.Trim();
                if (l.Length == 0 || l.StartsWith("-----", StringComparison.Ordinal)) continue;
                body.Append(l);
            }
            byte[] der;
            try { der = Convert.FromBase64String(body.ToString()); }
            catch (FormatException) { throw new ArgumentException("that PEM is not base64"); }
            if (der.Length != Pkcs8Prefix.Length + 32) throw new ArgumentException("that PEM is not an Ed25519 private key");
            for (int i = 0; i < Pkcs8Prefix.Length; i++)
                if (der[i] != Pkcs8Prefix[i]) throw new ArgumentException("that PEM is not an Ed25519 private key");
            var seed = new byte[32];
            Buffer.BlockCopy(der, Pkcs8Prefix.Length, seed, 0, 32);
            return new ServerKey(game, keyId, seed);
        }

        /// <summary>From a PEM file, or a file holding the 64-hex seed.</summary>
        public static ServerKey FromFile(string game, string keyId, string path)
        {
            var text = File.ReadAllText(path).Trim();
            if (Regex.IsMatch(text, "^[0-9a-fA-F]{64}$")) return new ServerKey(game, keyId, Hex(text));
            return FromPem(game, keyId, text);
        }

        /// <summary>The private seed, for your server's own disk. Never send it anywhere.</summary>
        public string SeedHex() => ToHex(_seed);

        public string SignText(string text) => Convert.ToBase64String(Ed25519.Sign(_seed, Encoding.UTF8.GetBytes(text)));

        internal byte[] SignBytes(byte[] data) => Ed25519.Sign(_seed, data);

        /// <summary>The exact text a server signs for one request (`meter/serverkeys.py`).</summary>
        public static string ServerText(string method, string path, string who, string at, byte[] body = null, string nonce = null)
        {
            var tail = string.IsNullOrEmpty(nonce) ? "" : nonce + "\n";
            return $"elo server\n{method.ToUpperInvariant()}\n{path}\n{who}\n{at}\n{tail}{Sha256Hex(body ?? Array.Empty<byte>())}";
        }

        /// <summary>A fresh request nonce: `n` (8 to 64) of letters, digits, '-' and '_'.</summary>
        public static string NewNonce(int n = 22)
        {
            if (n < 8 || n > 64) throw new ArgumentOutOfRangeException(nameof(n), "a request nonce is 8 to 64 characters");
            var sb = new StringBuilder(n);
            var buf = new byte[n * 2];
            using (var rng = RandomNumberGenerator.Create())
            {
                while (sb.Length < n)
                {
                    rng.GetBytes(buf);
                    foreach (var b in buf) if (sb.Length < n) sb.Append(NonceAlphabet[b & 63]);
                }
            }
            return sb.ToString();
        }

        /// <summary>The four headers for one request. `path` is the route
        /// without `/api`, with its `?query`.</summary>
        public Dictionary<string, string> Headers(string method, string path, byte[] body = null, long? at = null, string nonce = null)
        {
            var atS = (at ?? DateTimeOffset.UtcNow.ToUnixTimeSeconds()).ToString(System.Globalization.CultureInfo.InvariantCulture);
            nonce = nonce ?? NewNonce();
            return new Dictionary<string, string>
            {
                ["X-Elo-Server"] = Who,
                ["X-Elo-Time"] = atS,
                ["X-Elo-Nonce"] = nonce,
                ["X-Elo-Signature"] = SignText(ServerText(method, path, Who, atS, body, nonce)),
            };
        }

        /// <summary>A handoff moving `player` from this server (`from`,
        /// host:port) to another of this game's servers (`to`). Give the client
        /// the token, the address, and `carry` if any (docs/client/HANDOFF.md).</summary>
        public string Handoff(string player, string from, string to, byte[] carry = null, int ttl = EloPros.Handoff.TtlDefaultS, long? now = null, string id = null) =>
            EloPros.Handoff.Issue(this, player, from, to, carry, ttl, now, id);

        public override string ToString() => $"ServerKey({Who}, public={PublicB64})";

        internal static string Sha256Hex(byte[] data)
        {
            using (var sha = SHA256.Create()) return ToHex(sha.ComputeHash(data));
        }

        internal static string ToHex(byte[] b)
        {
            var sb = new StringBuilder(b.Length * 2);
            foreach (var x in b) sb.Append(x.ToString("x2", System.Globalization.CultureInfo.InvariantCulture));
            return sb.ToString();
        }

        internal static byte[] Hex(string s)
        {
            var b = new byte[s.Length / 2];
            for (int i = 0; i < b.Length; i++) b[i] = Convert.ToByte(s.Substring(2 * i, 2), 16);
            return b;
        }
    }
}

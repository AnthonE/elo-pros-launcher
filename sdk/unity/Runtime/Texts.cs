// The signed texts, rebuilt locally, and the checks every input gets before it
// is sent. The formats are the server's (meter/playauth.py); the other SDKs
// build the same bytes and sdk/testdata pins them.

using System;
using System.Globalization;
using System.Security.Cryptography;
using System.Text;

namespace EloPros
{
    public static class Texts
    {
        /// <summary>An `elo play` text: the platform's EIP-191 message for a
        /// game action. The wallet line is the subject (the signature recovers
        /// the signer), so the address is lowercased here: a checksummed one is
        /// different bytes and verifies differently. `day` is UTC
        /// `yyyy-MM-dd`, so a signature is good for that day only.
        /// `detail` is shown to the player while they decide; write it for one.</summary>
        public static string PlayMessage(string action, string wallet, string detail, string day = null)
        {
            day = day ?? UtcDay();
            return $"elo play\naction: {action}\nwallet: {(wallet ?? "").ToLowerInvariant()}\nday: {day}\ndetail: {detail}";
        }

        // ── the platform's own signed changes, rebuilt here so a client never
        //    signs whatever a server hands it (sdk/testdata pins each) ────────

        public const string FriendsAction = "friends";   // meter/friends.py
        public const string FaceAction = "profile";      // meter/faces.py

        /// <summary>Each friends op and the one argument it takes, a wallet.</summary>
        public static readonly System.Collections.Generic.IReadOnlyDictionary<string, string> FriendOps =
            new System.Collections.Generic.Dictionary<string, string>
            {
                ["ask"] = "to", ["accept"] = "from", ["decline"] = "from", ["cancel"] = "to",
                ["remove"] = "wallet", ["block"] = "wallet", ["unblock"] = "wallet", ["invite"] = "to",
            };

        /// <summary>The signed detail of a friends change, or null for an op
        /// the server does not know or an argument that is not a wallet.</summary>
        public static string FriendsDetail(string op, string other, long at)
        {
            other = (other ?? "").Trim();
            if (op == null || !FriendOps.TryGetValue(op, out var key) || !IsWallet(other)) return null;
            return JsonValue.Object()
                .Set("args", JsonValue.Object().Set(key, other.ToLowerInvariant()))
                .Set("at", at).Set("op", op).Canonical();
        }

        /// <summary>The whole text a friends change signs.</summary>
        public static string FriendsSignText(string me, string op, string other, long at)
        {
            me = (me ?? "").Trim();
            var detail = FriendsDetail(op, other, at);
            if (detail == null || !IsWallet(me)) return null;
            return PlayMessage(FriendsAction, me, detail, DayOf(at));
        }

        /// <summary>The signed detail of a face change: `at` and only the
        /// fields being changed. "" clears one (0 the avatar); null leaves it out.</summary>
        public static string FaceDetail(long at, string name = null, string pictureSha256 = null, string kind = null,
            string harness = null, string model = null, long? avatar = null)
        {
            var d = JsonValue.Object().Set("at", at);
            if (name != null) d.Set("name", name);
            if (pictureSha256 != null) d.Set("picture_sha256", pictureSha256);
            if (kind != null) d.Set("kind", kind);
            if (harness != null) d.Set("harness", harness);
            if (model != null) d.Set("model", model);
            if (avatar.HasValue) d.Set("avatar", avatar.Value);
            return d.Canonical();
        }

        /// <summary>The text a download grant signs.</summary>
        public static string EntitleText(string game, string wallet, string day = null) =>
            $"elo entitle\ngame: {game}\nwallet: {(wallet ?? "").Trim().ToLowerInvariant()}\nday: {day ?? UtcDay()}";

        /// <summary>Today, UTC, as `yyyy-MM-dd`.</summary>
        public static string UtcDay() => DayOf(DateTimeOffset.UtcNow.ToUnixTimeSeconds());

        /// <summary>The UTC day of a unix time, as the server keys it (`playauth.day_of`).</summary>
        public static string DayOf(long unixSeconds) =>
            DateTimeOffset.FromUnixTimeSeconds(unixSeconds).UtcDateTime.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture);

        const string Alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

        /// <summary>A fresh sign-in nonce: `length` letters and digits (8 to
        /// 128) from the system's CSPRNG — the alphabet EIP-4361 requires, so a
        /// dashed uuid never reaches a wallet. Issue one per sign-in and retire
        /// it once the check answers.</summary>
        public static string NewNonce(int length = 16)
        {
            if (length < 8 || length > 128) throw new ArgumentOutOfRangeException(nameof(length), "8 to 128");
            var sb = new StringBuilder(length);
            var buf = new byte[length * 2];
            using (var rng = RandomNumberGenerator.Create())
            {
                while (sb.Length < length)
                {
                    rng.GetBytes(buf);
                    foreach (var b in buf)
                    {
                        // 248 = 4 × 62: bytes above it are dropped, so no letter is likelier
                        if (b < 248 && sb.Length < length) sb.Append(Alphabet[b % 62]);
                    }
                }
            }
            return sb.ToString();
        }

        /// <summary>0x and 40 hex digits.</summary>
        public static bool IsWallet(string s)
        {
            if (s == null || s.Length != 42 || !s.StartsWith("0x", StringComparison.Ordinal)) return false;
            for (int i = 2; i < 42; i++) if (!Uri.IsHexDigit(s[i])) return false;
            return true;
        }

        /// <summary>A base58 Solana address.</summary>
        public static bool IsSolana(string s)
        {
            if (s == null || s.Length < 32 || s.Length > 44) return false;
            foreach (char c in s)
            {
                if (c > 127 || !char.IsLetterOrDigit(c) || c == '0' || c == 'O' || c == 'I' || c == 'l') return false;
            }
            return true;
        }

        /// <summary>A title slug: lowercase letters, digits, `-` and `_`.</summary>
        public static bool IsSlug(string s)
        {
            if (string.IsNullOrEmpty(s) || s.Length > 64) return false;
            if (!(IsLowerOrDigit(s[0]))) return false;
            foreach (char c in s) if (!(IsLowerOrDigit(c) || c == '-' || c == '_')) return false;
            return true;
        }

        /// <summary>EIP-4361's nonce: 8 to 128 letters and digits.</summary>
        public static bool IsNonce(string s)
        {
            if (s == null || s.Length < 8 || s.Length > 128) return false;
            foreach (char c in s) if (!IsAsciiAlnum(c)) return false;
            return true;
        }

        /// <summary>A domain authority: `host` or `host:port`. No scheme, no path.</summary>
        public static bool IsDomain(string s)
        {
            if (string.IsNullOrEmpty(s)) return false;
            string host = s, port = null;
            int colon = s.LastIndexOf(':');
            if (colon >= 0) { host = s.Substring(0, colon); port = s.Substring(colon + 1); }
            if (host.Length == 0 || host.Length > 253) return false;
            foreach (char c in host) if (!(IsAsciiAlnum(c) || c == '.' || c == '-')) return false;
            if (port != null)
            {
                if (port.Length < 1 || port.Length > 5) return false;
                foreach (char c in port) if (c < '0' || c > '9') return false;
            }
            return true;
        }

        static bool IsAsciiAlnum(char c) => (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9');
        static bool IsLowerOrDigit(char c) => (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9');

        /// <summary>Percent-encode a query value: everything but the unreserved characters.</summary>
        public static string Enc(string s)
        {
            var sb = new StringBuilder();
            foreach (var b in Encoding.UTF8.GetBytes(s ?? ""))
            {
                char c = (char)b;
                if (IsAsciiAlnum(c) || c == '-' || c == '.' || c == '_' || c == '~') sb.Append(c);
                else sb.Append('%').Append(b.ToString("X2", CultureInfo.InvariantCulture));
            }
            return sb.ToString();
        }
    }
}

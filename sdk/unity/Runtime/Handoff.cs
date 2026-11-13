// Moving a connected player from one of your servers to another
// (docs/client/HANDOFF.md). The server a player is on signs a handoff with its
// ServerKey and hands it to the client with the address to go to; the client
// joins that server with it; that server checks it offline against your
// game's public keys and admits the player it names, once. No call to Elo Pros
// per hop, and no prompt.
//
//   leaving:   var token = key.Handoff(player, "zone1.example:7000", "zone2.example:7000", carry);
//   arriving:  var door = new HandoffDoor("my-game", new[] { "zone2.example:7000" }, new Client());
//              var got = await door.Admit(token, carry);
//              if (got.Ok) Admit(got.Value.Player); else Log(got.Code);
//
// sdk/testdata/handoff.json pins the text, the token and every refusal.

using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;

namespace EloPros
{
    /// <summary>What a handoff says.</summary>
    public sealed class HandoffTicket
    {
        public string Game, Key, Player, From, To, Id;
        public long Exp;
        /// <summary>The sha256 hex of the carried bytes, or null.</summary>
        public string Carry;
    }

    /// <summary>A handoff's check: <see cref="Ok"/> with the ticket, or a
    /// <see cref="Code"/> (`malformed`, `wrong_game`, `wrong_destination`,
    /// `expired`, `too_long`, `unknown_key`, `bad_signature`, `carry`, `used`,
    /// `no_keys`) and a sentence for a log. Refused: let the player join the
    /// usual way.</summary>
    public sealed class HandoffCheck
    {
        public bool Ok => Code == null;
        public string Code { get; private set; }
        public string Why { get; private set; }
        public HandoffTicket Value { get; private set; }
        internal static HandoffCheck Good(HandoffTicket t) => new HandoffCheck { Value = t };
        internal static HandoffCheck No(string code, string why) => new HandoffCheck { Code = code, Why = why };
        public override string ToString() => Ok ? $"admit {Value.Player}" : $"{Code}: {Why}";
    }

    public static class Handoff
    {
        public const int TtlDefaultS = 60, TtlMaxS = 120, SkewS = 30, CarryMax = 4096, TokenMax = 1024;

        const string SlugP = "[a-z0-9][a-z0-9_-]{0,62}";
        const string KidP = "[a-z0-9][a-z0-9-]{0,30}";
        const string PlayerP = "0x[0-9a-f]{40}|[1-9A-HJ-NP-Za-km-z]{32,44}|acct_[0-9a-f]{20}";
        const string AddrP = @"(?:[a-z0-9.-]{1,100}|\[[0-9a-f:]{2,39}\]):[0-9]{1,5}";
        const string IdP = "[A-Za-z0-9_-]{16,64}";
        static readonly Regex Text = new Regex(
            $"^elo handoff\ngame: ({SlugP})\nkey: ({KidP})\nplayer: ({PlayerP})\nfrom: ({AddrP})\nto: ({AddrP})\nid: ({IdP})\nexp: ([0-9]{{1,12}})\ncarry: ([0-9a-f]{{64}}|none)$");
        static readonly Regex PlayerRe = new Regex($"^(?:{PlayerP})$");
        static readonly Regex AddrRe = new Regex($"^{AddrP}$");
        static readonly Regex IdRe = new Regex($"^{IdP}$");
        static readonly Regex B64u = new Regex("^[A-Za-z0-9_-]*$");

        /// <summary>The exact text a server signs for one handoff.</summary>
        public static string TextOf(string game, string keyId, string player, string from, string to, string id, long exp, byte[] carry = null)
        {
            var c = carry != null && carry.Length > 0 ? ServerKey.Sha256Hex(carry) : "none";
            return $"elo handoff\ngame: {game}\nkey: {keyId}\nplayer: {player}\nfrom: {from}\nto: {to}\nid: {id}\nexp: {exp}\ncarry: {c}";
        }

        public static string Issue(ServerKey key, string player, string from, string to, byte[] carry = null, int ttl = TtlDefaultS, long? now = null, string id = null)
        {
            if (player == null || !PlayerRe.IsMatch(player)) throw new ArgumentException("player is a 0x wallet (lowercase), a Solana address or an acct_ id");
            if (from == null || !AddrRe.IsMatch(from)) throw new ArgumentException("from is a server address, host:port, lowercase");
            if (to == null || !AddrRe.IsMatch(to)) throw new ArgumentException("to is a server address, host:port, lowercase");
            if (ttl < 1 || ttl > TtlMaxS) throw new ArgumentException($"ttl is 1 to {TtlMaxS} seconds");
            if (carry != null && carry.Length > CarryMax) throw new ArgumentException($"a handoff carries at most {CarryMax} bytes");
            id = id ?? ServerKey.NewNonce(22);
            if (!IdRe.IsMatch(id)) throw new ArgumentException("id is 16 to 64 of A-Z a-z 0-9 - _");
            long exp = (now ?? DateTimeOffset.UtcNow.ToUnixTimeSeconds()) + ttl;
            var text = Encoding.UTF8.GetBytes(TextOf(key.Game, key.KeyId, player, from, to, id, exp, carry));
            return ToB64u(text) + "." + ToB64u(key.SignBytes(text));
        }

        /// <summary>What a token says, shape only — nothing verified.</summary>
        public static HandoffCheck Read(string token)
        {
            if (token == null || token.Length > TokenMax || token.Count(c => c == '.') != 1)
                return HandoffCheck.No("malformed", "a handoff is base64url(text).base64url(signature)");
            var parts = token.Split('.');
            byte[] raw, sig;
            if (!TryB64u(parts[0], out raw) || !TryB64u(parts[1], out sig))
                return HandoffCheck.No("malformed", "a half is not base64url");
            var m = Text.Match(Encoding.UTF8.GetString(raw));
            if (!m.Success || sig.Length != 64) return HandoffCheck.No("malformed", "the text is not the nine handoff lines");
            var g = m.Groups;
            return HandoffCheck.Good(new HandoffTicket
            {
                Game = g[1].Value, Key = g[2].Value, Player = g[3].Value, From = g[4].Value, To = g[5].Value,
                Id = g[6].Value, Exp = long.Parse(g[7].Value, System.Globalization.CultureInfo.InvariantCulture),
                Carry = g[8].Value == "none" ? null : g[8].Value,
            });
        }

        /// <summary>Every check but `used`, in the posted order. `keys` maps a
        /// key id to its 32-byte public key.</summary>
        public static HandoffCheck Verify(string token, string game, IEnumerable<string> names, IDictionary<string, byte[]> keys, long now, byte[] carry = null)
        {
            var read = Read(token);
            if (!read.Ok) return read;
            var h = read.Value;
            if (h.Game != game) return HandoffCheck.No("wrong_game", $"this handoff is for {h.Game}");
            if (!names.Select(n => n.ToLowerInvariant()).Contains(h.To)) return HandoffCheck.No("wrong_destination", $"this handoff is for {h.To}");
            if (h.Exp <= now - SkewS) return HandoffCheck.No("expired", "this handoff has run out");
            if (h.Exp > now + TtlMaxS + SkewS) return HandoffCheck.No("too_long", $"a handoff lives at most {TtlMaxS} seconds");
            if (!keys.TryGetValue(h.Key, out var pub)) return HandoffCheck.No("unknown_key", $"{game} has no server key {h.Key}");
            var parts = token.Split('.');
            TryB64u(parts[0], out var raw);
            TryB64u(parts[1], out var sig);
            if (!Ed25519.Verify(pub, raw, sig)) return HandoffCheck.No("bad_signature", "the signature is not that key's");
            carry = carry ?? Array.Empty<byte>();
            if (carry.Length > CarryMax) return HandoffCheck.No("carry", $"a handoff carries at most {CarryMax} bytes");
            var got = carry.Length > 0 ? ServerKey.Sha256Hex(carry) : null;
            if (got != h.Carry) return HandoffCheck.No("carry", "the carried bytes are not the ones this handoff names");
            return read;
        }

        internal static string ToB64u(byte[] b) => Convert.ToBase64String(b).TrimEnd('=').Replace('+', '-').Replace('/', '_');

        internal static bool TryB64u(string s, out byte[] bytes)
        {
            bytes = null;
            if (s == null || !B64u.IsMatch(s) || s.Length % 4 == 1) return false;
            var p = s.Replace('-', '+').Replace('_', '/');
            p += new string('=', (4 - p.Length % 4) % 4);
            try { bytes = Convert.FromBase64String(p); return true; } catch (FormatException) { return false; }
        }
    }

    /// <summary>The receiving side: admits each good handoff once. Pin the
    /// game's keys, or give it a Client and it reads them from Elo Pros,
    /// again every 5 minutes so a revoked key stops working, and at once (at
    /// most once a minute) when a handoff names a key it has not seen.</summary>
    public sealed class HandoffDoor
    {
        public const int KeysRefreshS = 300, KeysRetryS = 60, SeenMax = 200_000;

        readonly string _game;
        readonly string[] _names;
        readonly bool _pinned;
        readonly Client _client;
        readonly Func<long> _clock;
        readonly SemaphoreSlim _lock = new SemaphoreSlim(1, 1);
        readonly Dictionary<string, long> _seen = new Dictionary<string, long>();
        Dictionary<string, byte[]> _keys = new Dictionary<string, byte[]>();
        long _readAt = long.MinValue / 2, _triedAt = long.MinValue / 2;

        /// <summary>`names`: every address this server answers to (host:port).
        /// `keys`: {key id: base64 public key}, or null to read them with `client`.</summary>
        public HandoffDoor(string game, IEnumerable<string> names, Client client = null, IDictionary<string, string> keys = null, Func<long> clock = null)
        {
            if (keys == null && client == null) throw new ArgumentException("a door needs the game's keys, or a Client to read them with");
            _game = game;
            _names = names.Select(n => n.ToLowerInvariant()).ToArray();
            _pinned = keys != null;
            _client = client;
            _clock = clock ?? (() => DateTimeOffset.UtcNow.ToUnixTimeSeconds());
            if (keys != null) foreach (var kv in keys) _keys[kv.Key] = Convert.FromBase64String(kv.Value);
        }

        public IReadOnlyDictionary<string, byte[]> Keys => _keys;

        async Task<bool> Refresh(long now)
        {
            _triedAt = now;
            var got = await _client.ServerKeys(_game);
            if (!got.IsAnswer) return false;
            var fresh = new Dictionary<string, byte[]>();
            foreach (var r in got.Value["keys"].Items)
            {
                var id = r.Str("id");
                try
                {
                    var pub = Convert.FromBase64String(r.StrOrEmpty("pub"));
                    if (id != null && pub.Length == 32) fresh[id] = pub;
                }
                catch (FormatException) { }
            }
            _keys = fresh;
            _readAt = now;
            return true;
        }

        public async Task<HandoffCheck> Admit(string token, byte[] carry = null)
        {
            long now = _clock();
            await _lock.WaitAsync();
            try
            {
                if (!_pinned)
                {
                    var read = Handoff.Read(token);
                    bool stale = now - _readAt >= KeysRefreshS;
                    bool unknown = read.Ok && !_keys.ContainsKey(read.Value.Key) && now - _triedAt >= KeysRetryS;
                    if ((stale || unknown) && !await Refresh(now) && _keys.Count == 0)
                        return HandoffCheck.No("no_keys", "could not read the game's server keys; let the player join the usual way");
                }
                var got = Handoff.Verify(token, _game, _names, _keys, now, carry);
                if (!got.Ok) return got;
                if (!First(got.Value.Id, got.Value.Exp + Handoff.SkewS, now))
                    return HandoffCheck.No("used", "this handoff was already admitted");
                return got;
            }
            finally
            {
                _lock.Release();
            }
        }

        bool First(string id, long until, long now)
        {
            if (_seen.Count >= SeenMax)
            {
                foreach (var k in _seen.Where(kv => kv.Value <= now).Select(kv => kv.Key).ToList()) _seen.Remove(k);
                while (_seen.Count >= SeenMax) _seen.Remove(_seen.OrderBy(kv => kv.Value).First().Key);
            }
            if (_seen.TryGetValue(id, out var u) && u > now) return false;
            _seen[id] = until;
            return true;
        }
    }
}

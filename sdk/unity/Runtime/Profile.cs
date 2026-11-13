// A name and a face to draw — Steam's GetPlayerSummaries.
//
// The same shape whether it came from the launcher's `profile` verb, the web
// SDK's Elo.profile, or a plain read of /api/who. `Handle` is checked (a hive
// handle held by one wallet); `DisplayName` is whatever the wallet typed.
// `Label` keeps the two apart, so draw that.

namespace EloPros
{
    public sealed class Profile
    {
        /// <summary>False when we looked and nothing is set for this address —
        /// an anonymous player, which is a normal way to play. A failed read
        /// is never a Profile; it is <see cref="LookKind.CouldNotLook"/>.</summary>
        public bool Found { get; private set; }
        public string Address { get; private set; }
        /// <summary>A hive handle (name@host), held by one wallet.</summary>
        public string Handle { get; private set; }
        /// <summary>True when <see cref="Handle"/> is a held handle.</summary>
        public bool Checked { get; private set; }
        /// <summary>SELF-DECLARED: whatever its owner typed. A claim.</summary>
        public string DisplayName { get; private set; }
        /// <summary>A full https URL to the picture, or null.</summary>
        public string AvatarUrl { get; private set; }
        public string Kind { get; private set; }
        /// <summary>How many people answered to the name looked up. A wallet
        /// names one; a self-declared name can be shared — say "one of 3".</summary>
        public int Matches { get; private set; } = 1;
        public string AchievementsUrl { get; private set; }
        public string ItemsUrl { get; private set; }
        public string TipUrl { get; private set; }
        /// <summary>The reply this was read from.</summary>
        public JsonValue Raw { get; private set; } = JsonValue.Null();

        /// <summary>The one string safe to draw without thinking about it: a
        /// held handle bare, a self-declared name as `~name`, else a short
        /// address. Never show a claimed name in the clothes of a checked one.</summary>
        public string Label
        {
            get
            {
                if (Checked && !string.IsNullOrEmpty(Handle)) return Handle;
                if (!string.IsNullOrEmpty(DisplayName)) return "~" + DisplayName;
                return Short(Address);
            }
        }

        public static string Short(string address)
        {
            if (string.IsNullOrEmpty(address)) return "anonymous";
            return address.Length >= 10 ? address.Substring(0, 6) + "…" + address.Substring(address.Length - 4) : address;
        }

        /// <summary>We looked; nothing is set for this address.</summary>
        public static Profile Nothing(string address) => new Profile { Found = false, Address = address };

        /// <summary>From a flattened profile: the launcher's `profile` reply,
        /// or the web SDK's Elo.profile. Relative paths are joined to `origin`.</summary>
        public static Profile FromFlat(JsonValue flat, string origin)
        {
            if (flat == null || !flat.IsObject) return null;
            var links = flat["links"];
            var m = flat["matches"].AsLong;
            return new Profile
            {
                Found = true,
                Raw = flat,
                Address = flat.Str("address"),
                Handle = NonEmpty(flat.Str("handle")),
                Checked = flat.IsTrue("checked"),
                DisplayName = NonEmpty(flat.Str("display_name")),
                AvatarUrl = Url(origin, flat.Str("avatar")),
                Kind = NonEmpty(flat.Str("kind")),
                Matches = m.HasValue ? (int)m.Value : 1,
                AchievementsUrl = Url(origin, links.Str("achievements")),
                ItemsUrl = Url(origin, links.Str("items")),
                TipUrl = Url(origin, links.Str("tip")),
            };
        }

        /// <summary>From a raw /api/who body — the same flattening the web
        /// SDK and the launcher do.</summary>
        public static Profile FromWho(JsonValue body, string address, string origin)
        {
            if (body == null || !body.IsObject) return null;
            var found = body["found"].IsObject ? body["found"] : JsonValue.Object();
            var wallet = NonEmpty(found.Str("wallet")) ?? NonEmpty(body["subject"].Str("wallet")) ?? address;
            var handle = NonEmpty(found.Str("nip05"));
            var m = body["matches"].AsLong;
            return new Profile
            {
                Found = true,
                Raw = body,
                Address = wallet,
                Handle = handle,
                Checked = handle != null,
                DisplayName = NonEmpty(found.Str("agent")),
                AvatarUrl = Url(origin, found.Str("avatar")),
                Kind = NonEmpty(found.Str("kind")),
                Matches = m.HasValue ? (int)m.Value : 1,
                AchievementsUrl = Url(origin, "achievements/of/" + wallet),
                ItemsUrl = Url(origin, "items/of/" + wallet),
                TipUrl = Url(origin, found["reach"].Str("tip")),
            };
        }

        static string NonEmpty(string s) => string.IsNullOrEmpty(s) ? null : s;

        /// <summary>An absolute URL on the origin's API for a path the origin
        /// handed back (`/face/…`, `/api/face/…` or `face/…`); https URLs pass.</summary>
        public static string Url(string origin, string path)
        {
            if (string.IsNullOrEmpty(path)) return null;
            if (path.StartsWith("https://", System.StringComparison.Ordinal)) return path;
            var p = path.TrimStart('/');
            if (p.StartsWith("api/", System.StringComparison.Ordinal)) p = p.Substring(4);
            return (origin ?? Client.DefaultOrigin).TrimEnd('/') + "/api/" + p;
        }

        public override string ToString() => Found ? Label : $"nothing set for {Short(Address)}";
    }
}

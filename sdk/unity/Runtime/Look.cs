// What a call answered: an answer, a refusal, or "could not look".
//
// The three are different claims and this file never merges them. A reader
// that answers "nothing" when it could not look has invented a fact about a
// player: a ticket read that times out is not "does not own", and an origin
// that is down is not an empty inventory.

using System;

namespace EloPros
{
    public enum LookKind { Answer, Refused, CouldNotLook }

    /// <summary>A refusal: the HTTP status, the server's sentence, and its
    /// whole body. Status 0 means this SDK refused before sending anything,
    /// because the server would have.</summary>
    public sealed class Refusal
    {
        public int Status { get; }
        public string Error { get; }
        public JsonValue Body { get; }

        public Refusal(int status, string error, JsonValue body = null)
        {
            Status = status;
            Error = error ?? "";
            Body = body ?? JsonValue.Null();
        }

        /// <summary>Refused here, before asking.</summary>
        public static Refusal Here(string why) => new Refusal(0, why);

        /// <summary>The field a sign-in check refused on (`domain`, `nonce`,
        /// `stale`, `signature`, …), or null.</summary>
        public string Code => Body.Str("code");

        public override string ToString() => Status == 0 ? Error : $"{Status}: {Error}";
    }

    /// <summary>One call's outcome. Read <see cref="Kind"/> before
    /// <see cref="Value"/>: a could-not-look carries no value, and treating it
    /// as "no" is the bug this type exists to stop.</summary>
    public readonly struct Look<T>
    {
        public LookKind Kind { get; }
        /// <summary>The answer; default unless <see cref="IsAnswer"/>.</summary>
        public T Value { get; }
        /// <summary>The refusal; null unless <see cref="IsRefused"/>.</summary>
        public Refusal Refusal { get; }
        /// <summary>Why nobody could tell; null unless <see cref="IsCouldNotLook"/>.</summary>
        public string Why { get; }

        Look(LookKind kind, T value, Refusal refusal, string why)
        {
            Kind = kind; Value = value; Refusal = refusal; Why = why;
        }

        public static Look<T> Answer(T value) => new Look<T>(LookKind.Answer, value, null, null);
        public static Look<T> Refused(Refusal r) => new Look<T>(LookKind.Refused, default, r, null);
        public static Look<T> CouldNotLook(string why) => new Look<T>(LookKind.CouldNotLook, default, null, why ?? "could not look");

        public bool IsAnswer => Kind == LookKind.Answer;
        public bool IsRefused => Kind == LookKind.Refused;
        public bool IsCouldNotLook => Kind == LookKind.CouldNotLook;

        /// <summary>`answer`, `refused` or `could_not_look` — the same word the
        /// Rust, Python and JavaScript SDKs use.</summary>
        public string KindWord => Kind == LookKind.Answer ? "answer" : Kind == LookKind.Refused ? "refused" : "could_not_look";

        /// <summary>Change the answer, keeping a refusal or could-not-look as it is.</summary>
        public Look<U> Map<U>(Func<T, U> f) =>
            Kind == LookKind.Answer ? Look<U>.Answer(f(Value)) :
            Kind == LookKind.Refused ? Look<U>.Refused(Refusal) : Look<U>.CouldNotLook(Why);

        public Look<U> Then<U>(Func<T, Look<U>> f) =>
            Kind == LookKind.Answer ? f(Value) :
            Kind == LookKind.Refused ? Look<U>.Refused(Refusal) : Look<U>.CouldNotLook(Why);

        public override string ToString() =>
            Kind == LookKind.Answer ? $"answer: {Value}" :
            Kind == LookKind.Refused ? $"refused: {Refusal}" : $"could not look: {Why}";
    }

    public static class Looks
    {
        /// <summary>Classify an HTTP answer exactly as the other three SDKs do:
        /// status 0 or 5xx or a body that is not JSON, or `reachable: false`
        /// on a 2xx, is could-not-look; any other non-2xx is a refusal.</summary>
        public static Look<JsonValue> Classify(int status, string body)
        {
            if (status == 0)
                return Look<JsonValue>.CouldNotLook(string.IsNullOrEmpty(body) ? "the server could not be reached" : body);
            var value = JsonValue.TryParse(body ?? "");
            if (value == null)
                return Look<JsonValue>.CouldNotLook($"the server answered {status} with a body that is not JSON");
            if (status >= 500)
                return Look<JsonValue>.CouldNotLook(Said(value, "why", "error") ?? $"the server answered {status}");
            if (status < 200 || status >= 300)
                return Look<JsonValue>.Refused(new Refusal(status, Said(value, "error", "why", "detail") ?? $"the server answered {status}", value));
            if (value["reachable"].AsBool == false)
                return Look<JsonValue>.CouldNotLook(Said(value, "why", "error") ?? "the server could not look");
            return Look<JsonValue>.Answer(value);
        }

        static string Said(JsonValue v, params string[] keys)
        {
            foreach (var k in keys)
            {
                var s = v[k].AsString;
                if (!string.IsNullOrEmpty(s)) return s;
            }
            return null;
        }
    }
}

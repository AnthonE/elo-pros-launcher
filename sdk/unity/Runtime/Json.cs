// A very small JSON reader and writer, so this package needs no dependency.
//
// Unity's JsonUtility cannot tell a missing field from null, and null is a
// real answer on this platform: `entitled: null` means "could not look",
// never "does not own". This keeps the three apart.

using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace EloPros
{
    public enum JsonKind { Null, Bool, Number, String, Array, Object }

    /// <summary>One JSON value. Reading a missing key or index answers
    /// <see cref="Missing"/> (a null), never throws.</summary>
    public sealed class JsonValue
    {
        public static readonly JsonValue Missing = new JsonValue(JsonKind.Null);

        public JsonKind Kind { get; }
        readonly bool _bool;
        readonly double _num;
        readonly string _str;
        readonly List<JsonValue> _arr;
        readonly List<KeyValuePair<string, JsonValue>> _obj;

        JsonValue(JsonKind kind) { Kind = kind; }
        JsonValue(bool b) { Kind = JsonKind.Bool; _bool = b; }
        JsonValue(double n) { Kind = JsonKind.Number; _num = n; }
        JsonValue(string s) { Kind = JsonKind.String; _str = s; }
        JsonValue(List<JsonValue> a) { Kind = JsonKind.Array; _arr = a; }
        JsonValue(List<KeyValuePair<string, JsonValue>> o) { Kind = JsonKind.Object; _obj = o; }

        public static JsonValue Null() => new JsonValue(JsonKind.Null);
        public static JsonValue Of(bool b) => new JsonValue(b);
        public static JsonValue Of(double n) => new JsonValue(n);
        public static JsonValue Of(long n) => new JsonValue((double)n);
        public static JsonValue Of(string s) => s == null ? Null() : new JsonValue(s);
        public static JsonValue Array(IEnumerable<JsonValue> items) => new JsonValue(new List<JsonValue>(items));
        public static JsonValue Array(IEnumerable<string> items)
        {
            var list = new List<JsonValue>();
            foreach (var s in items) list.Add(Of(s));
            return new JsonValue(list);
        }
        public static JsonValue Object() => new JsonValue(new List<KeyValuePair<string, JsonValue>>());

        /// <summary>Set a key on an object (replacing it), and return the object.</summary>
        public JsonValue Set(string key, JsonValue value)
        {
            if (Kind != JsonKind.Object) throw new InvalidOperationException("not an object");
            for (int i = 0; i < _obj.Count; i++)
            {
                if (_obj[i].Key == key) { _obj[i] = new KeyValuePair<string, JsonValue>(key, value ?? Null()); return this; }
            }
            _obj.Add(new KeyValuePair<string, JsonValue>(key, value ?? Null()));
            return this;
        }
        public JsonValue Set(string key, string value) => Set(key, Of(value));
        public JsonValue Set(string key, long value) => Set(key, Of(value));
        public JsonValue Set(string key, bool value) => Set(key, Of(value));

        public bool IsNull => Kind == JsonKind.Null;
        public bool IsObject => Kind == JsonKind.Object;
        public bool IsArray => Kind == JsonKind.Array;

        /// <summary>True when the object has this key, even if its value is null.</summary>
        public bool Has(string key)
        {
            if (Kind != JsonKind.Object) return false;
            foreach (var kv in _obj) if (kv.Key == key) return true;
            return false;
        }

        public JsonValue this[string key]
        {
            get
            {
                if (Kind != JsonKind.Object) return Missing;
                foreach (var kv in _obj) if (kv.Key == key) return kv.Value;
                return Missing;
            }
        }

        public JsonValue this[int index] =>
            Kind == JsonKind.Array && index >= 0 && index < _arr.Count ? _arr[index] : Missing;

        public int Count => Kind == JsonKind.Array ? _arr.Count : Kind == JsonKind.Object ? _obj.Count : 0;

        public IEnumerable<JsonValue> Items => Kind == JsonKind.Array ? _arr : (IEnumerable<JsonValue>)System.Array.Empty<JsonValue>();

        public IEnumerable<KeyValuePair<string, JsonValue>> Fields =>
            Kind == JsonKind.Object ? _obj : (IEnumerable<KeyValuePair<string, JsonValue>>)System.Array.Empty<KeyValuePair<string, JsonValue>>();

        /// <summary>The string, or null when this is not a string.</summary>
        public string AsString => Kind == JsonKind.String ? _str : null;

        /// <summary>The boolean, or null when this is not one. Null is a third
        /// answer here, never false.</summary>
        public bool? AsBool => Kind == JsonKind.Bool ? _bool : (bool?)null;

        public double? AsDouble => Kind == JsonKind.Number ? _num : (double?)null;

        public long? AsLong
        {
            get
            {
                if (Kind != JsonKind.Number) return null;
                if (_num < long.MinValue || _num > long.MaxValue || Math.Floor(_num) != _num) return null;
                return (long)_num;
            }
        }

        /// <summary>The string at key, or null.</summary>
        public string Str(string key) => this[key].AsString;

        /// <summary>The string at key, or "".</summary>
        public string StrOrEmpty(string key) => this[key].AsString ?? "";

        public bool IsTrue(string key) => this[key].AsBool == true;

        // ── reading ────────────────────────────────────────────────────────

        /// <summary>Parse JSON text. Throws <see cref="FormatException"/> on bad input.</summary>
        public static JsonValue Parse(string text)
        {
            if (text == null) throw new FormatException("no text");
            int i = 0;
            var v = ParseValue(text, ref i, 0);
            SkipWs(text, ref i);
            if (i != text.Length) throw new FormatException($"trailing characters at {i}");
            return v;
        }

        /// <summary>Parse, or null when the text is not JSON.</summary>
        public static JsonValue TryParse(string text)
        {
            try { return Parse(text); } catch (FormatException) { return null; }
        }

        const int MaxDepth = 128;

        static void SkipWs(string s, ref int i)
        {
            while (i < s.Length && (s[i] == ' ' || s[i] == '\t' || s[i] == '\n' || s[i] == '\r')) i++;
        }

        static JsonValue ParseValue(string s, ref int i, int depth)
        {
            if (depth > MaxDepth) throw new FormatException("nested too deep");
            SkipWs(s, ref i);
            if (i >= s.Length) throw new FormatException("unexpected end");
            char c = s[i];
            switch (c)
            {
                case '{': return ParseObject(s, ref i, depth);
                case '[': return ParseArray(s, ref i, depth);
                case '"': return new JsonValue(ParseString(s, ref i));
                case 't': Lit(s, ref i, "true"); return new JsonValue(true);
                case 'f': Lit(s, ref i, "false"); return new JsonValue(false);
                case 'n': Lit(s, ref i, "null"); return Null();
                default:
                    if (c == '-' || (c >= '0' && c <= '9')) return ParseNumber(s, ref i);
                    throw new FormatException($"unexpected '{c}' at {i}");
            }
        }

        static void Lit(string s, ref int i, string word)
        {
            if (string.CompareOrdinal(s, i, word, 0, word.Length) != 0) throw new FormatException($"bad literal at {i}");
            i += word.Length;
        }

        static JsonValue ParseNumber(string s, ref int i)
        {
            int start = i;
            if (s[i] == '-') i++;
            while (i < s.Length && "0123456789+-.eE".IndexOf(s[i]) >= 0) i++;
            var tok = s.Substring(start, i - start);
            if (!double.TryParse(tok, NumberStyles.Float, CultureInfo.InvariantCulture, out var d))
                throw new FormatException($"bad number '{tok}'");
            return new JsonValue(d);
        }

        static string ParseString(string s, ref int i)
        {
            i++; // opening quote
            var sb = new StringBuilder();
            while (true)
            {
                if (i >= s.Length) throw new FormatException("unterminated string");
                char c = s[i++];
                if (c == '"') return sb.ToString();
                if (c < 0x20) throw new FormatException("control character in string");
                if (c != '\\') { sb.Append(c); continue; }
                if (i >= s.Length) throw new FormatException("bad escape");
                char e = s[i++];
                switch (e)
                {
                    case '"': sb.Append('"'); break;
                    case '\\': sb.Append('\\'); break;
                    case '/': sb.Append('/'); break;
                    case 'b': sb.Append('\b'); break;
                    case 'f': sb.Append('\f'); break;
                    case 'n': sb.Append('\n'); break;
                    case 'r': sb.Append('\r'); break;
                    case 't': sb.Append('\t'); break;
                    case 'u':
                        if (i + 4 > s.Length) throw new FormatException("bad \\u escape");
                        if (!int.TryParse(s.Substring(i, 4), NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var code))
                            throw new FormatException("bad \\u escape");
                        sb.Append((char)code); // surrogate pairs arrive as two escapes and stay paired
                        i += 4;
                        break;
                    default: throw new FormatException($"bad escape \\{e}");
                }
            }
        }

        static JsonValue ParseArray(string s, ref int i, int depth)
        {
            i++;
            var list = new List<JsonValue>();
            SkipWs(s, ref i);
            if (i < s.Length && s[i] == ']') { i++; return new JsonValue(list); }
            while (true)
            {
                list.Add(ParseValue(s, ref i, depth + 1));
                SkipWs(s, ref i);
                if (i >= s.Length) throw new FormatException("unterminated array");
                if (s[i] == ',') { i++; continue; }
                if (s[i] == ']') { i++; return new JsonValue(list); }
                throw new FormatException($"expected , or ] at {i}");
            }
        }

        static JsonValue ParseObject(string s, ref int i, int depth)
        {
            i++;
            var obj = new List<KeyValuePair<string, JsonValue>>();
            SkipWs(s, ref i);
            if (i < s.Length && s[i] == '}') { i++; return new JsonValue(obj); }
            while (true)
            {
                SkipWs(s, ref i);
                if (i >= s.Length || s[i] != '"') throw new FormatException($"expected a key at {i}");
                var key = ParseString(s, ref i);
                SkipWs(s, ref i);
                if (i >= s.Length || s[i] != ':') throw new FormatException($"expected : at {i}");
                i++;
                var v = ParseValue(s, ref i, depth + 1);
                // a repeated key: the last one wins, as in every mainstream parser
                int at = obj.FindIndex(kv => kv.Key == key);
                if (at >= 0) obj[at] = new KeyValuePair<string, JsonValue>(key, v);
                else obj.Add(new KeyValuePair<string, JsonValue>(key, v));
                SkipWs(s, ref i);
                if (i >= s.Length) throw new FormatException("unterminated object");
                if (s[i] == ',') { i++; continue; }
                if (s[i] == '}') { i++; return new JsonValue(obj); }
                throw new FormatException($"expected , or }} at {i}");
            }
        }

        // ── writing ────────────────────────────────────────────────────────

        public override string ToString()
        {
            var sb = new StringBuilder();
            Write(sb, false);
            return sb.ToString();
        }

        /// <summary>The bytes a signed detail carries: keys sorted by code
        /// point, no spaces, every non-ASCII character escaped — Python's
        /// json.dumps(sort_keys=True, separators=(",", ":")), which the meter
        /// builds its texts with.</summary>
        public string Canonical()
        {
            var sb = new StringBuilder();
            Write(sb, true);
            return sb.ToString();
        }

        void Write(StringBuilder sb, bool canonical)
        {
            switch (Kind)
            {
                case JsonKind.Null: sb.Append("null"); break;
                case JsonKind.Bool: sb.Append(_bool ? "true" : "false"); break;
                case JsonKind.Number:
                    if (double.IsNaN(_num) || double.IsInfinity(_num)) { sb.Append("null"); break; }
                    if (Math.Floor(_num) == _num && Math.Abs(_num) < 9007199254740992d)
                        sb.Append(((long)_num).ToString(CultureInfo.InvariantCulture));
                    else
                        sb.Append(_num.ToString("R", CultureInfo.InvariantCulture));
                    break;
                case JsonKind.String: Escape(_str, sb, canonical); break;
                case JsonKind.Array:
                    sb.Append('[');
                    for (int i = 0; i < _arr.Count; i++) { if (i > 0) sb.Append(','); _arr[i].Write(sb, canonical); }
                    sb.Append(']');
                    break;
                case JsonKind.Object:
                    sb.Append('{');
                    var fields = _obj;
                    if (canonical)
                    {
                        fields = new List<KeyValuePair<string, JsonValue>>(_obj);
                        fields.Sort((a, b) => string.CompareOrdinal(a.Key, b.Key));
                    }
                    for (int i = 0; i < fields.Count; i++)
                    {
                        if (i > 0) sb.Append(',');
                        Escape(fields[i].Key, sb, canonical);
                        sb.Append(':');
                        fields[i].Value.Write(sb, canonical);
                    }
                    sb.Append('}');
                    break;
            }
        }

        /// <summary>Write a JSON string literal. Every control character is
        /// escaped, so a newline in a signed text survives the wire.</summary>
        public static void Escape(string s, StringBuilder sb, bool asciiOnly = false)
        {
            sb.Append('"');
            foreach (char c in s)
            {
                switch (c)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    case '\b': sb.Append("\\b"); break;
                    case '\f': sb.Append("\\f"); break;
                    default:
                        if (c < 0x20 || c == (char)0x2028 || c == (char)0x2029 || (asciiOnly && c > 0x7f))
                            sb.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
                        else sb.Append(c);
                        break;
                }
            }
            sb.Append('"');
        }
    }
}

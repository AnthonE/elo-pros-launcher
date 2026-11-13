// The C# half of Plugins/WebGL/EloPros.jslib: one call into the page's web
// SDK (elo-3.js), answered by a callback with a JSON reply. Compiled into a
// WebGL player only; the editor and desktop builds use the launcher.

#if UNITY_WEBGL && !UNITY_EDITOR
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;

namespace EloPros
{
    static class WebBridge
    {
        delegate void Reply(int id, IntPtr json);

        [DllImport("__Internal")]
        static extern void EloPros_Call(int id, string op, string argsJson, Reply cb);

        static readonly Dictionary<int, TaskCompletionSource<JsonValue>> Pending = new Dictionary<int, TaskCompletionSource<JsonValue>>();
        static int _next;

        /// <summary>Run one op in the page. The reply is `{ok, value}` or
        /// `{ok: false, code, reason}`.</summary>
        public static Task<JsonValue> Call(string op, JsonValue args)
        {
            var tcs = new TaskCompletionSource<JsonValue>();
            int id = ++_next;
            Pending[id] = tcs;
            try
            {
                EloPros_Call(id, op, (args ?? JsonValue.Object()).ToString(), OnReply);
            }
            catch (Exception e)
            {
                Pending.Remove(id);
                tcs.TrySetResult(JsonValue.Object().Set("ok", false).Set("code", "no_bridge").Set("reason", e.Message));
            }
            return tcs.Task;
        }

        [AOT.MonoPInvokeCallback(typeof(Reply))]
        static void OnReply(int id, IntPtr json)
        {
            if (!Pending.TryGetValue(id, out var tcs)) return;
            Pending.Remove(id);
            var parsed = JsonValue.TryParse(Utf8(json));
            tcs.TrySetResult(parsed ?? JsonValue.Object().Set("ok", false).Set("code", "bad_reply").Set("reason", "the page answered with something that is not JSON"));
        }

        static string Utf8(IntPtr p)
        {
            if (p == IntPtr.Zero) return "";
            int n = 0;
            while (Marshal.ReadByte(p, n) != 0) n++;
            var bytes = new byte[n];
            Marshal.Copy(p, bytes, 0, n);
            return Encoding.UTF8.GetString(bytes);
        }
    }
}
#endif

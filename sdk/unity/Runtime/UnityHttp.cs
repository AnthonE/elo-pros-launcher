#if UNITY_5_3_OR_NEWER
// UnityWebRequest as the HTTP transport, so the same Client runs on every
// platform Unity builds for, WebGL included. Start it on the main thread.

using System.Collections.Generic;
using System.Threading.Tasks;
using UnityEngine.Networking;

namespace EloPros
{
    public sealed class UnityHttp : IHttpTransport
    {
        public Task<HttpReply> Send(HttpRequest r)
        {
            var done = new TaskCompletionSource<HttpReply>();
            UnityWebRequest req;
            try
            {
                req = new UnityWebRequest(r.Url, r.Method) { downloadHandler = new DownloadHandlerBuffer(), timeout = r.TimeoutSeconds };
                if (r.Body != null)
                {
                    req.uploadHandler = new UploadHandlerRaw(r.Body);
                    if (r.ContentType != null) req.SetRequestHeader("Content-Type", r.ContentType);
                }
                foreach (var kv in r.Headers) req.SetRequestHeader(kv.Key, kv.Value);
            }
            catch (System.Exception e)
            {
                return Task.FromResult(new HttpReply(0, $"the request could not be made: {e.Message}"));
            }
            var op = req.SendWebRequest();
            op.completed += _ =>
            {
                try
                {
                    // A connection error has no status; a 4xx/5xx still has a body to classify.
                    if (req.result == UnityWebRequest.Result.ConnectionError || req.responseCode == 0)
                    {
                        done.TrySetResult(new HttpReply(0, $"the server could not be reached: {req.error}"));
                        return;
                    }
                    var headers = new Dictionary<string, string>();
                    var got = req.GetResponseHeaders();
                    if (got != null) foreach (var kv in got) headers[kv.Key.ToLowerInvariant()] = kv.Value;
                    done.TrySetResult(new HttpReply((int)req.responseCode, req.downloadHandler != null ? req.downloadHandler.data : null, headers));
                }
                finally
                {
                    req.Dispose();
                }
            };
            return done.Task;
        }
    }
}
#endif

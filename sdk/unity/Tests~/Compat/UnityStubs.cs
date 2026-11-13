// Stand-ins for the UnityEngine types Runtime/ touches, with Unity's shapes,
// so the Unity-only files compile here. Never shipped: this folder ends in ~.
using System;

namespace UnityEngine
{
    public static class Application
    {
        public static string version => "0";
        public static event Action quitting { add { } remove { } }
        public static void OpenURL(string url) { }
    }

    public class Object { }
    public class Component : Object { }
    public class Behaviour : Component { }
    public class MonoBehaviour : Behaviour { }

    [AttributeUsage(AttributeTargets.Field)]
    public sealed class TooltipAttribute : Attribute { public TooltipAttribute(string tooltip) { } }

    public struct Rect { public Rect(float x, float y, float width, float height) { } }
    public class GUIStyle { }
    public class GUISkin { public GUIStyle box => new GUIStyle(); }
    public static class GUI { public static GUISkin skin => new GUISkin(); }
    public static class GUILayout
    {
        public static void BeginArea(Rect screenRect, GUIStyle style) { }
        public static void EndArea() { }
        public static void Label(string text) { }
        public static bool Button(string text) => false;
    }

    public class AsyncOperation
    {
        public event Action<AsyncOperation> completed { add { } remove { } }
    }
}

namespace UnityEngine.Networking
{
    public class DownloadHandler { public string text => ""; public byte[] data => new byte[0]; }
    public sealed class DownloadHandlerBuffer : DownloadHandler { }
    public class UploadHandler { }
    public sealed class UploadHandlerRaw : UploadHandler { public UploadHandlerRaw(byte[] data) { } }
    public sealed class UnityWebRequestAsyncOperation : AsyncOperation { }

    public sealed class UnityWebRequest : IDisposable
    {
        public enum Result { InProgress, Success, ConnectionError, ProtocolError, DataProcessingError }
        public UnityWebRequest(string url, string method) { }
        public DownloadHandler downloadHandler { get; set; }
        public UploadHandler uploadHandler { get; set; }
        public int timeout { get; set; }
        public long responseCode => 0;
        public string error => null;
        public Result result => Result.Success;
        public void SetRequestHeader(string name, string value) { }
        public System.Collections.Generic.Dictionary<string, string> GetResponseHeaders() => null;
        public UnityWebRequestAsyncOperation SendWebRequest() => new UnityWebRequestAsyncOperation();
        public void Dispose() { }
    }
}

namespace AOT
{
    [AttributeUsage(AttributeTargets.Method)]
    public sealed class MonoPInvokeCallbackAttribute : Attribute { public MonoPInvokeCallbackAttribute(Type type) { } }
}

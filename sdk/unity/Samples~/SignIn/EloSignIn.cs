// Elo Pros in a Unity game, start to finish: start Elo, draw the player's
// name, sign in to your own server, and check their copy. Drop this on any
// GameObject, set `game` to your store slug, and press Play.
//
// In the editor and a desktop build it talks to the Elo launcher on this
// machine; in a WebGL build, to the player's wallet. With neither it plays
// anonymously, which is a normal way to play.

using EloPros;
using UnityEngine;

public class EloSignIn : MonoBehaviour
{
    [Tooltip("Your store slug, e.g. gates.")]
    public string game = "gates";

    [Tooltip("Your game server's host, or host:port. On WebGL leave it empty for the page's host.")]
    public string server = "";

    string status = "starting…";
    string label = "";
    string copy = "";

    async void Start()
    {
        if (!Elo.Init(game))
        {
            status = "playing anonymously — " + Elo.WhyAnonymous;
            return;
        }
        status = $"connected through the {Elo.Mode.ToString().ToLowerInvariant()} (signer: {Elo.Signer})";

        await Elo.Connect();
        var me = await Elo.Profile();
        // Three states: a name, nobody set, or could not look. The last draws
        // nothing about identity, never "anonymous".
        label = me.IsAnswer ? me.Value.Label : "";

        var owns = await Elo.Owns();
        copy = owns.IsAnswer ? (owns.Value ? "you own this game" : "no copy on this wallet")
             : owns.IsRefused ? "" : "could not check your copy right now";
    }

    async void SignInToServer()
    {
        // 1. Ask YOUR server for a nonce: 8+ letters and digits, one per
        //    sign-in. Texts.NewNonce() makes one; this sample stands in for
        //    your server so it runs alone.
        var nonce = Texts.NewNonce();

        // 2. Ask the player's launcher or wallet to sign in to your server.
        var signIn = await Elo.SignIn(string.IsNullOrEmpty(server) ? null : server, nonce);
        if (!signIn.Ok)
        {
            status = signIn.Outcome == SignOutcome.RefusedByPlayer ? "you said no — play on as a guest" : signIn.Reason;
            return;
        }

        // 3. Send signIn.Message and signIn.Signature to your server. It checks
        //    them against the domain and nonce it issued:
        //      var ok = await new Client().CheckSignIn(message, signature, domain, nonce);
        //    and admits ok.Value.Address. Never trust the address the client sends.
        status = "signed in as " + Profile.Short(signIn.Address) + " — now send it to your server";
    }

    void OnGUI()
    {
        GUILayout.BeginArea(new Rect(16, 16, 520, 220), GUI.skin.box);
        GUILayout.Label("Elo Pros — " + status);
        if (label.Length > 0) GUILayout.Label("player: " + label);
        if (copy.Length > 0) GUILayout.Label(copy);
        if (Elo.Mode != EloMode.Anonymous && GUILayout.Button("Sign in to my server")) SignInToServer();
        GUILayout.EndArea();
    }
}

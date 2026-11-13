# Elo Pros for Unity

`com.elopros.sdk` gives a Unity game what Steamworks gives a Steam game: who is
playing, a sign-in your server can check, signatures, a name and a face to
draw, copies, items and leaderboards. The game never holds a key.

| your build | Elo talks to | |
|---|---|---|
| the editor, Windows, macOS, Linux | the Elo launcher on this machine | `sdk/PROTOCOL.md`: a local socket, or a named pipe on Windows |
| WebGL | the player's wallet, in the page | `elo-3.js`, loaded and pinned by hash for you |
| either, with neither there | nobody | the game plays anonymously, which is a normal state |

Unity 2021.3 or newer. No dependencies.

## Install

*Window → Package Manager → + → Add package from git URL…*

```
https://github.com/AnthonE/elo-pros-launcher.git?path=sdk/unity
```

Or in `Packages/manifest.json`:

```json
"com.elopros.sdk": "https://github.com/AnthonE/elo-pros-launcher.git?path=sdk/unity"
```

The package's page in the Package Manager has a **Sign in** sample to import.

## Quick start

```csharp
using EloPros;

async void Start()
{
    if (!Elo.Init("your-game"))              // your store slug
        Debug.Log("playing anonymously: " + Elo.WhyAnonymous);   // normal

    await Elo.Connect();                     // the player's address, or null
    var me = await Elo.Profile();
    if (me.IsAnswer) nameplate.text = me.Value.Label;   // "moss@elopros.com", or "~moss" if self-declared
}
```

Every call is `async`. Await it from the main thread: launcher calls run on a
worker thread, so a consent prompt never freezes a frame, and the answer comes
back on the main thread. Nothing throws for an ordinary outcome.

| Steamworks | Elo | |
|---|---|---|
| `SteamAPI.Init()` | `Elo.Init(game)` | false is normal: play anonymously |
| `SteamUser.GetSteamID()` | `Elo.Connect()` → `Elo.Address` | an address is a claim, not a login |
| `GetAuthSessionTicket` + `BeginAuthSession` | `Elo.SignIn(domain, nonce)` → your server's `CheckSignIn` | a standard SIWE sign-in |
| `GetPersonaName`, avatars | `Elo.Profile(address)` → `.Label`, `.AvatarUrl` | |
| `BIsSubscribedApp` | `Elo.Owns()` (your UI); `Client.CheckRoster` (your server's gate) | |
| `BIsDlcInstalled`, trials | `Elo.Access()` → `.Entitled`, `.Trial`, `.TrialMinutesLeft`, `.Addons` | the roster carries the same per wallet |
| bundles | `Client.BundleBuy(items, wallet)` | one checkout, each item at its posted price |
| crash reporting | `Elo.ReportBug(report, shot, dump, playerChoseDump)`, `Elo.CrashDir` | a dump goes only when the player chose to send it |
| — | `Elo.Sign(text, why)` | the launcher or wallet asks the player |
| `ActivateGameOverlayToWebPage` | `Elo.Open(url)` | |
| overlay | `Elo.Snapshot()` | draw your own HUD from it |
| `ISteamFriends` | `Elo.Friends()`, `Elo.Friend(op, other)` | presence and `elo://join` links; a change is signed by the player |
| `SetPersonaName`, avatars | `Elo.SetFace(change)` | signed by the player |
| `ISteamMatchmaking` | `Elo.QuickJoin(party)`, `Elo.MyLobby()`; `Client.Lobby(id)` on your server | |
| `ISteamUGC` | `Elo.Mods()` | the player's subscribed Workshop mods, with sha256s |
| `ISteamUserStats` | `Elo.Leaderboard()`, `Elo.Achievements()`; your server reports with `Client.StatsReport` | |
| `ISteamRemoteStorage` | `Client.CloudPut` / `CloudGet` / `CloudList` / `CloudDelete` | written by your server |
| `ISteamGameServer` login token | `ServerKey` | Ed25519, public half named on the dev desk |
| — | `ServerKey.Handoff`, `HandoffDoor` | move a player between your servers with no call to us |

## Sign a player in to your server

The address from `Connect` is a claim any modified client can make. When your
server needs to know who is playing:

```csharp
// client: your server issues the nonce (8+ letters and digits, one per sign-in)
var signIn = await Elo.SignIn("play.yourgame.example", nonceFromYourServer);
if (signIn.Ok) SendToMyServer(signIn.Message, signIn.Signature);
else if (signIn.Outcome == SignOutcome.RefusedByPlayer) { /* they said no: play on as a guest */ }
```

```csharp
// your server (a Unity headless build or any .NET process): the same package
var api = new Client();
var ok = await api.CheckSignIn(message, signature, "play.yourgame.example", theNonceYouIssued);
if (ok.IsAnswer) Admit(ok.Value.Address);             // proven
else if (ok.IsRefused) Refuse(ok.Refusal.Code);       // domain, nonce, stale, signature, …
else RetryLater();                                    // could not look — never refuse a player on it
```

Retire the nonce the moment you check it. On WebGL, pass `null` as the domain
to sign in to the page's own host. The message is standard EIP-4361, so a
server in any language can check it with a `siwe` library instead
(`docs/client/GAMEDEV.md` §1).

## Copies

```csharp
var owns = await Elo.Owns();               // this player, this game
// owns.IsAnswer && owns.Value  → a copy
// owns.IsAnswer && !owns.Value → no copy on this wallet
// owns.IsCouldNotLook          → we could not read the chain. Never paywall on it.
```

That is for your own screens. The gate is your server's, on a wallet it
proved: `Client.CheckRoster(slug, wallets)` at join and on an interval. Kick on
a definite `false` only — `null` is a read that failed.

## Three outcomes, never two

Every read answers a `Look<T>`: `IsAnswer`, `IsRefused`, or `IsCouldNotLook`.
"Could not look" is never "no": an outage is not an empty inventory, and a
profile that could not be read is not an anonymous player. A `Profile` with
`Found == false` means we looked and nothing is set.

## Signing

```csharp
var text = Texts.PlayMessage("duel", Elo.Address, "ETH up 5");   // "elo play\naction: duel\n…"
var r = await Elo.Sign(text, "settling round 41");
switch (r.Outcome)
{
    case SignOutcome.Signed:          Send(r.Signature); break;
    case SignOutcome.RefusedByPlayer: break;                      // do not retry
    case SignOutcome.Handoff:         await Elo.Open(r.HandoffUrl); break;   // not an error
    default:                          Show(r.Reason); break;
}
```

The launcher shows your `text` to the player verbatim, with `why` above it, in
its own window. Write both for a person. Never draw your own "allow this?"
dialog: the launcher asks.

## The platform API

`Elo.Api` (or `new Client()`) reads anything public about any wallet:
`Profile`, `Who`, `Face`, `Items`, `Achievements`, `Leaderboard`, `Friends`,
`GuildTags`, `GuildsOf`, `Ticket`, `TicketOf`, `Owns`, `TicketBuy`; at join,
`Lobby`, `LobbyOf`, `QuickJoin`, `PlaytestAccess`, `WorkshopSubscribed`; the
player's signed changes, `Friend` and `SetFace`; and the server's half,
`CheckSignIn`, `CheckPass` (Sign in with Elo Pros), `CheckRoster`,
`ServerKeys`, `ServerCheck`, the `Cloud*` calls and `StatsReport`. Inside
Unity it sends through `UnityWebRequest`, so it works on WebGL; start calls on
the main thread.

## WebGL

The package loads `https://elopros.com/sdk/elo-3.js`, pinned by its hash, the
first time you call `Connect`, `SignIn` or `Sign`. If your WebGL template
already loads it, that copy is used. `Connect` asks the player's wallet
(MetaMask, Phantom, …) to connect; call it from a button.

## Desktop

The launcher sets `ELO_LAUNCHER_SOCKET` on every build it starts. A game
started another way looks in the platform's default place. In the editor, run
the Elo launcher and press Play.

Calls queue on one connection: while a `Sign` waits for the player, other
launcher calls wait behind it. The launcher's socket is not a sandbox; your
players should know what it does (`sdk/PROTOCOL.md`, *What this socket is
not*). A desktop build that reaches it must keep its source public to list
(`docs/client/GAMEDEV.md` §5).

## Your game server

Every Elo Pros game is multiplayer, so the numbers that matter are measured
on your server: stats, achievements and scores are reported by it, never by a
player's machine. The same package runs there (a Unity headless build, or any
.NET process).

```csharp
// once: name key.PublicB64 on the dev desk (act serverkeys) or in elo.json
var key = ServerKey.FromFile("your-game", "eu-1", "/etc/your-game/server.key");
var api = new Client();
await api.ServerCheck(key);                              // does the origin accept it?

// at join: who, and do they still own it
var who = await api.CheckSignIn(message, signature, "play.yourgame.example", nonce);
var roster = await api.CheckRoster("your-game", connectedWallets);   // kick on false only

// while they play
await api.CloudPut(key, wallet, "save", bytes);
var save = await api.CloudGet(key, wallet, "save");      // .Data, .Etag
await api.StatsReport(key, JsonValue.Parse(
    "[{\"wallet\":\"0x…\",\"stats\":{\"kills\":3},\"unlock\":[\"FIRST_BLOOD\"],\"scores\":{\"speedrun\":9123}}]"));

// move a player to another of your servers
var token = key.Handoff(wallet, "zone1.yourgame.example:7000", "zone2.yourgame.example:7000");
// …and on zone2:
var door = new HandoffDoor("your-game", new[] { "zone2.yourgame.example:7000" }, api);
var got = await door.Admit(token);                       // got.Ok, got.Value.Player — or got.Code
```

Make a key with `openssl genpkey -algorithm ed25519 -out server.key`, or
`ServerKey.Generate`. Keep it on the server: a key in a build players
download is a key every player has. The package signs with its own Ed25519
(no dependency), pinned byte for byte to the other SDKs' output.

## Tests

Outside Unity, with the .NET SDK:

```bash
dotnet run --project "sdk/unity/Tests~"
```

It runs the package against `sdk/testdata` and against the real launcher
broker (build it first: `cd launcher-rs && cargo build --example headless -p
elo-broker`). `Tests~/Compat` compiles the Unity-only files as a
`netstandard2.1` library, and `python3 "sdk/unity/Tests~/unity_meta.py"`
checks every file has its `.meta`.

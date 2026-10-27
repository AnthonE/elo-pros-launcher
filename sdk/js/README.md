# @elopros/sdk (JavaScript)

The Elo Pros platform SDK for JavaScript: Node 18+, Deno, Bun and browsers.
No dependencies.

- **The platform API**: friends and buddies, tickets (does this wallet own a
  copy, the calls that buy one, the download grant), items, faces, profiles,
  leaderboards, an agent's check-in, the agent floor, your agents and the
  board at `elopros.com/api/`, plus
  the payout file a game server writes when a wipe closes.
- **A game server's half**: `checkSignIn` (is this sign-in for my domain and
  nonce, fresh, and the wallet's own — smart wallets and Solana included),
  `checkRoster` (who still holds a copy, paged) and `newNonce`.
- **The web SDK** for a browser game: `elo-3.js`, the same file served at
  `https://elopros.com/sdk/elo-3.js`, with TypeScript types for `window.Elo`.
  It connects the player's wallet, Ethereum or Solana, signs them in to your
  server (Sign-In With Ethereum, or its Solana twin), asks whether they own a
  copy, and gives you their name and face. Released web SDK files never
  change; a new version is a new file, and `elo-1.js` and `elo-2.js` stay as
  they were.

**A whole working example**: [`examples/web-game/`](examples/web-game/), a
page and a zero-dependency server. `node examples/web-game/server.mjs`.

The Rust (`sdk/rust`) and Python (`sdk/python`) packages have the same API;
`sdk/README.md` has the table. A desktop game that wants the launcher's
signer uses one of those: the launcher's socket is not reachable from a
browser.

## Install

Until the package is on npm, from a clone of the repo (npm cannot install
one folder of a repo):

```bash
git clone https://github.com/AnthonE/elo-pros-launcher
npm install ./elo-pros-launcher/sdk/js
```

A web page needs no install: the `<script>` tag below.

## Quick start

```js
import { Client, friendLabel } from "@elopros/sdk";

const elo = new Client(); // https://elopros.com

// Does this wallet own a copy of Gates?
const owns = await elo.owns("gates", "0x1111111111111111111111111111111111111111");
if (owns.kind === "answer") console.log(owns.value ? "owns a copy" : "no copy");
else if (owns.kind === "could_not_look") console.log("could not look:", owns.error); // never kick on this
else console.log("refused:", owns.error);

// Friends and buddies, with who is playing where.
const got = await elo.friends("0x1111111111111111111111111111111111111111");
if (got.ok) {
  for (const f of [...got.value.friends, ...got.value.buddies]) {
    if (f.playing?.join) console.log(friendLabel(f), "is on a server:", f.playing.join);
  }
}
```

A signed change: `sign` is any function from the text to a `0x…` signature,
for example a browser wallet's `personal_sign`:

```js
const sign = (text) => window.ethereum.request({ method: "personal_sign", params: [text, me] });
const done = await elo.friend(me, "ask", "0x7777777777777777777777777777777777777777", sign);
```

`friend`, `setFace`, `boardClaim`, `boardSubmit` and `entitle` ask the
server to prepare the change, rebuild the text to sign locally, and throw an
`ActError` with `code: "mismatch"` for a server that offers any other text,
before your signer is asked. `buddy` reads the change back out of the
server's text (`buddyChange`) and throws unless it is the one asked for. Bad input throws
`code: "bad_input"` before anything is sent.

## A game server: the sign-in check and the roster sweep

```js
import { Client, newNonce } from "@elopros/sdk";

const elo = new Client();
const nonce = await newNonce();          // send it to the page; keep it to check against

// … the page calls Elo.signIn({ nonce }) and posts back { message, signature } …
const got = await elo.checkSignIn(message, signature, { domain: "play.your-game.example", nonce });
// retire `nonce` now, whatever the answer: it works once
if (got.kind === "answer") admit(got.value.address);           // proven
else if (got.kind === "refused") refuse(got.body?.code, got.error);
else retryLater(got.error);                                    // never refuse a player on this

// every minute: who still holds a copy (a copy can be sold mid-session)
const roster = await elo.checkRoster("your-slug", wallets);
if (roster.ok) for (const [w, holds] of Object.entries(roster.value.entitled)) if (holds === false) kick(w);
```

## A game server's key

Cloud saves, stats and scores take your server's word, signed with an
Ed25519 key you named on the dev desk or in `elo.json` (`server_keys`).
Server-side only (Node 18+, Deno, Bun), no dependencies:

```js
import { readFileSync } from "node:fs";
import { Client, ServerKey } from "@elopros/sdk";

const key = await ServerKey.fromPem("your-slug", "eu-1", readFileSync("/etc/your-game/server.key", "utf8"));
console.log(key.publicB64);                         // name this with us once
const elo = new Client();
await elo.cloudPut(key, player, "save", bytes);     // a Look: { etag, size, version, updated }
const got = await elo.cloudGet(key, player, "save"); // got.value.data is a Uint8Array
await elo.statsReport(key, [{ wallet: player, stats: { kills: 3 }, scores: { speedrun: 9123 } }]);
```

## The web SDK, in a page

```html
<script src="https://elopros.com/sdk/elo-3.js"
        integrity="sha384-pKOcdX/DZSGO11b/QS2tFdcm6YFqcVCl0xc6IHDJKFBYRJCvY2JBNjbaVA3jIih/"
        crossorigin="anonymous"></script>
<script type="module">
  const wallet = await Elo.connect();                          // code "no_wallet" when there is none
  const { message, signature } = await Elo.signIn({ nonce });  // nonce from your server
  const me = await Elo.profile();                              // me.label, me.avatar; null: could not look
  Elo.on("account", (who) => { /* switched or signed out: sign in again */ });
</script>
```

or from the package: `import "@elopros/sdk/elo-3.js"`, which sets
`window.Elo` (and does nothing during a server render). Types:
`/// <reference types="@elopros/sdk/elo-3" />`.

`examples/web-game/` puts both halves together; `examples/friends.mjs`,
`examples/owns.mjs` and `examples/payout.mjs` show the other parts.

## "Could not look" is never "no"

Every call resolves to a Look:

| kind | means |
|---|---|
| `answer` | the server answered: `look.value` |
| `refused` | the server said no (`look.status`, `look.error`), or this SDK did (`status` 0) before asking |
| `could_not_look` | unreachable, timed out, a 5xx, a body that is not JSON, or the server saying it could not read (`reachable: false`); `look.error` says why |

A ticket read that times out is not "does not own", and an index that is down
is not an empty inventory. Treat `could_not_look` as "ask again later".

## Payout files

```js
import { payoutDoc, payoutFileName, writePayout } from "@elopros/sdk";

const doc = payoutDoc(3, [{ address: "0x2222222222222222222222222222222222222222", amount: 120,
  ticker: "JUNK", for: ["EXTRACTED 120"] }]);           // throws if the meter would refuse it
await writePayout(payoutFileName("island", 3), doc);    // Node: island.payout-3.json, atomically
```

`checkPayout` is the meter's own format check (`meter/gamepay.py`), pinned by
`sdk/testdata/payout.json` and `payout_bad.json`. Amounts are whole coins in
game units, and in JavaScript they must be safe integers.

## Tests

```bash
node --test test/*.test.js
```

They check every fixture in `sdk/testdata/` and run the client against a
local stub server. The fixtures are shared with the other SDKs, Gates and the
launcher, and the meter checks them against its own code.

MIT licensed.

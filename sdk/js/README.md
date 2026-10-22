# @elopros/sdk (JavaScript)

The Elo Pros platform SDK for JavaScript: Node 18+, Deno, Bun and browsers.
No dependencies.

- **The platform API**: friends and buddies, tickets (does this wallet own a
  copy), items, faces, profiles and leaderboards at `elopros.com/api/`, plus
  the payout file a game server writes when a wipe closes.
- **The web SDK** for a browser game: `elo-1.js`, the same file served at
  `https://elopros.com/sdk/elo-1.js`. It connects the player's wallet, signs
  them in to your server (Sign-In With Ethereum) and asks whether they own a
  copy. Released web SDK files never change; a new version is a new file.

The Rust (`sdk/rust`) and Python (`sdk/python`) packages have the same API;
`sdk/README.md` has the table. A desktop game that wants the launcher's
signer uses one of those: the launcher's socket is not reachable from a
browser.

## Install

From the repo folder, until the package is on npm:

```bash
npm install ./elo-pros-launcher/sdk/js
```

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

`friend` and `setFace` ask the server to prepare the change, rebuild the text
to sign locally, and throw an `ActError` with `code: "mismatch"` for a server
that offers any other text, before your signer is asked. Bad input throws
`code: "bad_input"` before anything is sent.

The web SDK, in a page:

```html
<script src="https://elopros.com/sdk/elo-1.js"></script>
```

or from the package: `import "@elopros/sdk/elo-1.js"`, which sets `window.Elo`.

`examples/friends.mjs`, `examples/owns.mjs` and `examples/payout.mjs` show
each part.

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

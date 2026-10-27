# A web game, wired to Elo Pros

The smallest complete game server: a player signs in with their own wallet,
your server checks the sign-in and their copy, and keeps checking while they
play. No dependencies — Node 18 or later.

```bash
node server.mjs            # then open http://localhost:8080
```

| file | what it is |
|---|---|
| `index.html` | the page: connect a wallet, sign in, draw the player's name and face (`elo-3.js`) |
| `server.mjs` | the server: challenges, the sign-in check, the copy check, the roster sweep |

## The flow

1. **The page asks your server for a challenge** — `GET /nonce`. Letters and
   digits, good for five minutes, one use.
2. **The player signs it** in their wallet — `Elo.signIn({ nonce })`. Ethereum
   or Solana, a browser wallet or a phone.
3. **Your server checks it** — `POST /signin`. It retires the nonce first, then
   asks `https://elopros.com/api/signin/check` whether the sign-in is for *your*
   domain and *that* nonce, fresh, and signed by the wallet it names. Smart
   wallets and Solana keys are checked too. Nothing is stored there.
4. **Your server checks the copy** — `owns(slug, address)`. A definite *no*
   is turned away with a link to buy one; a check that could not look lets the
   player in.
5. **Every minute, the whole roster** — `checkRoster(slug, wallets)`. A player
   who sold their copy is signed out. A read that failed signs nobody out.

## Settings

| variable | default | what |
|---|---|---|
| `PORT` | `8080` | where it listens |
| `GAME_DOMAIN` | `localhost:PORT` | the host your players see, e.g. `play.your-game.example`. A sign-in for any other host is refused |
| `ELO_SLUG` | `your-slug` | your game on elopros.com. Until it sells copies, everyone owns it, so this runs before you list |
| `ELO_ORIGIN` | `https://elopros.com` | the platform |
| `SWEEP_MS` | `60000` | how often the roster is checked |

## Taking it live

- Serve it over **https**, on the host in `GAME_DOMAIN`. Wallets show the
  player the domain they are signing in to.
- Keep the game's rules on your server. The session token the page gets back
  is what each request carries (`Authorization: Bearer …`).
- Sessions and challenges live in memory here. With more than one server
  process, keep them in a shared store (Redis, your database).
- Phone wallets need your own WalletConnect project ID, from
  dashboard.reown.com: `Elo.config({ walletconnectProjectId: "…" })` before
  `Elo.connect()`.
- Copying this folder into your own project: `npm install @elopros/sdk` and
  change the import at the top of `server.mjs` to `"@elopros/sdk"`.

## Not on Node?

Step 3 is one HTTPS call, so any server language works:

```bash
curl -s https://elopros.com/api/signin/check -H 'content-type: application/json' -d '{
  "message": "…", "signature": "…", "domain": "play.your-game.example", "nonce": "…"}'
```

`200` is a checked sign-in (`address`, `chain`); `401` is not, with `code`
naming what failed; `503` means the check could not be made — ask again, and
never turn a player away on it. The Rust and Python SDKs wrap the same call
(`check_sign_in`) and the roster sweep (`check_roster`).

To list the game when it is ready, see `sdk/repo/elo.json` and
`https://elopros.com/devs.html`.

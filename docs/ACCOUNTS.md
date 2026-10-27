---
status: live
lane: [platform]
updated: 2026-10-27
about: "sign in with Steam, Discord, Google, GitHub or a wallet; a game on the site joins with a pass instead of a wallet prompt; a locker keeps what an account unlocks and mints it to the wallet it links later; GitHub on the board — built on the platform 2026-10-25, off until the operator switches it on; Gates' half is merged (wire v151), not shipped; the GitHub App and `elo login` are owed"
---

# ACCOUNTS.md — play with a login, link a wallet when you want one

> Operator, 2026-10-24: *"gamers can login with steam, discord, or google. and
> yea the crypto side can come later. if a gamer really wants a skin they will
> just get a wallet and grind money lol, till then we can figure out some
> system to like fake a wallet. save what rewards we can like skins they
> unlock and give them when they link a wallet."* And: GitHub tied into the
> dev side.

## 0 · The shape

| who | signs in with | keeps |
|---|---|---|
| a guest | nothing | nothing past the session |
| a player | Steam, Discord, Google or GitHub | unlocked skins and the free copy, held in a **locker** |
| a player with a wallet | the wallet (an account of its own), or a login linked to one | everything on chain, as today |
| a builder | GitHub + a wallet | board deliveries checked against the PR's author |

**A locker is not a wallet.** It is a record of what an account unlocked. No
key exists for it, and we never generate one for a player (invariant 7). Value
moves only when the account links a wallet; then the house mints what is held.

Decided (operator, 2026-10-24):

- **Skins and the free copy wait; coin does not.** No balance accrues for an
  account; games stay ledgerless (`docs/money/COINS.md`). A wipe's payout row
  for an account pays the wallet it links when the wipe closes, and with none
  is not paid (`gamepay.py`, `no_wallet`). `login.html` says so before play.
- **Guests still play.** Signing in is optional; it is how you keep what you
  unlock.
- **Every sign-in is an account, the wallet too** (operator, 2026-10-25: *"so
  we can ease gamedevs into crypto"*). A game on this site asks for a **pass**
  and never for a wallet signature: one integration, whichever way the player
  came in, and a wallet player and a Discord player look the same to the game
  until it pays coin.

## 1 · What is built, and what is owed

| part | where | state |
|---|---|---|
| shared sign-in plumbing | `meter/oauthflow.py` (drops moved onto it) | built |
| accounts, the token | `meter/accounts.py`, `test_accounts.py` | built, **off** |
| the locker, the wallet link | `meter/locker.py`, `test_locker.py` | built, **off** |
| the mint job | `meter/locker_job.py` (pm2 `elo-locker`) | built, not started |
| the site | `login.html`, `js/account.js`, the masthead (`js/store.js` §signed in with a login), `drive_login.py` | built |
| GitHub on the board | `accounts.pr_author`, `munus.py` (`github` on a delivery, `PAYEE_RULE`) | built |
| a wallet as a sign-in | `/account/siwe/prepare`, `/account/siwe`; the login page's wallet door | built |
| a game's pass | `POST /account/play`, `sdk/testdata/account_token.json` | built |
| friends for an account, the in-game overlay | `meter/friends.py` `/friends/me`, `FRIENDS.md` §1a; `js/game-overlay.js` | built |
| Gates accepts the pass, holds unlocks | AnthonE/Gates#490, §4 | merged, **not shipped** |
| the Elo Pros GitHub App webhook | §5.3 | not built |
| `elo login` in the launcher | §3 | not built |

**Switching it on** (operator): `.venv/bin/python accounts.py --new-key`
(writes `ELO_ACCOUNT_KEY`, default `/data/apps/secrets/elo-account-ed25519.key`,
0600, never over one); `ELO_ACCOUNTS=1` on the meter; an app per provider with
its creds in keys.env and the redirect below; `ELO_LOCKER_SECRET` (32+
characters) in keys.env and on the game box; start `elo-locker`; and on each
shard that takes passes, `elo_account_pubkey` set to what
`/api/account/pubkey` serves. Read `/api/health` `.accounts`: `why_not` names
what is missing.

| provider | protocol | redirect | keys.env | what we keep |
|---|---|---|---|---|
| Steam | OpenID 2.0, checked with `check_authentication` | none to set | none; `ELO_STEAM_WEB_KEY` adds the persona name | steamid64 |
| Discord | OAuth2, scope `identify` | `https://elopros.com/api/account/discord/callback` | `ELO_DISCORD_CLIENT_ID/SECRET` | user id, display name |
| Google | OIDC, scope `openid` only (PKCE, a nonce) — no email | `https://elopros.com/api/account/google/callback` | `ELO_GOOGLE_CLIENT_ID/SECRET` | `sub` |
| GitHub | OAuth app, no scopes | `https://elopros.com/api/account/github/callback` | `ELO_GITHUB_CLIENT_ID/SECRET` | user id, login |

Every provider's token is dropped in the callback. With logins on, the
corner's "sign in" goes to `/login.html` (which carries the wallet door too);
off, it is the wallet's press, as before.

**The copy, at switch-on** (tier 1, a `git pull`): these say a wallet is the
only way in, which is true while logins are off — `index.html` (benefit 01),
`about.html`, `house-rules.html` ("No account"), `docs.html` (sign in). Reword
them the day a provider is on. The API's "no account, no API key"
(`api.html`, `auth.md`, `llms.txt`) stays true: an agent needs no login.

## 2 · Accounts (`meter/accounts.py`)

- **Books.** `$SCRY_VOWS_DIR/accounts/` (or `ELO_ACCOUNTS_DIR`):
  `accounts.json` — `logins`: `provider:id` → `acct_<20 hex>`; `accounts`:
  `acct` → `{created, name, logins[], wallet, wallet_at}` — and `flow.json`,
  the sign-ins under way. One lock over both.
- **Sign-in** is drops' shape (`oauthflow.py`): the page sends the sha256 of a
  sessionStorage secret as `h`; the callback 302s to `/login.html#t=<ticket>`;
  the ticket opens only with that secret. A second login is added while signed
  in (`attach`); a login that already has its own account is refused, and so
  is a second login from the same provider. The last login cannot be removed.
- **The token.** Signed by its own ed25519 key, never the issuer's (an account
  key that is the issuer's switches accounts off), and sent as
  `Authorization: Elo-Account <b64url(text)>.<b64url(signature)>`. The site's
  token (30 days, localStorage `elo.account.token`; the account it names in
  `elo.account.view`) is good on these routes only:

      elo account
      id: acct_…
      for: site
      exp: <unix seconds>

- **A game's pass** (`POST /account/play {game, shard, nonce}`, ten minutes)
  is what a game's server is handed instead of a wallet signature:

      elo account
      id: acct_…
      for: play:<game>
      shard: <the domain that shard signs in as, up to 100 characters>
      nonce: <the nonce that shard's sign-in handed this join>
      wallet: <0x… lowercase, or none>
      exp: <unix seconds>

  It is no good on these routes, so a server holding one cannot link its own
  wallet to the account. It names one shard and one join's nonce, as SIWE does,
  so a server a player was sent to (a `?server=` link) cannot replay it at
  ours. With a wallet linked the game plays them as that wallet; with none, as
  the account. A game checks it offline against `GET /account/pubkey`
  (standard base64 of 32 bytes); the vector is
  `sdk/testdata/account_token.json`.
- **A wallet signs in** with EIP-4361 for this site (`/account/siwe/prepare
  {wallet}`, then `/account/siwe {nonce, signature}`; Solana's form says "your
  Solana account" and `Chain ID: mainnet`). The nonce is ours, kept ten minutes
  and used once, and a wallet checks the domain against the page asking. Its
  first sign-in makes an account whose wallet is that address; signed in, it
  adds the wallet as another way in. The plain link below adds no sign-in.
- **Routes** (bare; `/api/` at the edge): `GET /account` · `GET /account/pubkey`
  · `GET /account/{provider}/start?h=` · `GET /account/{provider}/callback` ·
  `POST /account/redeem {ticket, s}` · `GET /account/me` · `POST /account/attach`
  · `POST /account/detach {provider}` · `POST /account/name {name}` (the
  profile name rule, `faces.claim_error`) · `POST /account/play {game, shard,
  nonce}` · `POST /account/siwe/prepare {wallet}` · `POST /account/siwe
  {nonce, signature}` ·
  `POST /account/wallet/prepare {wallet}` · `POST /account/wallet {wallet, at,
  signature}` · `POST /account/forget` (the account, its logins and its
  locker) · `GET /account/{acct}/wallet` (house only).
- **Linking a wallet**: the wallet signs
  `elo account-link\naccount: acct_…\nwallet: <the signer>\nat: …` through
  `sigcheck` (EOA, smart wallet or Solana). Items land on Robinhood Chain, so
  a Solana signer must be linked to an 0x address first (`walletlink`). The
  first line is an `elo <family>`, so the launcher relay carries it. One wallet
  per account; relinking moves only what is not minted yet.
- **Rate**: start 60 per IP per day, acts 240; nginx and `/mcp` put the
  callbacks and the wallet link on the heavy list.

## 3 · The locker (`meter/locker.py`, `meter/locker_job.py`)

- **Book.** `$SCRY_VOWS_DIR/locker/locker.json`: `acct` →
  `[{game, kind: item|copy, catalog_id, ref, at, state: held|minted, tx, why}]`.
  One of each catalog, and one copy, per account per game.
- **Intake.** `POST /locker/hold {rows: [...]}`, house only:
  `X-Elo-At` and `X-Elo-Mac = hex(hmac_sha256(ELO_LOCKER_SECRET,
  "elo locker-hold\n<at>\n" + body))`, ten minutes of skew. A `coin` row is
  refused. `GET /account/{acct}/wallet` takes the same MAC over
  `"elo account-wallet\n<at>\n" + acct`.
- **Reads.** `GET /locker/me` (site token) — "held for you" on `login.html`;
  `GET /locker/{acct}/items/{game}` — public, as a wallet's items are on
  chain: `{items: [{catalog_id, state}], copy}`, what a game's server reads
  for an account with no wallet so they wear what they unlocked;
  `GET /locker/{acct}/count` — public, names nothing.
- **The job** mints a held skin on `EloItem:<GAME>` with mintRef
  `keccak("<game>:locker:<acct>:<catalog>")` (a double run or a relink is a
  no-op: the ref is used), and the copy with `EloGameTicket:<GAME>`
  `mintFree(wallet)`; a wallet that holds a copy, or has used its free mint,
  closes the row `already held`; while the free mint is shut the row waits.

**The launcher, later:** `elo login` shows a short code; the player finishes
the provider sign-in at `/login.html?code=`; the launcher polls for a
`play:` token — `SIGN-IN.md`'s device-code shape, direction flipped. No key.

## 4 · What Gates changes — merged, not shipped

AnthonE/Gates#490 (`PROTO_VER` 151, a WIRE change: every client updates):

- **The wire.** The auth frame takes a third proof, the pass (tag 2, a 16-bit
  length, the bytes as issued). That frame alone may run to
  `MAX_AUTH_MSG_BYTES`; every other handshake frame keeps its ceiling.
- **The shard** pins `elo_account_pubkey` in `shard.toml` (no call at join)
  and checks the pass (`auth::verify_pass`): elo's signature, exactly seven
  lines, `for: play:gates`, `shard` among the names its SIWE accepts, `nonce`
  this join's, `exp` ahead. A pass naming a wallet is that wallet: the same
  save and copy as signing with it. One naming none plays as `acct_…`.
  Unpinned, a pass is refused and wallets and guests are unchanged.
- **The browser build** (`client-web`, `app.js`) reads `elo.account.token`;
  signed in, PLAY asks `POST /api/account/play {game, shard, nonce}` with the
  shard's nonce and joins with no wallet prompt, falling back to the wallet
  when a shard refuses the pass. A wallet in the browser that is not the
  account's is a choice to play as it.
- **Skins** for an `acct_…` player read `/api/locker/{acct}/items/gates`;
  standings, the gift and THE OUTFITTER treat an account like a wallet.
- **`ops/mint_pending.py`**: an account's sale and its free copy go to
  `POST /api/locker/hold` (MAC'd, `ELO_LOCKER_SECRET` from
  `/home/master/gates-data/locker.env`), logged so they are held once.
- A payout row may name the `acct_…` id: the meter pays its linked wallet
  (`gamepay.py`).

## 5 · GitHub on the developer side

1. **Sign in with GitHub** is the same account; the board's how-to and the
   profile carry a door to `login.html`, shown once logins are on.
2. **Built: the board's payee from GitHub, not prose.** A delivery of a GitHub
   PR, from a wallet an account with a GitHub login links, reads the PR
   (`api.github.com`, `ELO_GITHUB_TOKEN` only if set) and records
   `github: {author_login, merged, author_matches}` beside it, matched by the
   author's id. `PAYEE_RULE` names both doors: the PR's description, or its
   author's linked GitHub. **Paying stays a human act**
   (`merge_is_a_human_act`).
3. **Not built: the Elo Pros GitHub App**, read-only, a webhook at
   `POST /github/webhook` (`ELO_GITHUB_WEBHOOK_SECRET`): a push touching
   `elo.json` re-reads that repo now (`gamerepo`); a merged PR marks the
   delivery; `repo_public` and the license become evidence for "its own source
   public"; later `bug_issues.py` files with the installation token.

## 6 · How we know it works

- `meter/test_accounts.py`: every provider through start → callback → redeem
  with stubbed providers; a wrong `s`, a forged token, a pass on a site route,
  a pass with no nonce, a pass moved to another shard, the SDK vector, Steam's
  forged and misdirected assertions, a second login, the wallet link and its
  replay, a wallet's sign-in (EVM and Solana, a spent nonce, another site's
  text), the house MAC, forgetting, the PR author check.
- `meter/test_locker.py`: the house door, a coin row refused, hold → link →
  one mint each, a relink that cannot re-mint, a copy already held, a shut free
  mint, a payout row for an account.
- `meter/test_drops.py` green across the `oauthflow` move.
- `watchtower/drive_login.py`, in a real browser: the corner's sign-in, Discord
  (stubbed) and back, the account page, a held skin, a rename, a wallet linked
  with a real signature, sign-out, then the wallet door signing in to an
  account of its own and the pass a game gets for it.
- Gates: `server` `auth` pins the platform's vector; `tests/siwe_wire.rs` joins
  with a pass over a real socket and refuses it replayed, for another shard,
  by another key, and on a shard with no key pinned.
- After release: `/api/pubkey` unchanged, `/api/health` `.accounts` reads why
  it is off, then the providers once on.

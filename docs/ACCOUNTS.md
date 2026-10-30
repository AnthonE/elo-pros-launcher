---
status: live
lane: [platform]
updated: 2026-10-29
about: "sign in with Steam, Discord, Google, GitHub or a wallet; a game on the site joins with a pass instead of a wallet prompt; Sign in with Elo Pros gives any game on its own site the same pass (§6, built 2026-10-27); a locker keeps what an account unlocks and mints it to the wallet it links later; GitHub on the board — built on the platform 2026-10-25, off until the operator switches it on; Gates' half is merged (wire v151), not shipped; the GitHub App and `elo login` are owed; a wallet belongs to one account, an account signs in with several, and changing them takes a recent sign-in (2026-10-28); agents and apps sign in with OAuth — an MCP client by the code and PKCE, a CLI by the device grant, an agent with a wallet by its sign-in, an agent on its own with an account of its own (§7, 2026-10-29)"
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
| one wallet, one account; several wallets per account; unlink; a recent sign-in to change them | `accounts.wallet_owner`, `/account/wallet/unlink`, `RECENT_S`; `login.html` | built 2026-10-28 |
| a game's pass | `POST /account/play`, `sdk/testdata/account_token.json` | built |
| friends for an account, the in-game overlay | `meter/friends.py` `/friends/me`, `FRIENDS.md` §1a; `js/game-overlay.js` | built |
| Gates accepts the pass, holds unlocks | AnthonE/Gates#490, §4 | merged, **not shipped** |
| Sign in with Elo Pros, for any game on its own site | `game-signin.html`, `js/game-signin.js`, `sdk/elo-signin-1.js`, `POST /account/play/check`; the SDKs' `checkPass` / `check_pass` | built 2026-10-27; live when accounts are |
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
- **A fresh handle.** A new account's `name` is unset, so it shows as
  `player-…` until its holder picks one (`POST /account/name`). A provider's
  display name stays on its login, which only the holder sees, and is never
  copied to the account name: nobody here has to know your main
  (`docs/start/PITCH.md` §0a).
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
  adds the wallet as another way in. An account signs in with up to ten
  wallets, on either chain, each removed by its address (`/account/detach
  {provider: "wallet", id}`).
- **One wallet, one account** (operator, 2026-10-28: *"two accounts cant have
  the same wallet"*). A wallet belongs to at most one account — as a way in,
  where things are sent, or both (`wallet_owner`). It joins an account only by
  its own fresh signature, and one that belongs to another account is refused
  (409, before it is asked to sign). Signing in with a wallet opens the account
  it belongs to, a link made only to receive included, and makes it a way in
  there: the holder can always reach that account and remove it.
  `python3 accounts.py --dupes` lists any wallet two accounts held from before.
- **A recent sign-in** (`RECENT_S`, half an hour) is needed to add or remove a
  login or a wallet, or to change or unlink where things are sent; an older
  token gets 403 `code: reauth` and the page asks to sign in again. A token
  lifted from a browser cannot send what is held elsewhere or add a way back
  in. Each wallet change is kept on the account (`wallet_log`, the last 30),
  shown on `login.html` and in the export.
- **Telling the holder**, in house. Every change to a wallet or a sign-in, and
  a sign-out everywhere, is kept in `wallet_log` and counts as unseen
  (`alerts` on `/account/me`) until `login.html` shows it (`POST
  /account/notices/seen`, `seen_at`). Until then the masthead's chip carries a
  dot on every page, and once a tab a toast (`js/toast.js`) leads to the
  account page, which marks what was new. No email and no chat app: we keep
  neither address (operator, 2026-10-28: *"lets not worry about discord we
  have enough in house services for this alert"*).
- **A Solana wallet's own sign-in** (`solana:signIn`, Sign In With Solana).
  `/account/siwe/prepare` hands a Solana wallet `sign_in`, the fields of its
  message; a wallet with the feature (Phantom, Solflare) writes the text, checks
  the domain itself and shows a sign-in screen, and the page sends what it
  signed as `signed_message`. The meter reads that text, never compares it byte
  for byte: this site, this wallet, this nonce, our URI, issued within the
  nonce's life (`siws_check`), then the signature over those bytes. A wallet
  without it signs the plain text as before.
- **Phones.** Phone wallets (WalletConnect) are on: `walletconnectProjectId`
  in `watchtower/js/wallet-config.js` holds the reown project's ID (set
  2026-10-28; elopros.com must be under the project's allowed domains), and
  `python3 watchtower/build_meta.py` restamps the pages after a change. With
  no wallet in the browser, a phone's picker also offers the page in Phantom,
  MetaMask or Coinbase Wallet's own browser.
- **Sign out everywhere else** (`POST /account/signout-all`, recent): every
  token signed before now stops working on every route, a game's pass
  included, and this browser gets a new one (`signed_out_at` on the account).
- **Routes** (bare; `/api/` at the edge): `GET /account` · `GET /account/pubkey`
  · `GET /account/{provider}/start?h=` · `GET /account/{provider}/callback` ·
  `POST /account/redeem {ticket, s}` · `GET /account/me` · `POST /account/attach`
  · `POST /account/detach {provider, id?}` · `POST /account/name {name}` (the
  profile name rule, `faces.claim_error`) · `POST /account/play {game, shard,
  nonce}` · `POST /account/siwe/prepare {wallet}` · `POST /account/siwe
  {nonce, signature}` ·
  `POST /account/wallet/prepare {wallet}` · `POST /account/wallet {wallet, at,
  signature}` · `POST /account/wallet/unlink` · `POST /account/notices/seen` · `POST /account/signout-all` · `POST /account/forget {now?}` (the account and everything
  personal under it and its wallets: scheduled 30 days out and lifted by a
  sign-in, or at once with `now`; `meter/mydata.py`) · `GET /account/export`
  (all of it, one JSON file) · `GET /account/{acct}/wallet` (house only).
- **Linking a wallet**: the wallet signs
  `elo account-link\nLink this wallet to your Elo Pros account on <domain>. …
  This signature moves no funds.\naccount: acct_…\nwallet: <the signer>\nat: …`
  through `sigcheck` (EOA, smart wallet or Solana); the wording before
  2026-10-28, without the sentence, is still taken from a plain key
  (`link_text_v1`). Items land on Robinhood Chain, so
  a Solana signer must be linked to an 0x address first (`walletlink`). The
  first line is an `elo <family>`, so the launcher relay carries it. One wallet
  receives per account; relinking moves only what is not minted yet, and
  `POST /account/wallet/unlink` stops sending (what is held waits; `wallet_at`
  stays, so a link signed before cannot come back).
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

## 6 · Sign in with Elo Pros, for any game

> Operator, 2026-10-27: *"build sign in with elo for any game"* — the way to
> ease developers and players in without a wallet prompt.

A game on its own site gets the same pass a game on ours does (§2), without
reading our localStorage, which it cannot. The player signs in to Elo Pros
once, with any login or a wallet, and says yes to each site.

1. **The game's page** loads `https://elopros.com/sdk/elo-signin-1.js` (SRI
   pinned on `devs.html#signin`) and, with a nonce its server issued, calls
   `EloSignIn.popup({game, nonce})` or `EloSignIn.start({game, nonce,
   returnTo})`.
2. **`game-signin.html`** reads `game`, `nonce` and `return` (or `origin`
   for a popup). **The site that asked is the host of that url**: https
   anywhere, plain http only on `localhost` / `127.0.0.1`. That host is the
   pass's `shard`, so a pass a page asks for is good only on that page's own
   site. Signed out, it sends the player to `login.html?back=` and they come
   straight back. Signed in, it names the host, the account, and the wallet
   the game will see if one is linked, and asks.
3. **Continue** asks `POST /account/play {game, shard, nonce}` with the site
   token, then answers `#elo_pass=<pass>` on the return url, or
   `postMessage({type: "elo:pass", …})` to the opener at exactly the origin
   it named. **Cancel** answers `#elo_error=cancelled` / `elo:cancel`.
4. **The game's server** asks `POST /account/play/check {token, game, domain,
   nonce}`: 200 `{player, account, wallet, name, named, game, domain, exp}`,
   401 with `code` `pass` · `game` · `domain` · `nonce` · `gone`, 503 while
   accounts are off. Stateless, like `/signin/check`: the game retires the
   nonce. Or it verifies offline against `GET /account/pubkey`.

What protects the player: the consent card shows the host read from the link,
and a game's name only when its listing's `play_url` is on that same host;
the page cannot be framed (`frame-ancestors 'none'`); a pass is ten minutes,
one game, one host, one nonce, and no good on our own routes. What the game
learns: the account id, its name, and the linked wallet when there is one, the
same three things a pass for Gates carries.

**A popup that loses its opener still answers.** A provider's page sent with
`Cross-Origin-Opener-Policy` cuts a popup off from the game's window on the
way through its sign-in, and from the game's side the popup then reads as
closed. So a popup carries a `return` page on the game's own site (default:
the page that opened it; `callback` in the SDK), checked to be on the same
site as `origin`. With no opener left, `game-signin.html` lands there with
`#elo_pass=…&elo_popup=1&elo_nonce=…`; `elo-signin-1.js` on that page hands the
pass to the game's window over the site's own `BroadcastChannel` (and a
`storage` event, for an older browser) and closes. The game's window keeps
waiting for three minutes after the popup reads closed, rejects `replaced`
when a new `popup()` starts, and `EloSignIn.cancel()` stops it waiting. The
pass still reaches only the site the player saw on the card.

## 7 · Agents and apps: OAuth (`meter/oauth.py`)

> Operator, 2026-10-29, on the scanner asking for OAuth discovery: *"I want
> all of this done… agents are going to do what they do. so we'll just be as
> adaptable as we can."*

An agent or an app gets a token for an account and acts as it on every route a
site token works on (`accounts._signed_in` reads both, as `Bearer` or
`Elo-Account`). Four ways in, one token endpoint:

| who | how | what it gets |
|---|---|---|
| an app or MCP client with a browser | authorization code + PKCE (S256 only); the person says yes on `connect.html` | a grant on the person's account |
| an agent with no browser | the device grant (RFC 8628): the person types its code on `connect.html` | a grant on the person's account |
| an agent with a wallet | its EIP-4361 sign-in, traded at the token endpoint | a site token: the holder signing in |
| an agent on its own | `POST /api/agent/identity {type: anonymous}` | an account of its own, no login; its claim token later asks a person, by the device grant's code |

- **Clients** are public: no secret, PKCE always. They register by RFC 7591 or
  name an https URL to their own metadata document, fetched pinned to a public
  address. Their name is their own words; the card shows where the answer goes
  as the fact. Redirects: https, loopback http (any port), or an app's scheme.
- **Tokens.** Access: an EdDSA JWT (`at+jwt`), an hour, signed by the account
  key (`/.well-known/jwks.json`), naming its grant. Refresh: rotated each use;
  a replaced one presented after a 30-second grace revokes the grant. A grant
  unused 60 days lapses.
- **What a grant cannot do.** Its token is never a recent sign-in, so it
  cannot change wallets or sign-ins, sign out everywhere, or connect another
  app, and it never deletes or exports the account (`accounts._an_app`). Saying yes takes a recent sign-in, because a grant is a way back in
  (§2's rule). An agent's own account is the exception: there it is the holder.
- **Doors out.** `login.html` lists connected apps and disconnects any;
  signing out everywhere disconnects all; `POST /api/oauth/revoke`; deleting
  the account. An agent's own account that nothing refreshes for 60 days is
  swept with the deletions (`mydata.due`).
- **Discovery.** `/.well-known/oauth-authorization-server` (with the auth.md
  profile's `agent_auth` block), `/.well-known/oauth-protected-resource` for
  the site, `/api`, `/mcp` and `/mcp/account`, and every 401 names it in
  `WWW-Authenticate`. `watchtower/auth.md` is the recipe agents read.
- **Signed in, alongside the wallet** (`playauth.verify_signed`, 2026-10-29).
  Any token, a site token or a grant's, stands in for a wallet's signature on
  an off-chain action for a wallet its account has proven (its linked wallet
  or a wallet login, `mydata.wallets_of`); a left-out wallet is the linked
  one; a sent signature is still checked, and the `at`, replay and daily
  guards are unchanged. Faces, friends, guilds (log rows `signed_in`), the
  check-in, cloud saves, drops, lobbies, showcase, the stream kit, the
  workshop, bench, buddy, the chat key, the board, reviews, news comments and
  the download grant. Kept on the signature: predictions and votes (their
  recounts check each row's), the store's acts (invariant 8), a wallet link
  (two keys). **Fresh** (`playauth.FRESH`): a buddy's fund, gift, token or
  retire, exporting or deleting cloud saves, disbanding a guild or passing its lead, and a
  chat key link take a sign-in from the last half hour (`accounts._recent`;
  never a grant's token on a person's account) or the wallet's signature: a
  lifted or lent token can't move value, take a key or undo something.
  **One way in**: with sign-in by login on, every sign-in press goes to
  login.html (`connectAcct`, `EloAccount.signInPage`, `Deck.actorOrConnect`);
  a press for something that needs the wallet itself from someone already
  signed in (a vote, a prediction) asks the wallet. `meter/test_signed_in.py`. **The pages** do the same through
  one helper (`store.js` §the account sign-in, `window.EloAccount`): a signer
  asked for one of those acts answers without the wallet and the post carries
  the token; the Deck pages act as the connected wallet, else the account's
  (`Deck.actor`); news comments take the sign-in for a pass; a login with no
  wallet sees a profile door that says to link one.
  `watchtower/drive_signed_in.py` does it in a browser with no wallet.
- **MCP.** `/mcp/` stays open. `/mcp/account/` is the same tools behind a 401
  that starts the client's OAuth (`oauth.McpDoors`); the tools pass the
  connection's token to the routes they call.
- **Kept**: per grant, on the account, the client id, its name, how and when it
  was granted, when it last refreshed, and the refresh secret's hashes; per
  registered client its name, redirects and home page, dropped after a week
  if nothing connected. `privacy.html` says so; the export lists the apps.

## 8 · How we know it works

- `meter/test_oauth.py`: the discovery documents; registration and the
  redirects refused; authorize with a bad client, a bad redirect, no PKCE and
  a foreign resource; the yes signed out, stale and recent; a wrong verifier
  spending the code; the token on a site route and its limits; a forged
  payload; a code used twice; refresh rotation, its grace and a replay; the
  doors out; the 401 hint; a client metadata document and one naming another
  client; the device grant's pending, slow down, yes, no and expiry; an
  agent's own account, its assertion, its claim and its sweep; the wallet
  grant; the MCP door; an MCP tool forwarding the connection's token; the
  pinned fetch refusing private addresses.
- `watchtower/drive_connect.py`, in a real browser: an app on another origin
  sends a signed-out person through Discord (stubbed) to the card and back
  with a token; cancel; an hour-old sign-in sent to sign in again; an agent's
  typed code; the account page listing and disconnecting.

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
- `meter/test_accounts.py` also holds `/account/play/check`: the player and
  name for a good pass; another game, another site, another nonce, the site
  token, a changed pass and an expired one refused; malformed inputs named.
- `watchtower/drive_game_signin.py`, in a real browser: a game on another
  origin loads `elo-signin-1.js`; signed out, through Discord (stubbed) and
  back to the game with a pass its server accepts; the popup signed in; the
  popup with its opener taken away; the popup signed out through a provider
  page that sends `Cross-Origin-Opener-Policy`, which the game's window reads
  as closed (fails with no grace period); the popup's cancel; a broken link, a
  plain-http return and a popup return on another site refused.
- `watchtower/drive_login.py`, in a real browser: the corner's sign-in, Discord
  (stubbed) and back, the account page, a held skin, a rename, a wallet linked
  with a real signature, sign-out, then the wallet door signing in to an
  account of its own and the pass a game gets for it.
- Gates: `server` `auth` pins the platform's vector; `tests/siwe_wire.rs` joins
  with a pass over a real socket and refuses it replayed, for another shard,
  by another key, and on a shard with no key pinned.
- After release: `/api/pubkey` unchanged, `/api/health` `.accounts` reads why
  it is off, then the providers once on.

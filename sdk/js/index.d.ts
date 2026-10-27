/* @elopros/sdk — types. The shapes are the server's (sdk/testdata/ holds a
   captured example of each); a field may be null wherever the type says so. */

export const VERSION: string;
export const DEFAULT_ORIGIN: string;
export const DEFAULT_TIMEOUT_MS: number;
export const FRIENDS_ACTION: "friends";
export const FACE_ACTION: "profile";
export const BUDDY_ACTION: "buddy";
/** How many wallets one roster sweep sends (the server's own cap). */
export const ROSTER_PAGE: number;
/** A fresh sign-in nonce: `n` (default 16, 8–128) letters and digits from the CSPRNG. */
export function newNonce(n?: number): Promise<string>;

export type BuddyOp = "trial" | "create" | "update" | "face" | "pause" | "resume" | "retire" | "token"
  | "fund" | "gift";
export const BUDDY_OPS: readonly BuddyOp[];

export type FriendsOp = "ask" | "accept" | "decline" | "cancel" | "remove" | "block" | "unblock" | "invite";
export const OPS: Readonly<Record<FriendsOp, "to" | "from" | "wallet">>;

export function opKey(op: string): "to" | "from" | "wallet" | null;
export function isWallet(s: unknown): s is string;
export function dayOf(at: number): string;
export function playMessage(action: string, wallet: string, detail: string, day?: string): string;
export function jsonString(s: string): string;
export function friendsDetail(op: string, other: string, at: number): string | null;
export function friendsSignText(me: string, op: string, other: string, at: number): string | null;
export function faceDetail(at: number, fields?: {
  name?: string | null; pictureSha256?: string | null; kind?: string | null;
  harness?: string | null; model?: string | null;
}): string;
export function readPlay(text: unknown, action: string, wallet: string): { day: string; detail: string } | null;
export function buddyChange(text: unknown, wallet: string):
  { day: string; op: string; args: Record<string, unknown>; at: number } | null;
export function sameArgs(asked: unknown, signed: unknown): boolean;
export function claimText(wallet: string, questId: string, day?: string): string;
export function submitDetail(questId: string, url: string, n: number): Promise<string>;
export function submitText(wallet: string, questId: string, url: string, n: number, day?: string): Promise<string>;
export function entitleText(game: string, wallet: string, day?: string): string;

/* ── outcomes ── */

export interface Answer<T> { kind: "answer"; ok: true; value: T }
export interface Refused { kind: "refused"; ok: false; status: number; error: string; body: unknown }
export interface CouldNotLook { kind: "could_not_look"; ok: false; error: string }
/** "Could not look" is never "no". `status` 0 on a refusal: refused here, before asking. */
export type Look<T> = Answer<T> | Refused | CouldNotLook;

export function classify(status: number, text: string): Look<any>;

export class ActError extends Error {
  code: "bad_input" | "mismatch";
  /** On a mismatch: the text the server offered, and the one this SDK built. */
  offered?: string;
  expected?: string | null;
}

/** Returns, or resolves to, the 0x… signature of `text`. */
export type Signer = (text: string) => string | Promise<string>;

/* ── friends ── */

export interface Playing {
  game: string; title: string | null; server: string | null; server_name: string | null;
  since: number | null;
  /** elo://join/<slug>/<host:port>, which the launcher opens. */
  join: string | null;
  /** The game's rich-presence line ("Wave 5 · 3/4"), when it writes one. */
  status?: string | null;
}
export interface Friend {
  wallet: string;
  /** When the friendship began, or when the buddy was made. */
  since: number | null;
  name: string | null;
  /** A url under /api/, relative to the origin. */
  pic: string | null;
  agent: boolean; online: boolean; playing: Playing | null; last_online: number | null;
  /** Set on a buddy row: which of the owner's agents this is. */
  buddy?: { id: string; kind: "buddy" | "solo" | string };
}
export interface Ask { wallet: string; at: number; name: string | null }
export interface FriendsView {
  wallet: string;
  /** The friends alone; buddies are not counted. */
  count: number;
  friends: Friend[];
  /** The agents this wallet runs, as friend rows plus `buddy`. */
  buddies: Friend[];
  asks_in: Ask[];
  asks_out: Ask[];
  /** When presence could not be read nobody reads online: that is not "everyone is offline". */
  presence: { reachable: boolean; why: string | null };
  limits: { friends_max: number; asks_out_max: number };
}
export interface FriendsActed { ok: boolean; op: string; view: FriendsView }

/* ── tickets ── */

export interface Rail {
  amount_wei?: string | null; amount_units?: string | null; open: boolean; usd_now: number | null;
  where_it_goes?: string;
}
export interface ChainRail {
  rail_id: number; label: string; kind: string; token: string | null; amount: string | null;
  sink: string | null; open: boolean; payment: boolean; decimals: number | null; buyable: boolean;
}
export interface Ticket {
  game: string; ticketed: boolean; contract?: string; reachable?: boolean; lane?: "house" | "community";
  price_usd?: number | null; rails?: Record<string, Rail>; all_rails?: ChainRail[];
  copies_minted?: number | null; copies_sold?: number | null;
  free_mint?: { open: boolean; per_wallet: number | null; sponsored: boolean; how: string };
  how_to_buy?: Record<string, string>; downloads_gated?: boolean;
  /** Set on a free title. */
  what?: string;
  [extra: string]: unknown;
}
export interface TicketOf {
  game: string; ticketed?: boolean; wallet?: string; copies?: number;
  /** null is a failed read, never "does not own". */
  entitled: boolean | null;
  reachable?: boolean; free_left?: number | null; why?: string;
}

/** One roster sweep: kick on `false` only — `null` is a read that failed. */
export interface Roster {
  game: string; ticketed: boolean;
  /** Keyed 0x-lowercased, or by the Solana address as given. */
  entitled: Record<string, boolean | null>;
}

/* ── sign-in ── */

/** A sign-in POST /api/signin/check found fresh, for your domain and nonce, and
    signed by the wallet it names (plain key, smart wallet or Solana). */
export interface SignIn {
  ok: true;
  /** EIP-55 for an Ethereum wallet, base58 for a Solana one (case-sensitive). */
  address: string;
  chain: "evm" | "solana";
  /** "4663" for Robinhood Chain; "mainnet" for a Solana sign-in. */
  chain_id: string;
  domain: string; uri: string; nonce: string; issued_at: string;
  expiration_time: string | null; statement: string | null;
  retire: string; authorises: string;
}
export interface CheckSignInOptions {
  /** Your server's host or host:port — the one the message must name. */
  domain: string;
  /** The nonce you issued for this sign-in. Retire it once this answers. */
  nonce: string;
  /** When set, the URI line must equal it. */
  uri?: string;
  /** When set, the Chain ID line must equal it ("4663", "mainnet"). */
  chainId?: string | number;
  /** How old Issued At may be, in seconds (default 600, at most 86400). */
  maxAgeS?: number;
}

/* ── items ── */

export interface Item {
  token_id?: number; catalog_id: number; mint_kind?: number; mint_kind_name?: string; mint_ref?: string;
  derived?: Record<string, unknown> | null; catalog_name?: string; shape?: string; image?: string;
  /** "vault" for a skin held as its vault's token (no token_id). */
  via?: "vault"; vault?: string; copies?: number; balance?: string;
  [extra: string]: unknown;
}
export interface Items {
  wallet: string; collection?: string; title?: string | null; items: Item[]; count: number;
  balance_onchain?: number | null; index_disagrees?: boolean; truncated?: number | null; cap?: number;
  vault_errors?: string[] | null;
  /** false: no item contract is configured, so nobody holds an item. */
  configured?: boolean;
  [extra: string]: unknown;
}

/* ── faces and profiles ── */

export interface Face {
  wallet: string; name: string | null; picture: string | null; pic: string | null;
  kind: "agent" | "human" | null; harness: string | null; model: string | null; updated: number | null;
  /** The guild tag this wallet shows; null when it shows none. */
  guild: GuildBadge | null;
}

/* ── guilds ── */

/** The guild tag a game draws beside a player's name. */
export interface GuildBadge {
  tag: string; name: string | null; color: string | null; shape: string | null;
  role: "leader" | "officer" | "member" | null; page: string | null;
}
export interface GuildTags { tags: Record<string, GuildBadge | null>; how?: string }
export interface GuildCard {
  tag: string; name: string | null; line: string; emblem: { shape: string; color: string };
  door: "open" | "apply" | "invite"; founded: number; members: number; officers: number;
  games: { slug: string; title: string }[]; page: string;
  role?: "leader" | "officer" | "member"; since?: number; at?: number; by?: string;
}
export interface GuildsOf {
  wallet: string; guilds: GuildCard[]; applied: GuildCard[]; invites: GuildCard[];
  worn: string | null; hidden: boolean; limits: { memberships_max: number };
}
export interface FaceChange {
  name?: string; picturePng?: Uint8Array; clearPicture?: boolean;
  kind?: "agent" | "human"; harness?: string; model?: string;
}
export interface Who {
  handle: string; handle_kind: string; matches: number; authoritative: boolean;
  found: { wallet?: string; agent?: string; nip05?: string | null; profile?: Face | null;
           [extra: string]: unknown } | null;
  [extra: string]: unknown;
}

/* ── leaderboards ── */

export interface Entry {
  rank: number; wallet: string; name: string | null; agent: boolean | null; place: number | null;
  harness: string | null; model: string | null;
  /** The score sits under the board's sort_key. */
  [score: string]: unknown;
}
export interface Board {
  name: string; wipe: number | null; final: boolean; source: string; sort_key: string;
  ranked: number | null; grounded: boolean; refused: string | null; count: number; entries: Entry[];
  [extra: string]: unknown;
}
export interface Leaderboard {
  slug: string; reachable: boolean; published: boolean; boards: Board[]; kind?: string | null;
  why?: string; [extra: string]: unknown;
}

/** The exact text a game server signs for one request (meter/serverkeys.py). */
export function serverText(method: string, path: string, who: string, at: string,
  body?: Uint8Array | string, nonce?: string | null): Promise<string>;
/** A fresh request nonce: 8 to 64 of letters, digits, '-' and '_'. */
export function newServerNonce(n?: number): Promise<string>;

/** One of a game's server keys (Ed25519). Server-side only. */
export class ServerKey {
  readonly game: string;
  readonly keyId: string;
  /** Name this on the dev desk (act serverkeys) or in elo.json (server_keys). */
  readonly publicB64: string;
  readonly who: string;
  static fromSeed(game: string, keyId: string, seed: Uint8Array | string): Promise<ServerKey>;
  static fromPem(game: string, keyId: string, pem: string): Promise<ServerKey>;
  signText(text: string): Promise<string>;
  headers(method: string, path: string, body?: Uint8Array | string,
    options?: { at?: number; nonce?: string | null }): Promise<Record<string, string>>;
}

export interface CloudSave { data: Uint8Array; etag: string | null; version: number | null; }

export class Client {
  constructor(origin?: string, options?: { timeoutMs?: number; fetch?: typeof fetch });
  origin: string;
  timeoutMs: number;
  get(path: string): Promise<Look<any>>;
  post(path: string, body: unknown): Promise<Look<any>>;
  friends(wallet: string): Promise<Look<FriendsView>>;
  /** Throws ActError ("bad_input", "mismatch") and whatever `sign` throws. */
  friend(me: string, op: FriendsOp, other: string, sign: Signer): Promise<Look<FriendsActed>>;
  ticket(slug: string): Promise<Look<Ticket>>;
  ticketOf(slug: string, wallet: string): Promise<Look<TicketOf>>;
  /** Kick a player on a definite false only. */
  owns(slug: string, wallet: string): Promise<Look<boolean>>;
  /** The roster sweep, paged ROSTER_PAGE at a time. Kick on false only. */
  checkRoster(slug: string, wallets: string[]): Promise<Look<Roster>>;
  /** Refused carries the failed field in `body.code` (domain, nonce, stale, signature, …);
      could_not_look is a chain that could not be asked, never a "no". */
  checkSignIn(message: string, signature: string, options: CheckSignInOptions): Promise<Look<SignIn>>;
  items(wallet: string, options?: { title?: string }): Promise<Look<Items>>;
  face(wallet: string): Promise<Look<Face>>;
  setFace(wallet: string, change: FaceChange, sign: Signer): Promise<Look<Face>>;
  guildTags(wallets: string[]): Promise<Look<GuildTags>>;
  guildsOf(wallet: string): Promise<Look<GuildsOf>>;
  who(handle: string): Promise<Look<Who>>;
  leaderboard(slug: string, options?: { kind?: "agent" | "human" }): Promise<Look<Leaderboard>>;
  /** A game server's word, signed by its ServerKey. */
  serverCheck(key: ServerKey): Promise<Look<any>>;
  cloudPut(key: ServerKey, wallet: string, slot: string, data: Uint8Array | string,
    options?: { ifMatch?: string; createOnly?: boolean }): Promise<Look<any>>;
  cloudGet(key: ServerKey, wallet: string, slot: string): Promise<Look<CloudSave>>;
  cloudDelete(key: ServerKey, wallet: string, slot: string, options?: { ifMatch?: string }): Promise<Look<any>>;
  cloudList(key: ServerKey, wallet: string): Promise<Look<any>>;
  statsReport(key: ServerKey, reports: Array<{ wallet: string; stats?: Record<string, number>;
    unlock?: string[]; scores?: Record<string, number>; agent?: boolean }>): Promise<Look<any>>;
  /** Reads a game server makes at join. */
  playtestAccess(slug: string, wallet: string): Promise<Look<boolean>>;
  workshopSubscribed(slug: string, wallet: string): Promise<Look<any>>;
  lobby(id: string): Promise<Look<any>>;
  lobbyOf(wallet: string): Promise<Look<any>>;
  quickJoin(slug: string, party?: number): Promise<Look<any>>;
  /** The server's JSON as it is; no model yet. */
  home(wallet?: string | null, options?: { since?: number }): Promise<Look<any>>;
  floor(): Promise<Look<any>>;
  buddyOffer(): Promise<Look<any>>;
  buddies(wallet: string): Promise<Look<any>>;
  /** Throws ActError ("bad_input", "mismatch") and whatever `sign` throws. */
  buddy(me: string, op: BuddyOp, args: Record<string, unknown>, sign: Signer,
    options?: { picturePng?: Uint8Array }): Promise<Look<any>>;
  board(game?: string): Promise<Look<any>>;
  boardClaim(me: string, questId: string, sign: Signer): Promise<Look<any>>;
  boardSubmit(me: string, questId: string, url: string, sign: Signer, options?: { note?: string }): Promise<Look<any>>;
  ticketBuy(slug: string, options?: { wallet?: string; rail?: string }): Promise<Look<any>>;
  entitle(slug: string, wallet: string, sign: Signer): Promise<Look<{ grant: string; expires_at: number;
    [extra: string]: unknown }>>;
}

export function friendLabel(row: { wallet: string; name?: string | null }): string;

/* ── the payout file ── */

export interface PayoutRow { address: string; amount: number; ticker: string; for?: string[] }
export interface PayoutDoc { wipe: number; payouts: PayoutRow[] }
export function checkPayoutRow(row: unknown): string | null;
export function checkPayout(doc: unknown): string[];
export function payoutDoc(wipe: number, rows: PayoutRow[]): PayoutDoc;
export function payoutFileName(world: string, wipe: number): string;
export function writePayout(path: string, doc: PayoutDoc): Promise<void>;

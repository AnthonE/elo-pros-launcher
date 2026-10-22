/* @elopros/sdk — types. The shapes are the server's (sdk/testdata/ holds a
   captured example of each); a field may be null wherever the type says so. */

export const VERSION: string;
export const DEFAULT_ORIGIN: string;
export const DEFAULT_TIMEOUT_MS: number;
export const FRIENDS_ACTION: "friends";
export const FACE_ACTION: "profile";

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
}
export interface FaceChange {
  name?: string; picturePng?: Uint8Array; clearPicture?: boolean;
  kind?: "agent" | "human"; harness?: string; model?: string;
}
export interface Who {
  handle: string; handle_kind: string; matches: number; authoritative: boolean;
  found: { wallet?: string; agent?: string; vow_id?: string | null; sworn?: boolean; profile?: Face | null;
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
  items(wallet: string, options?: { title?: string }): Promise<Look<Items>>;
  face(wallet: string): Promise<Look<Face>>;
  setFace(wallet: string, change: FaceChange, sign: Signer): Promise<Look<Face>>;
  who(handle: string): Promise<Look<Who>>;
  leaderboard(slug: string, options?: { kind?: "agent" | "human" }): Promise<Look<Leaderboard>>;
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

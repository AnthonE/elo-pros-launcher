/* elo-3.js — types for the web SDK's global, window.Elo.

   Load the script (https://elopros.com/sdk/elo-3.js, or
   import "@elopros/sdk/elo-3.js") and these describe what it sets. In a
   TypeScript project without the import, reference them once:
     /// <reference types="@elopros/sdk/elo-3" /> */

export {};

declare global {
  /** Who is connected. `address` is EIP-55 for an Ethereum wallet, base58 for
      a Solana one (case-sensitive: store it exactly as given). */
  interface EloIdentity {
    address: string;
    chain: "evm" | "solana";
    /** The wallet's own name, e.g. "MetaMask". */
    wallet: string;
  }

  /** What Elo.signIn hands your page. POST it to your server, which checks it
      (POST https://elopros.com/api/signin/check, or viem's verifySiweMessage
      for "evm") with your domain and the nonce you issued. */
  interface EloSignIn {
    /** EIP-4361 for "evm"; the same shape saying "Solana" for "solana". */
    message: string;
    /** 0x… for "evm"; base58 of a 64-byte Ed25519 signature for "solana". */
    signature: string;
    address: string;
    chain: "evm" | "solana";
  }

  interface EloSignInOptions {
    /** From your server: 8+ letters and digits, one per sign-in. */
    nonce: string;
    /** Default: location.host. */
    domain?: string;
    /** Default: location.origin. */
    uri?: string;
    /** One line. Default "Sign in to play."; "" for none. */
    statement?: string;
    expirationTime?: string | number | Date;
  }

  /** A name and a face to draw. `label` is the one string safe to draw: a held
      hive handle bare, a self-declared name as "~name", else a short address. */
  interface EloProfile {
    address: string;
    label: string;
    /** A hive handle (name@host), held by one wallet. */
    handle: string | null;
    /** True when `handle` is a held handle; `display_name` is never checked. */
    checked: boolean;
    /** Whatever the wallet typed. A claim. */
    display_name: string | null;
    /** A full URL, or null when no picture is set. */
    avatar: string | null;
    kind: string | null;
    /** How many people answered to the name looked up. */
    matches: number;
    links: { achievements: string | null; items: string | null; tip: string | null };
  }

  interface EloConfig {
    /** Grey out Solana-only wallets (a server that checks Ethereum sign-ins only). */
    solana?: boolean;
    /** Another origin, e.g. a local meter while testing. */
    origin?: string;
    /** YOUR project ID from dashboard.reown.com: turns phone wallets on. */
    walletconnectProjectId?: string;
    /** Your own copy of the phone-wallet module (https). */
    walletconnectScript?: string;
    /** Its "sha384-…" pin, or "" to load it unpinned. */
    walletconnectIntegrity?: string;
  }

  /** One wallet that announced itself (EIP-6963). */
  interface EloProviderEntry {
    info: { name: string; icon: string; rdns: string; uuid: string };
    provider: { request(args: { method: string; params?: unknown[] }): Promise<unknown> };
  }

  interface EloWeb {
    readonly version: 3;
    ORIGIN: string;
    readonly CHAIN_ID: 4663;
    /** The connected address, or null. */
    address: string | null;
    chain: "evm" | "solana" | null;
    /** Connect and resolve to the address. Rejects with code "no_wallet" when
        the browser has none, and code 4001 when the player says no. */
    connect(choice?: EloProviderEntry | EloProviderEntry["provider"]): Promise<string>;
    identity(): EloIdentity | null;
    signIn(options: EloSignInOptions): Promise<EloSignIn>;
    /** Sign a platform text (a meter route's `sign_this`). */
    sign(text: string): Promise<{ signature: string; address: string; chain: "evm" | "solana" }>;
    /** Put an Ethereum wallet on Robinhood Chain (4663). */
    ensureChain(): Promise<4663>;
    /** true | false, or null for "could not look" — never a paywall on null. */
    owns(slug: string, wallet?: string): Promise<boolean | null>;
    /** null is "could not look" — never draw "anonymous" for it. */
    profile(wallet?: string): Promise<EloProfile | null>;
    on(event: "account", fn: (who: EloIdentity | null) => void): () => void;
    on(event: "chain", fn: (chainId: number) => void): () => void;
    providers(): Promise<EloProviderEntry[]>;
    disconnect(): Promise<void>;
    config(options?: EloConfig): Required<Omit<EloConfig, "origin">> & { origin: string };
  }

  interface Window { Elo: EloWeb }
  // eslint-disable-next-line no-var
  var Elo: EloWeb;
}

"""elo-sdk — the Elo Pros platform SDK for Python.

Two halves:

- **The overlay** (`Overlay`, `play_message`, …): a player's identity and
  signatures from the Elo launcher on the same machine, with no key in the
  game. It is `elo_overlay.py`, the same bytes a game may vendor as one file
  (sdk/SHA256SUMS); the wire is sdk/PROTOCOL.md.
- **The platform API** (`Client`): friends and buddies, tickets (buying a
  copy, the download grant), items, faces, profiles, leaderboards, an
  agent's check-in, the agent floor, your agents and the board at
  https://elopros.com/api/, plus
  `PayoutFile` for the payout file a game server writes when a wipe closes.
- **A game server's half** (`Client.check_sign_in`, `Client.check_roster`,
  `new_nonce`): check the sign-in a player's client hands you — your domain,
  your nonce, fresh, the wallet's own signature, smart wallets and Solana
  included — and sweep who still holds a copy. With a `ServerKey` (your
  server's Ed25519 key, named with us): cloud saves, stats, achievements and
  scores, and the reads a server makes at join (playtest access, a player's
  Workshop mods, a lobby, quick join).

    from elo_sdk import Client

    elo = Client()
    got = elo.owns("gates", "0x1111111111111111111111111111111111111111")
    if got.ok:
        print("owns a copy" if got.value else "no copy")
    elif got.could_not_look:
        print("could not look:", got.error)     # never kick a player on this
    else:
        print("refused:", got.error)

**"Could not look" is never "no".** Every call answers a `Look`: an answer,
a refusal in the server's words, or could-not-look.
"""
from elo_overlay import (DEFAULT_TIMEOUT as OVERLAY_TIMEOUT, FAMILIES, PROTOCOL, SOCKET_ENV, Overlay,
                         Profile, Result, default_socket, play_message)

from . import models, payout, texts
from .client import (DEFAULT_ORIGIN, DEFAULT_TIMEOUT, ROSTER_PAGE, ActError, BadInput, Client, Look, Mismatch,
                     SignRefused, __version__, classify, new_nonce, overlay_signer)
from .payout import PayoutFile, PayoutRow, check_doc, check_row
from .server import ServerKey, new_server_nonce, server_text

__all__ = [
    "OVERLAY_TIMEOUT", "FAMILIES", "PROTOCOL", "SOCKET_ENV", "Overlay", "Profile", "Result", "default_socket",
    "play_message", "models", "payout", "texts", "DEFAULT_ORIGIN", "DEFAULT_TIMEOUT", "ActError", "BadInput",
    "Client", "Look", "Mismatch", "SignRefused", "__version__", "classify", "overlay_signer", "PayoutFile",
    "PayoutRow", "check_doc", "check_row", "ROSTER_PAGE", "new_nonce", "ServerKey", "new_server_nonce",
    "server_text",
]

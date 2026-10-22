"""elo-sdk — the Elo Pros platform SDK for Python.

Two halves:

- **The overlay** (`Overlay`, `play_message`, …): a player's identity and
  signatures from the Elo launcher on the same machine, with no key in the
  game. It is `elo_overlay.py`, the same bytes a game may vendor as one file
  (sdk/SHA256SUMS); the wire is sdk/PROTOCOL.md.
- **The platform API** (`Client`): friends and buddies, tickets, items,
  faces, profiles and leaderboards at https://elopros.com/api/, plus
  `PayoutFile` for the payout file a game server writes when a wipe closes.

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
from .client import (DEFAULT_ORIGIN, DEFAULT_TIMEOUT, ActError, BadInput, Client, Look, Mismatch,
                     SignRefused, __version__, classify, overlay_signer)
from .payout import PayoutFile, PayoutRow, check_doc, check_row

__all__ = [
    "OVERLAY_TIMEOUT", "FAMILIES", "PROTOCOL", "SOCKET_ENV", "Overlay", "Profile", "Result", "default_socket",
    "play_message", "models", "payout", "texts", "DEFAULT_ORIGIN", "DEFAULT_TIMEOUT", "ActError", "BadInput",
    "Client", "Look", "Mismatch", "SignRefused", "__version__", "classify", "overlay_signer", "PayoutFile",
    "PayoutRow", "check_doc", "check_row",
]

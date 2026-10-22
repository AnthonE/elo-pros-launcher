"""Where the client keeps an agent's key and config, and how it reads its knobs.

`$ELO_PROS_HOME`, else `~/.elo-pros`. A key made by the client under its old
name lives in `~/.scry` (or `$SCRY_HOME`), and it is used where it is, never
copied or moved: the key is the agent's wallet, and a client update must not
hand an agent a new one.
"""
from __future__ import annotations

import os
from pathlib import Path


def home() -> Path:
    named = os.getenv("ELO_PROS_HOME") or os.getenv("SCRY_HOME")
    if named:
        return Path(named)
    new, old = Path.home() / ".elo-pros", Path.home() / ".scry"
    return old if old.exists() and not new.exists() else new


def env(name: str, default: str | None = None) -> str | None:
    """`ELO_PROS_<name>`, else the old client's `SCRY_<name>`, else `default`."""
    return os.getenv(f"ELO_PROS_{name}") or os.getenv(f"SCRY_{name}") or default

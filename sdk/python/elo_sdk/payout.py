"""A wipe's payout file, as a game server writes it.

When a wipe closes, the game writes `<world>.payout-<wipe>.json` and its
status responder serves it at `/payout/<wipe>.json` beside `status.json`. The
meter reads it off the shards the operator names and pays each row once
through the cashier (meter/gamepay.py):

    {"wipe": 3, "payouts": [{"address": "0x…", "amount": 120, "ticker": "JUNK",
                             "for": ["EXTRACTED 120"]}]}

Amounts are whole coins in game units; the meter applies the posted ratio.
`check_row` and `check_doc` are gamepay's own format checks, pinned by
sdk/testdata/payout.json and payout_bad.json. Which coins a cashier pays at
all is its policy, not the format, and is not checked here.
"""
from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass, field
from typing import List, Optional

_ADDR = re.compile(r"^0x[0-9a-fA-F]{40}$")
_TICKER = re.compile(r"^[A-Z0-9]{1,12}$")


def _whole(v) -> bool:
    return isinstance(v, int) and not isinstance(v, bool) and v >= 1


def check_row(row) -> Optional[str]:
    """Why gamepay would not owe this row, or None. The words are `gamepay._check`'s."""
    r = row if isinstance(row, dict) else {}
    addr = r.get("address")
    if not (isinstance(addr, str) and _ADDR.match(addr)):
        return "not a wallet"
    amount = r.get("amount")
    if not _whole(amount):
        return f"amount {amount!r}"
    ticker = str(r.get("ticker") or "").upper()
    if not _TICKER.match(ticker):
        return f"ticker {ticker!r}"
    return None


def check_doc(doc) -> List[str]:
    """Everything gamepay would refuse in a whole file: the file's own shape
    first (`wipe` a whole number from 1, `payouts` a list), then each row as
    `row N: why`, counted from 1. Empty when every row would be owed."""
    wipe = doc.get("wipe") if isinstance(doc, dict) else None
    if not _whole(wipe):
        return [f"the file's wipe is {wipe!r}, a whole number from 1"]
    rows = doc.get("payouts")
    if not isinstance(rows, list):
        return ["no payouts array"]
    return [f"row {i}: {why}" for i, why in enumerate((check_row(r) for r in rows), 1) if why]


@dataclass
class PayoutRow:
    """One wallet's pay for the wipe: whole coins, at least 1, in game units."""
    address: str
    amount: int
    ticker: str
    reasons: List[str] = field(default_factory=list)   # `for` on the wire

    def because(self, line: str) -> "PayoutRow":
        """Add a line saying what this was for (`EXTRACTED 120`)."""
        self.reasons.append(line)
        return self

    def to_json(self) -> dict:
        d = {"address": self.address, "amount": self.amount, "ticker": self.ticker}
        if self.reasons:
            d["for"] = list(self.reasons)
        return d


@dataclass
class PayoutFile:
    """A closed wipe's payout file."""
    wipe: int
    payouts: List[PayoutRow] = field(default_factory=list)

    @classmethod
    def from_json(cls, doc: dict) -> "PayoutFile":
        """Read a file that `check_doc` passes; raises ValueError on one it refuses."""
        bad = check_doc(doc)
        if bad:
            raise ValueError("; ".join(bad))
        return cls(doc["wipe"], [PayoutRow(r["address"], r["amount"], r["ticker"],
                                           [str(x) for x in r.get("for") or []]) for r in doc["payouts"]])

    def pay(self, row: PayoutRow) -> "PayoutFile":
        self.payouts.append(row)
        return self

    def to_dict(self) -> dict:
        return {"wipe": self.wipe, "payouts": [r.to_json() for r in self.payouts]}

    def validate(self) -> List[str]:
        """What gamepay would refuse; empty when the whole file would be owed."""
        return check_doc(self.to_dict())

    def to_json(self) -> str:
        """The file as the game writes it."""
        return json.dumps(self.to_dict(), indent=2) + "\n"

    def file_name(self, world: str) -> str:
        """`<world>.payout-<wipe>.json`, the name the game writes it under."""
        return f"{world}.payout-{self.wipe}.json"

    def write_atomic(self, path) -> None:
        """Validate, then write to `path` atomically: a temp file beside it,
        flushed, then renamed over it, so the status responder never serves
        half a file. A file gamepay would refuse is not written (ValueError)."""
        bad = self.validate()
        if bad:
            raise ValueError("; ".join(bad))
        path = os.fspath(path)
        tmp = os.path.join(os.path.dirname(path) or ".", f".{os.path.basename(path)}.tmp-{os.getpid()}")
        try:
            with open(tmp, "w", encoding="utf-8") as f:
                f.write(self.to_json())
                f.flush()
                os.fsync(f.fileno())
            os.replace(tmp, path)
        except BaseException:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            raise


__all__ = ["check_row", "check_doc", "PayoutRow", "PayoutFile"]

"""Moving a player between a game's servers: the handoff a server signs and the
door the next one admits it at, against the shared fixture byte for byte."""
import base64
import unittest

from _testdata import fx

from elo_sdk import HandoffDoor, HandoffRefused, Look, ServerKey, handoff_text, read_handoff, verify_handoff
from elo_sdk.handoff import KEYS_REFRESH_S, KEYS_RETRY_S, SKEW_S

FX = fx("handoff.json")
KEY = ServerKey(FX["game"], FX["key_id"], bytes.fromhex(FX["seed_hex"]))
KEYS = {k: base64.b64decode(v) for k, v in FX["keys"].items()}


def carry_of(row) -> bytes:
    return (row.get("carry_utf8") or "").encode()


class Clock:
    def __init__(self, t):
        self.t = t

    def __call__(self):
        return self.t


class Fixture(unittest.TestCase):
    def test_text_and_token(self):
        self.assertEqual(KEY.public_b64, FX["public_b64"])
        for c in FX["cases"]:
            carry = carry_of(c) or None
            t = handoff_text(FX["game"], FX["key_id"], c["player"], FX["from"], FX["to"], c["id"], FX["exp"], carry)
            self.assertEqual(t, c["text"])
            tok = KEY.handoff(c["player"], FX["from"], FX["to"], carry=carry, ttl=FX["exp"] - FX["now"],
                              now=FX["now"], hid=c["id"])
            self.assertEqual(tok, c["token"])
            self.assertEqual(read_handoff(tok).player, c["player"])

    def test_every_case_admits_once_then_every_refusal(self):
        door = HandoffDoor(FX["game"], [FX["to"]], keys=FX["keys"], clock=Clock(FX["now"]))
        for c in FX["cases"]:
            self.assertEqual(door.admit(c["token"], carry_of(c)).player, c["player"])
        for r in FX["refused"]:
            with self.assertRaises(HandoffRefused) as e:
                door.admit(r["token"], carry_of(r))
            self.assertEqual(e.exception.code, r["code"], r)

    def test_a_refused_handoff_is_not_used_up(self):
        door = HandoffDoor(FX["game"], [FX["to"]], keys=FX["keys"], clock=Clock(FX["now"]))
        c = FX["cases"][2]
        with self.assertRaises(HandoffRefused):
            door.admit(c["token"], b"tampered")
        self.assertEqual(door.admit(c["token"], carry_of(c)).id, c["id"])

    def test_the_window(self):
        tok = FX["cases"][0]["token"]
        kw = dict(game=FX["game"], names=[FX["to"]], keys=KEYS)
        verify_handoff(tok, now=FX["exp"] + SKEW_S - 1, **kw)
        with self.assertRaises(HandoffRefused) as e:
            verify_handoff(tok, now=FX["exp"] + SKEW_S, **kw)
        self.assertEqual(e.exception.code, "expired")

    def test_issuing_refuses_what_no_door_would_take(self):
        for bad in (dict(player="0xABC"), dict(frm="no-port"), dict(ttl=121), dict(ttl=0),
                    dict(carry=b"x" * 4097), dict(hid="short")):
            args = dict(player=FX["cases"][0]["player"], frm=FX["from"], to=FX["to"])
            extra = {k: v for k, v in bad.items() if k not in args}
            args.update({k: v for k, v in bad.items() if k in args})
            with self.assertRaises(ValueError, msg=bad):
                KEY.handoff(args["player"], args["frm"], args["to"], **extra)

    def test_a_fresh_handoff_admits_now(self):
        tok = KEY.handoff(FX["cases"][1]["player"], FX["from"], FX["to"])
        door = HandoffDoor(FX["game"], [FX["to"]], keys=FX["keys"])
        self.assertTrue(door.admit(tok).player.startswith("acct_"))


class Keys(unittest.TestCase):
    """A door reading the game's keys from Elo Pros."""

    class FakeClient:
        def __init__(self, rows):
            self.rows, self.calls = rows, 0

        def server_keys(self, game):
            self.calls += 1
            if self.rows is None:
                return Look.unknown("unreachable")
            return Look.answer({"game": game, "keys": self.rows})

    def rows(self, *kids):
        return [{"id": k, "pub": KEY.public_b64} for k in kids]

    def test_reads_once_then_caches(self):
        cl = self.FakeClient(self.rows(FX["key_id"]))
        clock = Clock(FX["now"])
        door = HandoffDoor(FX["game"], [FX["to"]], client=cl, clock=clock)
        door.admit(FX["cases"][0]["token"])
        door.admit(FX["cases"][1]["token"])
        self.assertEqual(cl.calls, 1)

    def test_a_rotated_key_works_on_first_use(self):
        cl = self.FakeClient(self.rows("old-1"))
        clock = Clock(FX["now"])
        door = HandoffDoor(FX["game"], [FX["to"]], client=cl, clock=clock)
        with self.assertRaises(HandoffRefused) as e:
            door.admit(FX["cases"][0]["token"])
        self.assertEqual(e.exception.code, "unknown_key")
        cl.rows = self.rows(FX["key_id"])
        clock.t += KEYS_RETRY_S
        self.assertEqual(door.admit(FX["cases"][0]["token"]).player, FX["cases"][0]["player"])

    def test_an_unknown_key_does_not_hammer_the_origin(self):
        cl = self.FakeClient(self.rows("old-1"))
        door = HandoffDoor(FX["game"], [FX["to"]], client=cl, clock=Clock(FX["now"]))
        for _ in range(5):
            with self.assertRaises(HandoffRefused):
                door.admit(FX["cases"][0]["token"])
        self.assertEqual(cl.calls, 1)

    def test_a_revoked_key_stops_after_the_refresh(self):
        fresh = lambda t: KEY.handoff(FX["cases"][0]["player"], FX["from"], FX["to"], now=int(t))  # noqa: E731
        cl = self.FakeClient(self.rows(FX["key_id"]))
        clock = Clock(FX["now"])
        door = HandoffDoor(FX["game"], [FX["to"]], client=cl, clock=clock)
        door.admit(fresh(clock.t))
        cl.rows = []                                    # the developer revoked it
        clock.t += KEYS_REFRESH_S - 1
        door.admit(fresh(clock.t))                      # still cached
        clock.t += 1
        with self.assertRaises(HandoffRefused) as e:
            door.admit(fresh(clock.t))
        self.assertEqual(e.exception.code, "unknown_key")

    def test_could_not_look_is_its_own_answer(self):
        door = HandoffDoor(FX["game"], [FX["to"]], client=self.FakeClient(None), clock=Clock(FX["now"]))
        with self.assertRaises(HandoffRefused) as e:
            door.admit(FX["cases"][0]["token"])
        self.assertEqual(e.exception.code, "no_keys")

    def test_a_door_needs_keys_or_a_client(self):
        with self.assertRaises(ValueError):
            HandoffDoor(FX["game"], [FX["to"]])


if __name__ == "__main__":
    unittest.main()

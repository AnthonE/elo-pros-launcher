"""Every fixture parses, and could-not-look stays apart from an answer."""
import json
import unittest

from _testdata import fx

from elo_sdk import classify
from elo_sdk.models import FriendsView, Items, Leaderboard, Ticket, TicketOf, Who


def answer(body, parse):
    got = classify(200, json.dumps(body), parse)
    assert got.ok, got
    return got.value


class Models(unittest.TestCase):
    def test_friends_view_with_buddies(self):
        v = answer(fx("friends_view.json"), FriendsView.from_json)
        self.assertEqual((v.count, len(v.friends), len(v.buddies)), (3, 3, 1))
        self.assertTrue(v.presence.reachable)
        self.assertEqual(v.limits.friends_max, 250)
        self.assertEqual(v.friends[0].label, "ash")
        self.assertEqual(v.friends[0].playing.join, "elo://join/gates/game.elopros.com:61234")
        self.assertIsNone(v.friends[1].playing.join)
        self.assertEqual(v.friends[2].label, "0x4444…4444")
        self.assertFalse(any(f.is_buddy for f in v.friends))
        pip = v.buddies[0]
        self.assertTrue(pip.is_buddy and pip.agent)
        self.assertIsNone(pip.since)
        self.assertEqual((pip.buddy.id, pip.buddy.kind), ("pip", "buddy"))
        self.assertEqual(v.asks_in[0].name, "rook")

    def test_ticket_and_ownership(self):
        t = fx("ticket.json")
        card = answer(t["ticketed"], Ticket.from_json)
        self.assertTrue(card.ticketed and card.rails["eth"].open and not card.rails["elo"].open)
        self.assertEqual(card.price_usd, 9.99)
        self.assertTrue(card.all_rails[0].buyable)
        self.assertEqual(card.free_mint.per_wallet, 1)
        self.assertFalse(answer(t["free"], Ticket.from_json).ticketed)
        o = fx("ticket_of.json")
        self.assertIs(answer(o["owned"], TicketOf.from_json).entitled, True)
        down = classify(o["could_not_look"]["status"], json.dumps(o["could_not_look"]["body"]), TicketOf.from_json)
        self.assertTrue(down.could_not_look)
        self.assertIn("could not look", down.error)

    def test_items(self):
        i = fx("items_of.json")
        held = answer(i["held"], Items.from_json)
        self.assertEqual((held.count, len(held.items)), (3, 3))
        self.assertEqual(held.items[0].token_id, 41)
        self.assertIn("float", held.items[1].derived)
        self.assertTrue(held.items[2].is_vault and held.items[2].token_id is None)
        self.assertTrue(classify(200, json.dumps(i["could_not_look"]), Items.from_json).could_not_look,
                        "an index that timed out is not an empty wallet")
        dark = answer(i["not_configured"], Items.from_json)
        self.assertEqual((dark.configured, dark.items), (False, []))

    def test_leaderboard(self):
        b = fx("leaderboard.json")
        lb = answer(b["published"], Leaderboard.from_json)
        island = lb.boards[0]
        self.assertEqual(island.sort_key, "island")
        self.assertEqual(island.score_of(island.entries[0]), 1200.0)
        self.assertEqual(island.entries[1].harness, "claude-code/2.1")
        self.assertTrue(lb.boards[2].is_final)
        self.assertEqual(answer(b["agents"], Leaderboard.from_json).kind, "agent")
        self.assertTrue(classify(200, json.dumps(b["unpublished"]), Leaderboard.from_json).could_not_look)

    def test_who(self):
        who = answer(fx("who_fixture.json")["who"], Who.from_json)
        self.assertTrue(who.authoritative)
        self.assertEqual((who.found.agent, who.found.vow_id), ("Dummmy", "1a434221c5333fa7"))
        self.assertIn("conduct", who.found.raw)

    def test_the_three_outcomes(self):
        self.assertTrue(classify(404, '{"error":"no ask from that wallet"}').refused)
        self.assertTrue(classify(502, "<html>bad gateway</html>").could_not_look)
        self.assertTrue(classify(200, '{"reachable": false, "error": "timeout"}').could_not_look)
        self.assertTrue(classify(200, "[1,2]", FriendsView.from_json).could_not_look)


if __name__ == "__main__":
    unittest.main()

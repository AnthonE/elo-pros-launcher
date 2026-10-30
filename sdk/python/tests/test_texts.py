"""The signed texts rebuild byte for byte from sdk/testdata/."""
import unittest

from _testdata import fx

from elo_sdk import texts


class FriendsTexts(unittest.TestCase):
    def test_ops_are_the_servers(self):
        self.assertEqual(fx("friends_texts.json")["ops"], texts.OPS)

    def test_every_case_rebuilds(self):
        cases = fx("friends_texts.json")["cases"]
        self.assertEqual({c["op"] for c in cases}, set(texts.OPS))
        for c in cases:
            with self.subTest(op=c["op"]):
                self.assertEqual(texts.day_of(c["at"]), c["day"])
                self.assertEqual(texts.friends_detail(c["op"], c["other"], c["at"]), c["detail"])
                self.assertEqual(texts.friends_sign_text(c["wallet"], c["op"], c["other"], c["at"]), c["text"])
                upper = "0x" + c["wallet"][2:].upper()
                self.assertEqual(texts.friends_sign_text(upper, c["op"], c["other"].upper().replace("0X", "0x"),
                                                         c["at"]), c["text"], "a checksummed wallet signs lowercase")

    def test_nothing_is_built_for_a_bad_op_or_wallet(self):
        w = "0x1111111111111111111111111111111111111111"
        self.assertIsNone(texts.friends_detail("poke", w, 1))
        self.assertIsNone(texts.friends_detail("ask", "0x123", 1))
        self.assertIsNone(texts.friends_sign_text("me", "ask", w, 1))

    def test_day_turns_at_utc_midnight(self):
        self.assertEqual(texts.day_of(1761004799), "2025-10-20")
        self.assertEqual(texts.day_of(1761004800), "2025-10-21")


class FaceTexts(unittest.TestCase):
    def test_every_case_rebuilds(self):
        for c in fx("face.json")["cases"]:
            f = c["fields"]
            with self.subTest(at=c["at"]):
                detail = texts.face_detail(c["at"], f.get("name"), f.get("picture_sha256"), f.get("kind"),
                                           f.get("harness"), f.get("model"), f.get("avatar"))
                self.assertEqual(detail, c["detail"])
                self.assertEqual(texts.play_message(texts.FACE_ACTION, c["wallet"], detail, c["day"]), c["text"])


class BuddyTexts(unittest.TestCase):
    def test_ops_are_the_servers(self):
        self.assertEqual(list(texts.BUDDY_OPS), fx("buddy_texts.json")["ops"])

    def test_every_case_reads_back_as_the_change_asked_for(self):
        for c in fx("buddy_texts.json")["cases"]:
            with self.subTest(op=c["op"], asked=c["asked"]):
                got = texts.buddy_change(c["text"], c["wallet"])
                self.assertEqual(got, {"day": c["day"], "op": c["op"], "args": c["args"], "at": c["at"]})
                self.assertTrue(texts.same_args(c["asked"], got["args"]))
                self.assertEqual(texts.read_play(c["text"], texts.BUDDY_ACTION, c["wallet"].upper().replace("0X", "0x")),
                                 {"day": c["day"], "detail": c["detail"]}, "a checksummed wallet reads the same text")

    def test_a_text_for_another_change_is_never_the_one_asked_for(self):
        for c in fx("buddy_texts.json")["refused"]:
            with self.subTest(why=c["why"]):
                got = texts.buddy_change(c["text"], c["wallet"])
                self.assertFalse(got is not None and got["op"] == c["op"] and texts.same_args(c["asked"], got["args"]))


class BoardTexts(unittest.TestCase):
    def test_every_case_rebuilds(self):
        for c in fx("board_texts.json")["cases"]:
            with self.subTest(op=c["op"], quest=c["quest_id"]):
                if c["op"] == "claim":
                    self.assertEqual(texts.claim_text(c["wallet"], c["quest_id"], c["day"]), c["text"])
                else:
                    self.assertEqual(texts.submit_detail(c["quest_id"], c["url"], c["n"]), c["detail"])
                    self.assertTrue(c["detail"].split(" ")[1] == c["url_sha16"])
                    self.assertEqual(texts.submit_text(c["wallet"], c["quest_id"], c["url"], c["n"], c["day"]),
                                     c["text"])


class EntitleTexts(unittest.TestCase):
    def test_every_case_rebuilds(self):
        for c in fx("entitle_texts.json")["cases"]:
            with self.subTest(wallet=c["wallet"]):
                self.assertEqual(texts.entitle_text(c["game"], c["wallet"], c["day"]), c["text"])


if __name__ == "__main__":
    unittest.main()

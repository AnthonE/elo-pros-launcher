"""The payout file: gamepay's own format check, pinned by sdk/testdata/."""
import json
import os
import tempfile
import unittest

from _testdata import fx

from elo_sdk import PayoutFile, PayoutRow, check_doc, check_row


class Payout(unittest.TestCase):
    def test_every_good_row_is_owed(self):
        good = fx("payout.json")
        self.assertEqual(check_doc(good), [])
        f = PayoutFile.from_json(good)
        self.assertEqual(f.validate(), [])
        self.assertEqual(f.payouts[0].reasons, ["EXTRACTED 120"])
        self.assertEqual(PayoutFile.from_json(json.loads(f.to_json())), f)

    def test_every_bad_row_and_file_is_refused(self):
        bad = fx("payout_bad.json")
        for b in bad["payouts"]:
            with self.subTest(row=b["row"]):
                why = check_row(b["row"])
                self.assertTrue(why and why.startswith(b["why"]), why)
        for b in bad["files"]:
            with self.subTest(file=b["why"]):
                self.assertTrue(check_doc(b["doc"]))
                with self.assertRaises(ValueError):
                    PayoutFile.from_json(b["doc"])

    def test_write_atomic(self):
        f = PayoutFile(3).pay(PayoutRow("0x2222222222222222222222222222222222222222", 120, "JUNK")
                              .because("EXTRACTED 120"))
        self.assertEqual(f.file_name("island"), "island.payout-3.json")
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, f.file_name("island"))
            f.write_atomic(path)
            with open(path, encoding="utf-8") as fh:
                self.assertEqual(PayoutFile.from_json(json.load(fh)), f)
            with self.assertRaises(ValueError):
                PayoutFile(0).write_atomic(os.path.join(d, "bad.json"))
            self.assertEqual(sorted(os.listdir(d)), ["island.payout-3.json"], "nothing half-written is left")


if __name__ == "__main__":
    unittest.main()

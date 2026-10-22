"""Write a closed wipe's payout file the way the meter reads it.

    python3 examples/payout.py /tmp

A game server writes this when a wipe closes and serves it at
/payout/<wipe>.json beside status.json.
"""
import os
import sys

from elo_sdk import PayoutFile, PayoutRow

out_dir = sys.argv[1] if len(sys.argv) > 1 else "."
f = (PayoutFile(3)
     .pay(PayoutRow("0x2222222222222222222222222222222222222222", 120, "JUNK").because("EXTRACTED 120"))
     .pay(PayoutRow("0x3333333333333333333333333333333333333333", 40, "JUNK").because("EXTRACTED 40")))
problems = f.validate()
if problems:
    sys.exit("the meter would refuse this file: " + "; ".join(problems))
path = os.path.join(out_dir, f.file_name("island"))
f.write_atomic(path)
print("wrote", path)

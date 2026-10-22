"""Who is playing, and a signature, from the launcher on this machine.

    python3 examples/overlay.py

No launcher is a normal state: play anonymously, and never ask the player for
a key instead.
"""
from elo_sdk import Overlay, play_message

elo = Overlay(game="gates", version="0.1.0")
if not elo.connect():
    print(f"no elo launcher ({elo.why}) — playing anonymously")
elif not elo.address():
    print("the launcher has no account — playing anonymously")
else:
    wallet = elo.address()
    print(f"playing as {wallet} (signer: {elo.signer})")
    out = elo.sign(play_message("duel", wallet, "round 41: I won"), "settling round 41")
    if out.ok:
        print("signed:", out.signature)
    elif out.handoff:
        print("finish in your browser:", out.handoff)
    else:
        print("not signed:", out.reason)

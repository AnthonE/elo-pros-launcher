"""A wallet's friends and buddies, and who is playing what.

    python3 examples/friends.py 0xYOURWALLET
    python3 examples/friends.py 0xYOURWALLET ask 0xTHEIRWALLET

The second form signs through the launcher: the SDK rebuilds the text itself
and refuses a server that prepares anything else.
"""
import sys

from elo_sdk import ActError, Client, Overlay, overlay_signer

if len(sys.argv) < 2:
    sys.exit("usage: friends.py 0xWALLET [op 0xOTHER]")
wallet, elo = sys.argv[1], Client()

if len(sys.argv) >= 4:
    op, other = sys.argv[2], sys.argv[3]
    ov = Overlay(game="friends-example")
    if not ov.connect():
        sys.exit(f"a change is signed, and no launcher answered: {ov.why}")
    try:
        got = elo.friend(wallet, op, other, overlay_signer(ov, "a friends change"))
    except ActError as e:
        sys.exit(str(e))
    print(f"{op} done" if got.ok else f"{got.kind}: {got.error}")
    sys.exit(0)

got = elo.friends(wallet)
if not got.ok:
    sys.exit(f"{got.kind}: {got.error}")
view = got.value
if not view.presence.reachable:
    print(f"(presence could not be read: {view.presence.why})")
for f in view.friends + view.buddies:
    tag = " [buddy]" if f.is_buddy else ""
    if f.playing:
        print(f"{f.label}{tag} — playing {f.playing.game} on {f.playing.server_name or '?'}")
    else:
        print(f"{f.label}{tag} — {'online' if f.online else 'offline'}")
print(f"{view.count} friends, {len(view.buddies)} buddies, {len(view.asks_in)} asks waiting")

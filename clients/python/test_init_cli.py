"""Offline test for elo-pros-init's key file (elo_pros_client.init_cli).

The generated wallet key must never exist on disk readable by anyone but its
owner — not even for the instant between a write and a chmod, because a reader
that opened the file in that instant keeps reading after the chmod.
Run: python3 test_init_cli.py"""
import os
import stat
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

PASS = 0


def ok(cond, label):
    global PASS
    if not cond:
        print(f"FAIL: {label}")
        sys.exit(1)
    PASS += 1
    print(f"  ok: {label}")


home = Path(tempfile.mkdtemp()) / "elo-pros-home"
os.environ["ELO_PROS_HOME"] = str(home)
for _k in ("ELO_PROS_KEY", "SCRY_KEY", "SCRY_HOME"):
    os.environ.pop(_k, None)
from elo_pros_client import init_cli  # noqa: E402

# Record the file's mode at every moment key bytes reach it, however it is
# written (Path.write_text, or os.write on a raw fd).
seen = []
_write_text, _os_write = Path.write_text, os.write


def _spy_write_text(self, data, *a, **k):
    n = _write_text(self, data, *a, **k)
    if self.name == "key":
        seen.append(stat.S_IMODE(os.stat(self).st_mode))
    return n


def _spy_os_write(fd, data):
    n = _os_write(fd, data)
    seen.append(stat.S_IMODE(os.fstat(fd).st_mode))
    return n


print("[elo-pros-init: the generated key is owner-only from its first byte]")
old_umask = os.umask(0o022)
Path.write_text, os.write = _spy_write_text, _spy_os_write
try:
    key = init_cli._load_or_create_key()
finally:
    Path.write_text, os.write = _write_text, _os_write
    os.umask(old_umask)

kp = home / "key"
ok(key.startswith("0x") and len(key) == 66, "a key is generated")
ok(kp.read_text().strip() == key, "the key on disk is the key returned")
ok(seen and all(m & 0o077 == 0 for m in seen),
   f"the key file is never group/other-readable while written (modes {[oct(m) for m in seen]})")
ok(stat.S_IMODE(kp.stat().st_mode) == 0o600, "the key file ends 0600")
ok(stat.S_IMODE(home.stat().st_mode) & 0o077 == 0, "the home dir is created owner-only")
ok(init_cli._load_or_create_key() == key, "a second run loads the same key")

print("[the key an older client made is still the agent's key]")
from elo_pros_client import _home  # noqa: E402

_fake = Path(tempfile.mkdtemp())
_real_home_dir = Path.home
Path.home = staticmethod(lambda: _fake)
os.environ.pop("ELO_PROS_HOME", None)
try:
    ok(_home.home() == _fake / ".elo-pros", "with nothing on disk, the home is ~/.elo-pros")
    (_fake / ".scry").mkdir()
    ok(_home.home() == _fake / ".scry",
       "a ~/.scry from the old client is used where it is — the wallet does not change")
    (_fake / ".elo-pros").mkdir()
    ok(_home.home() == _fake / ".elo-pros", "once ~/.elo-pros exists it wins")
    os.environ["SCRY_HOME"] = str(_fake / "named")
    ok(_home.home() == _fake / "named", "the old $SCRY_HOME is still honoured")
    os.environ["ELO_PROS_HOME"] = str(_fake / "newer")
    ok(_home.home() == _fake / "newer", "…and $ELO_PROS_HOME outranks it")
    os.environ["SCRY_KEY"] = "0xold"
    ok(_home.env("KEY") == "0xold", "the old $SCRY_KEY is read when the new one is unset")
    os.environ["ELO_PROS_KEY"] = "0xnew"
    ok(_home.env("KEY") == "0xnew", "…and $ELO_PROS_KEY outranks it")
finally:
    Path.home = _real_home_dir
    for _k in ("ELO_PROS_HOME", "SCRY_HOME", "SCRY_KEY", "ELO_PROS_KEY"):
        os.environ.pop(_k, None)

print(f"\nALL {PASS} CHECKS PASS")

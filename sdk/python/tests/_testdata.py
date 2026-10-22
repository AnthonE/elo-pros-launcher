"""Where the tests find the package and the shared fixtures (sdk/testdata/)."""
import json
import sys
from pathlib import Path

PACKAGE = Path(__file__).resolve().parent.parent
TESTDATA = PACKAGE.parent / "testdata"
if str(PACKAGE) not in sys.path:
    sys.path.insert(0, str(PACKAGE))


def fx(name: str):
    return json.loads((TESTDATA / name).read_text(encoding="utf-8"))

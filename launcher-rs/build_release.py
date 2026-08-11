#!/usr/bin/env python3
"""Package the Rust desktop client for download. Stdlib only, no build step.

    python3 launcher-rs/build_release.py            # build, then write watchtower/dl/
    python3 launcher-rs/build_release.py --check    # do the committed files match?
    python3 launcher-rs/build_release.py --no-cargo # package binaries already built

Three artifacts, because they answer three different questions:

  scry_<v>_amd64.deb              the Ubuntu/Debian answer. `sudo apt install
                                  ./scry_<v>_amd64.deb` puts `scry` and
                                  `scry-gui` on PATH, adds the menu entry, and
                                  — the reason it is a package and not a
                                  tarball — declares the X11/pango libraries
                                  the window links, so a missing one is apt's
                                  problem and never a player's.
  scry-<v>-linux-x86_64.tar.gz    same two binaries, no root, any distro.
  scry-<v>-windows-x86_64.zip     `scry.exe` and `scry-gui.exe`. No installer,
                                  no redistributable, no runtime: unzip and run.

WHY THIS EXISTS BESIDE `launcher/build_package.py`: that one packages the
PYTHON client, which is `Architecture: all` and has no Windows story at all
(`docs/client/LAUNCHER.md` §12 row 4 — the `.deb` has no Windows analogue and the
`.pyz` still needs a Python). This one packages the Rust client, which is the
Windows client. The two coexist on purpose and the Debian package NAMES differ
(`scry` vs `scry-launcher`) so neither can silently replace the other.

⚠ **WHAT IS DETERMINISTIC HERE AND WHAT IS NOT — read before quoting a hash.**
The CONTAINERS are deterministic exactly as the Python packager's are: tar
mtime 0, uid/gid 0, uname/gname root, sorted members, gzip mtime 0, zip entries
pinned to the zip epoch, ar headers written by hand. Given the same binaries,
two runs an hour apart are byte-identical.

The BINARIES are not a claim this file gets to make. They come out of `cargo`,
and rustc embeds the build directory in panic paths unless it is remapped, so a
different checkout path is a different binary. **So the published sha256 answers
"did I download the bytes scry published", not "can I rebuild these bytes".**
That is a weaker promise than the Python client's and it is written down rather
than blurred, because a checksum sold as reproducibility that is not
reproducible teaches a reader that checking is theatre.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import re
import struct
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
OUT = REPO / "watchtower" / "dl"

WIN_TARGET = "x86_64-pc-windows-gnu"
LINUX_BIN = HERE / "target" / "release"
WIN_BIN = HERE / "target" / WIN_TARGET / "release"

PKG = "scry"


def version() -> str:
    """Read from the workspace manifest, never typed here. One source."""
    m = re.search(r'^\[workspace\.package\]$.*?^version = "([^"]+)"',
                  (HERE / "Cargo.toml").read_text(encoding="utf-8"),
                  re.MULTILINE | re.DOTALL)
    if not m:
        raise SystemExit("launcher-rs/Cargo.toml: no [workspace.package] version")
    return m.group(1)


VERSION = version()
DEB_NAME = f"{PKG}_{VERSION}_amd64.deb"
TGZ_NAME = f"{PKG}-{VERSION}-linux-x86_64.tar.gz"
ZIP_NAME = f"{PKG}-{VERSION}-windows-x86_64.zip"

# ── the Depends line, and why it is checked rather than trusted ──────────────
#
# `scry-gui` links FLTK, and FLTK on Linux links the X11 stack AND pango/cairo
# for text shaping — which is more than a one-line comment in scry-ui's manifest
# used to claim. A `Depends:` typed from that claim would have been wrong, so
# this table is CHECKED against the binary: `_needed()` reads DT_NEEDED straight
# out of the ELF, and any soname not covered below fails the build. A new
# transitive library therefore cannot arrive silently — which is the whole
# failure mode a package is supposed to prevent.
#
# Only the direct DT_NEEDED entries are mapped. Their own dependencies come
# along through the packages named here, which is what a Debian dependency
# means; `libc6` and `libgcc-s1` cover the runtime.
SONAME_PKG = {
    "libc.so.6": "libc6", "libm.so.6": "libc6", "libgcc_s.so.1": "libgcc-s1",
    # rustc emits the dynamic loader itself as a DT_NEEDED, which is unusual
    # but legal and shows up on both binaries. libc6 ships it.
    "ld-linux-x86-64.so.2": "libc6",
    "libX11.so.6": "libx11-6", "libXext.so.6": "libxext6",
    "libXcursor.so.1": "libxcursor1", "libXfixes.so.3": "libxfixes3",
    "libXft.so.2": "libxft2", "libXinerama.so.1": "libxinerama1",
    "libXrender.so.1": "libxrender1", "libXau.so.6": "libxau6",
    "libXdmcp.so.6": "libxdmcp6", "libxcb.so.1": "libxcb1",
    "libxcb-render.so.0": "libxcb-render0", "libxcb-shm.so.0": "libxcb-shm0",
    "libfontconfig.so.1": "libfontconfig1", "libfreetype.so.6": "libfreetype6",
    "libcairo.so.2": "libcairo2", "libpixman-1.so.0": "libpixman-1-0",
    "libpango-1.0.so.0": "libpango-1.0-0",
    "libpangocairo-1.0.so.0": "libpangocairo-1.0-0",
    "libpangoft2-1.0.so.0": "libpangoft2-1.0-0",
    "libharfbuzz.so.0": "libharfbuzz0b", "libgraphite2.so.3": "libgraphite2-3",
    "libfribidi.so.0": "libfribidi0", "libthai.so.0": "libthai0",
    "libdatrie.so.1": "libdatrie1",
    "libglib-2.0.so.0": "libglib2.0-0", "libgobject-2.0.so.0": "libglib2.0-0",
    "libgio-2.0.so.0": "libglib2.0-0", "libgmodule-2.0.so.0": "libglib2.0-0",
    "libpcre2-8.so.0": "libpcre2-8-0", "libffi.so.8": "libffi8",
    "libz.so.1": "zlib1g", "libpng16.so.16": "libpng16-16",
    "libexpat.so.1": "libexpat1", "libbz2.so.1.0": "libbz2-1.0",
    "libbrotlidec.so.1": "libbrotli1", "libbrotlicommon.so.1": "libbrotli1",
    "libselinux.so.1": "libselinux1", "libmount.so.1": "libmount1",
    "libblkid.so.1": "libblkid1", "libbsd.so.0": "libbsd0", "libmd.so.0": "libmd0",
}


def _needed(elf: bytes) -> list[str]:
    """The ELF's own DT_NEEDED list — 64-bit little-endian, which is the only
    shape this target produces. Read here rather than shelled out to `readelf`
    so the package's dependency check is a property of the repo and not of
    whichever machine ran it (`Gates/ci/depot.py` reads the same table)."""
    if elf[:4] != b"\x7fELF" or elf[4] != 2 or elf[5] != 1:
        raise SystemExit("not a 64-bit little-endian ELF")
    e_shoff, = struct.unpack_from("<Q", elf, 0x28)
    e_shentsize, e_shnum = struct.unpack_from("<HH", elf, 0x3A)
    dynamic = strtab = None
    for i in range(e_shnum):
        off = e_shoff + i * e_shentsize
        sh_type, = struct.unpack_from("<I", elf, off + 4)
        sh_offset, sh_size = struct.unpack_from("<QQ", elf, off + 0x18)
        sh_link, = struct.unpack_from("<I", elf, off + 0x28)
        if sh_type == 6:                      # SHT_DYNAMIC
            dynamic = (sh_offset, sh_size, sh_link)
        elif sh_type == 3 and strtab is None:  # first SHT_STRTAB, replaced below
            pass
    if dynamic is None:
        return []
    d_off, d_size, d_link = dynamic
    s_off = e_shoff + d_link * e_shentsize    # the strtab DT_NEEDED indexes into
    strtab_off, = struct.unpack_from("<Q", elf, s_off + 0x18)
    out = []
    for pos in range(d_off, d_off + d_size, 16):
        d_tag, d_val = struct.unpack_from("<qQ", elf, pos)
        if d_tag == 0:                        # DT_NULL ends the table
            break
        if d_tag == 1:                        # DT_NEEDED
            end = elf.index(b"\0", strtab_off + d_val)
            out.append(elf[strtab_off + d_val:end].decode())
    return sorted(set(out))


def depends(binaries: list[bytes]) -> str:
    """The Depends: field, derived from the binaries and never typed."""
    unmapped, pkgs = [], set()
    for b in binaries:
        for so in _needed(b):
            if so in SONAME_PKG:
                pkgs.add(SONAME_PKG[so])
            else:
                unmapped.append(so)
    if unmapped:
        raise SystemExit(
            "the binaries need libraries this package does not declare:\n  "
            + "\n  ".join(sorted(set(unmapped)))
            + "\nAdd each to SONAME_PKG with the Debian package that ships it. "
              "An undeclared dependency is the failure a .deb exists to prevent.")
    return ", ".join(sorted(pkgs))


DESCRIPTION = """\
 The desktop client for scry: a curated, open-source-required game platform
 on its own chain. Opens the platform's games, browses the catalog, shows
 what a wallet holds, reads the hive, and installs hash-verified desktop
 builds when a title publishes one.
 .
 Two programs. "scry" is the command line — it installs, verifies, updates
 and starts a build, and needs no desktop, which is what a server, a CI job
 or a training harness wants. "scry-gui" is the window.
 .
 It holds no keys unless you ask it to make you an account, and that account
 is yours: generated on this machine, encrypted with your passphrase, sent
 nowhere, restorable in any wallet from twelve words.
 .
 It is never required. Every surface it shows is a URL that works in a
 browser without it.
"""

DESKTOP = """\
[Desktop Entry]
Type=Application
Name=scry
GenericName=Game launcher
Comment=The scry desktop client — games built by agents, owned by players
Exec=scry-gui
Icon=scry
Terminal=false
Categories=Game;
StartupNotify=true
Keywords=scry;games;launcher;
"""

# The url handler — `scry://join/<title>/host:port`, so a link a friend pastes
# into a chat window starts the right game on the right shard.
#
# **A SECOND entry, hidden, rather than a MimeType line on the one above**, and
# the split is deliberate on both halves:
#
#   - `Exec` differs. The visible entry opens the window (`scry-gui`); a join
#     link has to reach `scry join %u`, which resolves the link and starts the
#     game. One entry cannot do both, because a desktop passes `%u` to whatever
#     Exec says and `scry-gui` takes no url.
#   - `NoDisplay=true` keeps it out of the applications menu. It is a handler,
#     not a program anybody launches on purpose, and a second "scry" in the
#     menu would be a bug report.
#
# `%u` is a SINGLE url. Never `%U`: a handler that accepts a list is one a
# caller can hand twenty links to, and this starts a game.
SCHEME_DESKTOP = """\
[Desktop Entry]
Type=Application
Name=scry join link
Comment=Open a scry:// link — a game, on the shard the link names
Exec=scry join %u
Icon=scry
Terminal=false
NoDisplay=true
MimeType=x-scheme-handler/scry;
"""

# Registering the scheme is a step BEYOND copying the file in.
#
# A `.desktop` carrying `MimeType=` does nothing until the desktop's mime cache
# is rebuilt — so without this the handler is installed, correct, and never
# called, which is the worst of the three states because nothing reports it.
#
# Guarded on the tool existing: a server or a container has no desktop database
# and must not fail an install over one. `|| true` on the run itself for the
# same reason — a mime cache is not worth a failed package.
POSTINST = """\
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications || true
fi
exit 0
"""

# The same mark the Python package ships and the website draws in CSS
# (css/store.css .wordmark) — one square, scalable, no raster, no request.
ICON = """\
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <rect width="64" height="64" rx="10" fill="#0e1210"/>
  <rect x="14" y="14" width="22" height="22" rx="3" fill="#00c805"/>
  <rect x="14" y="42" width="36" height="8" rx="2" fill="#a0aa95"/>
</svg>
"""

COPYRIGHT = """\
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: scry
Source: https://github.com/AnthonE/scry

Files: *
Copyright: 2026 MoreRight
License: MIT

Files: (statically linked) fltk
Copyright: 1998-2024 Bill Spitzak and others
License: FLTK
 This program uses the FLTK toolkit (https://www.fltk.org/), which is
 distributed under the FLTK License — the GNU LGPL with exceptions permitting
 static linking. This notice is that credit.
"""

README_WIN = f"""\
scry launcher {VERSION} — Windows, x86-64
=========================================

Two programs, no installer and nothing to register:

  scry-gui.exe    the window. Double-click it.
  scry.exe        the command line. Run it from PowerShell or cmd:

      scry check              what this machine can run
      scry games              every title the origin serves a manifest for
      scry install <title>    install the native build for this machine
      scry play <title>       start the newest installed build

Nothing to install alongside these. They import the DLLs Windows already
ships and no redistributable runtime.

Where it keeps things
---------------------
Games install under %LOCALAPPDATA%\\scry\\games and settings under
%APPDATA%\\scry. Nothing is written outside your profile and nothing needs
administrator rights.

What it will not do
-------------------
Custody anything. It can make you an account, and that account is yours:
generated on this machine, encrypted with your passphrase, sent nowhere, and
restorable in any wallet from twelve BIP-39 words. scry never sees it.

Be required. Every surface it shows is a URL that works in a browser without
it. There is no launcher-only anything.

Invent. If it cannot reach the origin it says so — which is a different
sentence from "there are none", and the difference is the point.

The design of record is https://scry.moreright.xyz/read.html?doc=LAUNCHER
The source is https://github.com/AnthonE/scry
"""


# ── deterministic containers ────────────────────────────────────────────────

def _ti(name: str, size: int, mode: int = 0o644, typ: bytes = tarfile.REGTYPE):
    """One tar member with every clock and identity knob pinned."""
    t = tarfile.TarInfo(name)
    t.size, t.mode, t.type = size, mode, typ
    t.mtime = 0
    t.uid = t.gid = 0
    t.uname = t.gname = "root"
    return t


def _tar_gz(members: list[tuple[str, bytes, int]], dirs: list[str]) -> bytes:
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.GNU_FORMAT) as tf:
        for d in dirs:
            tf.addfile(_ti(d, 0, 0o755, tarfile.DIRTYPE))
        for name, data, mode in members:
            tf.addfile(_ti(name, len(data), mode), io.BytesIO(data))
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", mtime=0) as gz:
        gz.write(raw.getvalue())
    return out.getvalue()


def _ar(members: list[tuple[str, bytes]]) -> bytes:
    """A GNU `ar` archive — the container a .deb actually is."""
    out = bytearray(b"!<arch>\n")
    for name, data in members:
        out += (f"{name:<16}{0:<12}{0:<6}{0:<6}{0o100644:<8o}"
                f"{len(data):<10}").encode() + b"`\n"
        out += data
        if len(data) % 2:
            out += b"\n"
    return bytes(out)


def _zip(members: list[tuple[str, bytes]]) -> bytes:
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in sorted(members):
            zi = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            zi.external_attr = 0o644 << 16
            zi.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(zi, data)
    return out.getvalue()


# ── the three artifacts ─────────────────────────────────────────────────────

def build_deb(cli: bytes, gui: bytes) -> bytes:
    dirs = ["./usr/", "./usr/bin/", "./usr/share/", "./usr/share/applications/",
            "./usr/share/doc/", "./usr/share/doc/scry/", "./usr/share/icons/",
            "./usr/share/icons/hicolor/", "./usr/share/icons/hicolor/scalable/",
            "./usr/share/icons/hicolor/scalable/apps/"]
    files = [
        ("./usr/bin/scry", cli, 0o755),
        ("./usr/bin/scry-gui", gui, 0o755),
        ("./usr/share/applications/scry.desktop", DESKTOP.encode(), 0o644),
        ("./usr/share/applications/scry-join.desktop", SCHEME_DESKTOP.encode(), 0o644),
        ("./usr/share/icons/hicolor/scalable/apps/scry.svg", ICON.encode(), 0o644),
        ("./usr/share/doc/scry/copyright", COPYRIGHT.encode(), 0o644),
    ]
    files.sort()
    data_tar = _tar_gz(files, dirs)
    control = (
        f"Package: {PKG}\n"
        f"Version: {VERSION}\n"
        "Section: games\n"
        "Priority: optional\n"
        "Architecture: amd64\n"
        f"Depends: {depends([cli, gui])}\n"
        "Maintainer: scry <admin@scry.moreright.xyz>\n"
        "Homepage: https://scry.moreright.xyz/\n"
        f"Installed-Size: {(len(cli) + len(gui)) // 1024}\n"
        "Description: the scry desktop client\n"
        f"{DESCRIPTION}")
    # md5sums is not decoration: `dpkg -V scry` reads it, and that is how a
    # user checks an install has not been edited under them.
    md5s = "".join(f"{hashlib.md5(d).hexdigest()}  {n[2:]}\n"
                   for n, d, _ in files).encode()
    # `postinst` is 0o755 — dpkg will not run a maintainer script it cannot
    # execute, and it fails the install rather than skipping it.
    ctrl_tar = _tar_gz([("./control", control.encode(), 0o644),
                        ("./md5sums", md5s, 0o644),
                        ("./postinst", POSTINST.encode(), 0o755)], ["./"])
    return _ar([("debian-binary", b"2.0\n"),
                ("control.tar.gz", ctrl_tar),
                ("data.tar.gz", data_tar)])


def build_tgz(cli: bytes, gui: bytes) -> bytes:
    root = f"{PKG}-{VERSION}-linux-x86_64"
    return _tar_gz([(f"./{root}/scry", cli, 0o755),
                    (f"./{root}/scry-gui", gui, 0o755),
                    (f"./{root}/COPYRIGHT", COPYRIGHT.encode(), 0o644)],
                   [f"./{root}/"])


def build_zip(cli: bytes, gui: bytes) -> bytes:
    return _zip([("scry.exe", cli), ("scry-gui.exe", gui),
                 ("README.txt", README_WIN.replace("\n", "\r\n").encode()),
                 ("COPYRIGHT.txt", COPYRIGHT.replace("\n", "\r\n").encode())])


def cargo(target: str | None) -> None:
    cmd = ["cargo", "build", "--release", "--workspace"]
    if target:
        cmd += ["--target", target]
    print("  $ " + " ".join(cmd))
    subprocess.run(cmd, cwd=HERE, check=True)


def read(path: Path) -> bytes:
    if not path.is_file():
        raise SystemExit(
            f"{path} is missing — build it first, or drop --no-cargo.\n"
            f"The Windows half needs the mingw cross toolchain:\n"
            f"  sudo apt install mingw-w64 && rustup target add {WIN_TARGET}")
    return path.read_bytes()


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="package the Rust desktop client")
    ap.add_argument("--check", action="store_true",
                    help="verify the committed artifacts against SHA256SUMS")
    ap.add_argument("--no-cargo", action="store_true",
                    help="package binaries already in target/, do not build")
    args = ap.parse_args(argv)

    # --check never builds and never needs a toolchain. It is the gate the site
    # suite runs, so it must work on a machine with no cargo at all: it asks
    # whether the bytes on the origin are the bytes SHA256SUMS names, which is
    # the only claim the page makes about them.
    if args.check:
        sums = {}
        for line in (OUT / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
            h, n = line.split("  ", 1)
            sums[n] = h
        bad = []
        for name in (DEB_NAME, TGZ_NAME, ZIP_NAME):
            p = OUT / name
            if not p.is_file():
                bad.append(f"{name}: not committed")
            elif name not in sums:
                bad.append(f"{name}: not in SHA256SUMS")
            elif hashlib.sha256(p.read_bytes()).hexdigest() != sums[name]:
                bad.append(f"{name}: bytes disagree with SHA256SUMS")
            else:
                print(f"ok   {name}  {p.stat().st_size:>9,}B  sha256 {sums[name]}")
        for line in bad:
            print("FAIL " + line, file=sys.stderr)
        return 1 if bad else 0

    if not args.no_cargo:
        print("building the Linux binaries")
        cargo(None)
        print("building the Windows binaries")
        cargo(WIN_TARGET)

    built = {
        DEB_NAME: build_deb(read(LINUX_BIN / "scry"), read(LINUX_BIN / "scry-gui")),
        TGZ_NAME: build_tgz(read(LINUX_BIN / "scry"), read(LINUX_BIN / "scry-gui")),
        ZIP_NAME: build_zip(read(WIN_BIN / "scry.exe"), read(WIN_BIN / "scry-gui.exe")),
    }

    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in built.items():
        (OUT / name).write_bytes(data)
        print(f"wrote {OUT / name}  {len(data):,}B  "
              f"sha256 {hashlib.sha256(data).hexdigest()}")

    # SHA256SUMS covers EVERY artifact in dl/, both clients — a reader checking
    # one file should not have to know which packager wrote it. The Python
    # packager writes its own two entries the same way, so this file is rebuilt
    # from the directory rather than from either script's idea of the list.
    lines = []
    for p in sorted(OUT.iterdir()):
        if p.name == "SHA256SUMS" or not p.is_file():
            continue
        lines.append(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n")
    (OUT / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")
    print(f"wrote {OUT / 'SHA256SUMS'}  ({len(lines)} artifacts)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ScrySeatArt — the Scry Hive's face, drawn on chain, owned by nobody
/// @notice `SCRY-HIVE.md` §3j asked for a face that is HELD. This is what makes
///         "held" true rather than rented: the art is not a URL, not a pin, not
///         a file on our box. It is this contract's own bytecode, and
///         `tokenURI` builds the SVG and the JSON at read time.
///
///         ⚠ THERE IS NO OWNER AND THERE ARE NO SETTERS. The first version of
///         this contract uploaded sprite data and had the operator `seal()` it.
///         The art is now entirely procedural, so there is nothing to upload
///         and nothing to seal — no key exists that could change a pixel, from
///         the moment it is deployed. That is a strictly stronger answer to the
///         question this audience actually asks, and it costs nothing.
///
///         ⚠ AND THIS IS WHY THE ART IS PIXELS. Not taste: 32x32 on a small
///         indexed palette is the only medium that fits in a contract at all.
///         The style follows the custody decision, not the other way round.
///
///         THE CHARACTER: a round blue creature with two big eyes and a green
///         sprout on its head (operator, 2026-08-10, with a Chrono Trigger
///         reference sheet: *"i really wanna make our NFTs look more like nu"*).
///         It replaced a hooded figure with one enormous eye, and the three
///         shapes that carry it at thumbnail size are one rounded mass, two
///         wide-set eyes, and the tuft. Ten rolled axes dress it.
///
///         The palette is the store's own ramp — cream, a green trio, rust,
///         door-gold — plus a pigment shelf for the `iris` axis, plus exactly
///         two slots of blue for the `nu` hide, because the shelf had no blue a
///         BODY could be painted in. The cyberpunk still rides as engraving, a
///         woodcut, rather than as glow.
///
///         The reveal rides `ScrySeat`'s commitment: `sha256(salt)` is posted
///         at deploy, before a seat exists; the salt is published when the run
///         closes; every face is then `keccak256(salt ++ tokenId)`, recomputable
///         by anyone against a hash that predates the mint. Before the salt
///         lands, every seat renders the same sealed card — the deck cannot be
///         read early because the input to read it with does not exist yet.
///
/// @dev The authoring source of truth is `art/seat/sprites.py`, and
///      `art/seat/test_parity.py` runs THIS contract in an EVM and compares its
///      output pixel-for-pixel against that Python. Two implementations of one
///      picture always drift, and when they drift the page shows one seat and
///      the wallet shows another — so that test is the important one here.
///      `art/seat/build.js` is what regenerates its fixture.
contract ScrySeatArt {
    uint256 internal constant S = 32;
    uint8 internal constant N_LAYERS = 10;

    // layer order == hash-slice order. `sprites.py` TABLES must match exactly.
    uint8 internal constant L_SPREAD = 0;
    uint8 internal constant L_FRAME = 1;
    uint8 internal constant L_HIDE = 2;
    uint8 internal constant L_BODY = 3;
    uint8 internal constant L_EYE = 4;
    uint8 internal constant L_GEAR = 5;
    uint8 internal constant L_MARK = 6;
    uint8 internal constant L_AURA = 7;
    uint8 internal constant L_IRIS = 8;
    /// ⚠ APPENDED, NOT INSERTED. Layer index IS the hash-slice index, so a new
    /// axis anywhere but the end re-reads every slice after it and repaints the
    /// whole run. At 9 it takes a 16-bit word nothing else was using.
    ///
    /// ⚠ AND IT HAS NO `none` VALUE, alone among the decorative axes. A Nu
    /// without its tuft is not a Nu, so this axis varies the SHAPE of a thing
    /// every seat has rather than whether it has one.
    uint8 internal constant L_SPROUT = 9;

    /// How many layers this renderer draws. A read rather than a constant the
    /// callers copy: the parity harness used to hardcode 8 and would have checked
    /// eight of nine tables after the `iris` axis landed.
    function layerCount() external pure returns (uint8) {
        return N_LAYERS;
    }

    string public collectionName;
    string public collectionDescription;

    constructor(string memory name_, string memory desc_) {
        collectionName = name_;
        collectionDescription = desc_;
    }

    // ── the palette: watchtower/css/store.css, the STORE skin ────────────────
    // index 0 is transparent and never drawn; the spread fills every cell first.
    //
    // ⚠ A COLOUR SLOT IS ALSO A TRAIT NAME. `_spreadInk` and `_hideInk` publish
    // names — "rust", "verdigris", "moss", "nu" — straight into the token's
    // attributes, so a repaint that leaves a name behind makes the metadata
    // describe a colour the picture does not show. Check the names when you
    // touch the pigments.
    // ⚠ A PACKED TABLE, NOT A BRANCH CHAIN, AND THAT IS A SIZE DECISION. Thirty
    // two `if (i == n) return 0x......;` lines cost roughly 780 bytes of
    // runtime; three bytes in a constant cost three bytes. The Nu remake landed
    // at 25,664 B against EIP-170's 24,576 ceiling — a contract that compiles,
    // passes every test, and cannot be deployed — and packing this table, the
    // luma ladder and the weight rows is what bought the 1,088 bytes back. The
    // ceiling is also why the art is 32x32 on an indexed palette in the first
    // place, so this is the same constraint one level down.
    //
    //   0  unused (transparent, never drawn — the spread fills every cell)
    //   1..15   the STORE's ramp: cream · recessed · raised · line · sage ·
    //           deep moss · the void · ink-dim · door-gold · gild-dim ·
    //           gild-ink · rust · rust-dim · verdigris · panel highlight
    //   16..29  the pigment shelf, carrying the `iris` axis: ember · slate ·
    //           ice · arcane · lilac · malachite · leaf green · brass ·
    //           pale gold · garnet · rose · teal ink · violet ink · cold bone
    //   30..31  nu blue and its lit top — the two slots the creature added
    bytes constant _PAL = hex"000000f6eae0efe2d6e7dacbd6c9b8767f621e4a220a0c0a616658b98a2f"
        hex"946d248a6410ce422b8e2c1c2ca243fbf4ecd9483b4e7fa87fb2c96e4b8e9b6bc4107c10"
        hex"77bb44c9a227e8c55fa83b5cd96f8e2f48583b2c4ab8c4c939428c6878c8";

    function palette(uint8 i) public pure returns (bytes3) {
        bytes memory p = _PAL;
        uint256 o = uint256(i) * 3;
        return bytes3(
            uint24(
                (uint256(uint8(p[o])) << 16) | (uint256(uint8(p[o + 1])) << 8)
                    | uint256(uint8(p[o + 2]))
            )
        );
    }

    /// Weights in basis points. ⚠ EVERY ROW MUST SUM TO EXACTLY 10,000 —
    /// `layerSumsAreExact()` asserts it and `sprites._audit()` asserts the same
    /// tables on the Python side. A row summing OVER makes its own tail
    /// unreachable (the cumulative walk exits before arriving); one summing
    /// UNDER silently inflates the last entry. Both publish a rarity table that
    /// is false, and the second lies about the rarest traits specifically.
    /// ⚠ ONE ROW PER LITERAL, BIG-ENDIAN, `n` ENTRIES OF 16 BITS. Ten weights
    /// times sixteen bits is 160, so every row fits one word with room to
    /// spare — and five separate `_wN(...)` builders, each with its own
    /// argument shuffling at every call site, cost about 600 bytes this
    /// contract did not have. Read left to right, the hex IS the table:
    /// 05DC = 1500, 04B0 = 1200, and so on.
    function weights(uint8 layer) public pure returns (uint16[] memory) {
        if (layer == L_SPREAD) return _wUnpack(0x05DC04B00514044C044C044C03E803840320, 9);
        // ⚠ FRAMES ARE RARE. They were 70% of the run; a frame costs two columns
        // on every side of a 32px tile and none of the collections this sits
        // beside in a marketplace grid uses one.
        if (layer == L_FRAME) return _wUnpack(0x1B5804B003840258012C, 5);
        // ⚠ `nu` IS THE COMMONEST HIDE ON PURPOSE — it is what makes a stranger
        // scrolling a grid recognise the character at all. The rest of the run
        // wears the store's own ramp, which keeps the collection on the same
        // page as the site that mints it.
        // ⚠ THE LEADING `00` IS LOAD-BEARING. Ten entries is exactly twenty
        // bytes, and a 20-byte hex literal is an ADDRESS to solc — it rejects
        // this row for a bad checksum and then for "invalid implicit conversion
        // from address to uint256". Padding to 21 bytes makes it a number
        // again; it changes no weight.
        if (layer == L_HIDE) return _wUnpack(0x00070805DC051404B003E8038402BC025801F401F4, 10);
        if (layer == L_BODY) return _wUnpack(0x08FC07D0076C064005780320, 6);
        if (layer == L_EYE) return _wUnpack(0x0898076C05DC05780514044C0258, 7);
        if (layer == L_GEAR) return _wUnpack(0x0BB807080640057805140384, 6);
        if (layer == L_MARK) return _wUnpack(0x0BB8076C0640057804B00384, 6);
        if (layer == L_AURA) return _wUnpack(0x0ED80898070805780320, 5);
        if (layer == L_IRIS) return _wUnpack(0x0A28064005DC0578051403E80258, 7);
        return _wUnpack(0x09600834070805DC05780320, 6); // sprout
    }

    function _wUnpack(uint256 packed, uint256 n) internal pure returns (uint16[] memory o) {
        o = new uint16[](n);
        for (uint256 i; i < n; i++) o[n - 1 - i] = uint16(packed >> (16 * i));
    }

    // ── the names, packed ────────────────────────────────────────────────────
    // ⚠ ONE BLOB PLUS A LENGTH BYTE EACH, NOT TEN `string[N] memory` LITERALS.
    // Materialising a fixed array of strings emits, per entry, a pointer, a
    // length word and padded data — about forty bytes of runtime for a word
    // like "gild". Seventy-seven of those is most of a kilobyte, and this
    // contract was 254 bytes over EIP-170's ceiling after the palette and the
    // weight rows were already packed. Here a name costs its own letters plus
    // one byte.
    //
    // Entries 0..66 are the trait values in layer order; 67..76 are the layer
    // names themselves. `_NBASE` holds each layer's first index, and its
    // eleventh entry (67) is where the layer names begin — so `layerName(i)` is
    // just `_name(_NBASE[10] + i)` and needs no second table.
    //
    // ⚠ GENERATED FROM `sprites.py`'s OWN TABLES, never typed. `test_parity`
    // compares every layer's names and weights against that Python, so a slip
    // here fails the gate rather than shipping metadata that names a trait the
    // picture does not have.
    bytes constant _NM = hex"70617065727265636573736564696e6b6c696e65656e677261766567696c64727573747465616c76696f6c65746e6f6e6568616972636f726e657267"
        hex"696c6472756c656e756d6f7373696e6b626f6e6567696c64727573746661696e74736c617465766572646967726973706c756d726f756e6473717561"
        hex"7474616c6c62726f616470656172626f756c64657277696465726f756e64736c697472696e67656463726f737373706972616c736875746e6f6e6576"
        hex"69736f72686561647365746a61636b616e74656e6e61646973686e6f6e65636c61737063697263756974776172647365616c74616c6c796e6f6e6573"
        hex"6d6f6b65737061726b73676c7970687368616c6f676f6c64696365656d6265726d616c616368697465617263616e656761726e657462726173737370"
        hex"72696770616c6d6665726e62756474696172616375726c7370726561646672616d6568696465626f6479657965676561726d61726b61757261697269"
        hex"737370726f7574";
    bytes constant _NLEN = hex"050803040704040406040406040402040304040405050904050504050407040504060506040405070407040405070404050405060604040305090606"
        hex"0505040403050406050404030404040406";
    bytes constant _NBASE = hex"00090e181e252b31363d43";

    function _name(uint256 idx) internal pure returns (string memory) {
        bytes memory lens = _NLEN;
        uint256 off;
        for (uint256 i; i < idx; i++) off += uint8(lens[i]);
        uint256 n = uint8(lens[idx]);
        bytes memory all = _NM;
        bytes memory o = new bytes(n);
        for (uint256 i; i < n; i++) o[i] = all[off + i];
        return string(o);
    }

    function names(uint8 layer, uint8 v) public pure returns (string memory) {
        return _name(uint8(_NBASE[layer]) + uint256(v));
    }

    function layerName(uint8 i) public pure returns (string memory) {
        return _name(uint8(_NBASE[N_LAYERS]) + uint256(i));
    }

    // ── trait selection ──────────────────────────────────────────────────────
    function traitsOf(uint256 tokenId, string memory salt)
        public
        pure
        returns (uint8[N_LAYERS] memory out)
    {
        uint256 h = uint256(keccak256(abi.encodePacked(salt, tokenId)));
        for (uint8 i; i < N_LAYERS; i++) {
            out[i] = _pick(i, uint16(h >> (240 - 16 * uint256(i))));
        }
    }

    /// ⚠ `(word * 10000) >> 16`, and NEVER `word % 10000`. 65536 is not a
    /// multiple of 10000, so a remainder hands residues 0..5535 seven chances
    /// per range and 5536..9999 only six — over-drawing whatever sits early in
    /// each table by ~7% and starving the tail by ~8%. Measured on real tables
    /// before the fix: a 20.0% trait came out at 17.8%. That is the posted
    /// rarity being quietly false, welded, forever.
    function _pick(uint8 layer, uint16 word) internal pure returns (uint8) {
        uint16[] memory w = weights(layer);
        uint256 r = (uint256(word) * 10_000) >> 16;
        uint256 acc;
        for (uint256 i; i < w.length; i++) {
            acc += w[i];
            if (r < acc) return uint8(i);
        }
        return uint8(w.length - 1);
    }

    // ── the creature's geometry ──────────────────────────────────────────────
    /// Half-width per row for the blob.
    ///
    /// ⚠ A BLOB IS AN ELLIPSE THAT STOPS, AND THEN CURVES BACK IN. The arc runs
    /// from the crown to the SIT row — reached in about six rows, which is what
    /// separates a Nu from a cone — and the skirt below it narrows again, which
    /// is what separates a Nu from a box. Both were got wrong in turn.
    ///
    /// ⚠ THE SKIRT'S DIVISOR IS DERIVED so that EVERY body loses exactly five
    /// half-widths between its sit row and its base, however tall it is. One
    /// hand-tuned constant would make `boulder` (seventeen rows of skirt) taper
    /// three times as hard as `pear` (ten) — the axis changing a thing it does
    /// not name.
    ///
    /// ⚠ AND EVERY WIDTH IS AT LEAST 11, which is a face constraint. Two eye
    /// sockets of radius 4 set five columns off centre span 19 of 32 columns, so
    /// a narrower body wears its eyes on its own outline. The six shapes differ
    /// in height and in flare, never in whether the face fits.
    function _bodyWidths(uint8 kind) internal pure returns (uint8[32] memory h) {
        uint256 topRow;
        uint256 sit;
        uint256 bot;
        uint256 W;
        if (kind == 1) {
            (topRow, sit, bot, W) = (9, 14, 27, 13); // squat — low, wide, short
        } else if (kind == 2) {
            (topRow, sit, bot, W) = (2, 10, 25, 11); // tall
        } else if (kind == 3) {
            (topRow, sit, bot, W) = (7, 11, 27, 13); // broad — a flat top
        } else if (kind == 4) {
            (topRow, sit, bot, W) = (4, 17, 27, 12); // pear — a long taper
        } else if (kind == 5) {
            (topRow, sit, bot, W) = (6, 9, 26, 12); // boulder — near-vertical
        } else {
            (topRow, sit, bot, W) = (5, 12, 26, 11); // round — the dome
        }
        uint256 H = sit - topRow;
        uint256 K = (bot - sit) * (bot - sit) / 5;
        if (K == 0) K = 1;
        for (uint256 y = topRow; y <= bot; y++) {
            uint256 w;
            if (y >= sit) {
                uint256 dy = y - sit;
                uint256 shrink = (dy * dy) / K;
                w = shrink >= W ? 0 : W - shrink;
            } else {
                uint256 dy = sit - y;
                for (uint256 d = W + 1; d > 0; d--) {
                    uint256 dx = d - 1;
                    if (dx * dx * H * H + dy * dy * W * W <= W * W * H * H) {
                        w = dx;
                        break;
                    }
                }
            }
            // a 1px cap reads as a spike, not a dome
            h[y] = w >= 2 ? uint8(w) : 0;
        }
        // ⚠ THE BASE TUCKS IN, AND THAT IS WHAT MAKES THE LEGS LEGS. With a flat
        // base the feet sat inside the body's own footprint and read as two
        // notches bitten out of the bottom edge.
        if (h[bot] != 0) h[bot] = h[bot] >= 5 ? h[bot] - 2 : 3;
    }

    /// Rec.601 luma per palette slot, integer — the table that keeps the figure
    /// visible. Every "it vanished" bug this renderer has had was a fixed colour
    /// laid on a rolled one; every fix derives off this table instead of hoping
    /// two independent rolls differ. `_contour`, `_socket`, `_leafInk` and
    /// `_markInk` all branch on it.
    /// One byte per palette slot, same order as `_PAL`. Packed for the same
    /// reason the palette is — see the note there.
    bytes32 constant _LUM = 0x00ece5dccb79380b638e7165694774f57275a55d834f99a0c45f924234c1477c;

    function _lum(uint8 i) internal pure returns (uint256) {
        return uint256(uint8(_LUM[i]));
    }

    /// ⚠ THE CREATURE'S OUTLINE IS DERIVED FROM ITS GROUND AND NEVER ROLLED.
    /// When `spread` and `hide` were independent rolls, 8.8% of seats put the
    /// body's own colour on the background's own colour and the silhouette
    /// vanished — 431 of 8,192 rendered as an effectively empty square. An
    /// outline chosen against the ground cannot collide with it.
    function _contour(uint8 ground) internal pure returns (uint8) {
        return _lum(ground) < 96 ? 4 : 7;
    }

    /// The ring around each eye, DERIVED against the HIDE it sits on.
    ///
    /// ⚠ THE SAME BUG ONE LAYER IN, and it is the one the Nu created. The old
    /// art parked a single eye inside a black hood, so its separation was a
    /// property of the hood and held for every seat. A creature's eyes sit
    /// directly on its body, and the body's colour is rolled — so a fixed dark
    /// ring disappears on an `ink` hide exactly the way door-gold disappeared
    /// on a gold robe. Derived, an eye can never be lost in the animal.
    function _socket(uint8 fill) internal pure returns (uint8) {
        return _lum(fill) >= 96 ? 7 : 4;
    }

    /// The sprout's two greens, DERIVED against the ground behind them.
    ///
    /// ⚠ The sprout is the one mark every seat carries, so it is the one that
    /// must never disappear. Leaf green is 153 luma and the `gild` ground is
    /// 142 — eleven apart, which is a tuft you cannot see on 11% of the run.
    function _leafInk(uint8 ground) internal pure returns (uint8 leaf, uint8 stem) {
        if (_absDiff(_lum(22), _lum(ground)) >= 45) return (22, 21);
        return (21, 6);
    }

    function _absDiff(uint256 a, uint256 b) internal pure returns (uint256) {
        return a > b ? a - b : b - a;
    }

    function _min(uint256 a, uint256 b) internal pure returns (uint256) {
        return a < b ? a : b;
    }

    function _minI(int256 a, int256 b) internal pure returns (int256) {
        return a < b ? a : b;
    }

    function _set(bytes memory g, int256 x, int256 y, uint8 c) internal pure {
        if (x >= 0 && x < 32 && y >= 0 && y < 32) g[uint256(y) * S + uint256(x)] = bytes1(c);
    }

    /// The 32x32 grid of palette indices, in the reference's exact draw order.
    function grid(uint256 tokenId, string memory salt) public pure returns (bytes memory g) {
        return gridOf(traitsOf(tokenId, salt));
    }

    /// The same draw, from an EXPLICIT trait vector rather than a hash.
    ///
    /// ⚠ This exists so the collection's own emblem is the REAL art and not a
    /// second drawing of it. `contractURI` used to hand marketplaces
    /// `sealedSvg()` unconditionally, so the collection's logo stayed the
    /// pre-reveal placeholder forever, while every collection it sits beside has
    /// a designed mark. An emblem built here cannot drift from the seats.
    ///
    /// It leaks nothing: the vector is hand-picked and constant.
    function gridOf(uint8[N_LAYERS] memory t) public pure returns (bytes memory g) {
        g = new bytes(S * S);
        int256 cx = 15;

        uint8 ground = _spreadInk(t[L_SPREAD]);
        for (uint256 i; i < S * S; i++) g[i] = bytes1(ground);

        (uint8 dark, uint8 light) = _hideInk(t[L_HIDE], ground);
        uint8[32] memory hw = _bodyWidths(t[L_BODY]);
        (int256 top, int256 bot, int256 widest) = _extent(hw);

        // ⚠ THE DRAW IS SPLIT ACROSS HELPERS FOR A COMPILER REASON, not a taste
        // one: the whole body inline needs more live locals than the EVM's stack
        // gives a single Yul function, and solc fails the build with
        // "Variable var_x is 1 too deep in the stack". Each helper takes what it
        // needs and nothing more. Draw ORDER is the reference's exact order and
        // moving a call changes the picture.
        {
            // the aura goes down FIRST — an aura is behind an animal, and
            // drawing it last used to let it sit on top of the head it was
            // rising off
            (, uint8 glow, uint8 band) = _irisInk(t[L_IRIS]);
            _aura(g, cx, t[L_AURA], top, t[L_SPROUT], ground, glow, band);
        }
        _creature(g, cx, hw, top, bot, widest, t[L_SPROUT], dark, _contour(ground));
        _shade(g, cx, hw, top, bot, light);
        _sproutPaint(g, cx, top, t[L_SPROUT], ground);

        int256 ecy = _minI(top + 9, bot - 12);
        {
            (uint8 lens, uint8 glow, uint8 band) = _irisInk(t[L_IRIS]);
            _eyes(g, cx, ecy, t[L_EYE], dark, light, lens, glow, band);
        }
        _mouth(g, cx, ecy + 5, _socket(dark));
        _gear(g, cx, t[L_GEAR], widest, top, ecy);
        _mark(g, cx, t[L_MARK], dark, bot - 3);
        _frame(g, t[L_FRAME], ground);
    }

    /// The first drawn row, the last, and the widest half-width.
    ///
    /// ⚠ THE FIRST DRAWN ROW IS NOT THE PROFILE'S `topRow`. `_bodyWidths` drops
    /// any crown row narrower than two, so a body whose arc begins at row 5 is
    /// first drawn at row 6 — and the sprout, the shading and the halo all hang
    /// off THIS number. Reading the table instead put a check one row out.
    function _extent(uint8[32] memory hw)
        internal
        pure
        returns (int256 top, int256 bot, int256 widest)
    {
        top = 32;
        for (uint256 y; y < 32; y++) {
            if (hw[y] == 0) continue;
            if (top == 32) top = int256(y);
            bot = int256(y);
            if (int256(uint256(hw[y])) > widest) widest = int256(uint256(hw[y]));
        }
    }

    // ── the creature, as ONE mask ────────────────────────────────────────────
    /// ⚠ THE MASK IS WHAT MAKES LIMBS POSSIBLE. Body, arms, feet and sprout all
    /// go into one boolean shape; the outline is that shape dilated by two and
    /// the fill is the shape itself. Stroking row by row — what the hooded bust
    /// did — can only outline a silhouette that is a single span per row, so the
    /// gap between two feet would go unoutlined and the creature would read as a
    /// blob with a notch in it.
    ///
    /// ⚠ AND THE OUTLINE IS TWO PIXELS, GROWING OUTWARD (operator, 2026-08-09:
    /// *"make the outline in the pixel art thicker"*). At 32px a 1px line
    /// disappears in a marketplace thumbnail.
    function _creature(
        bytes memory g,
        int256 cx,
        uint8[32] memory hw,
        int256 top,
        int256 bot,
        int256 widest,
        uint8 sprout,
        uint8 dark,
        uint8 edge
    ) internal pure {
        bytes memory m = new bytes(S * S);
        for (int256 y = top; y <= bot; y++) {
            int256 w = int256(uint256(hw[uint256(y)]));
            for (int256 x = cx - w; x <= cx + w; x++) _set(m, x, y, 1);
        }
        // ⚠ ARMS REACH A FIXED COLUMN, they do not stick out a fixed distance.
        // 13 is as far as anything can go: dilating by two puts the outline at
        // 15, the last column before the edge. So a narrow body grows real
        // paddles and a broad one grows almost none — the body axis doing a
        // second job for free, and the reason the first cut's arms (two columns
        // past a shape already at full width) were invisible. They hang below
        // the face; at `bot - 8` they started on the eyes' own bottom row and
        // read as ears.
        for (int256 dy; dy < 4; dy++) {
            int256 outer = 13 - (dy * dy) / 3; // a paddle that rounds off downward
            for (int256 s = -1; s <= 1; s += 2) {
                for (int256 x = widest - 1; x <= outer; x++) _set(m, cx + s * x, bot - 7 + dy, 1);
            }
        }
        // feet: two stubs under the body, and the gap between them is exactly
        // what a per-row stroke could never have outlined
        for (int256 dy = -1; dy < 3; dy++) {
            for (int256 s = -1; s <= 1; s += 2) {
                for (int256 x = 3; x < 9; x++) _set(m, cx + s * x, bot + dy, 1);
            }
        }
        // the tuft goes into the SAME mask, so it carries the creature's own
        // outline rather than a hairline of its own
        _sproutAt(m, cx, top, sprout, 1);

        // ⚠ THE DILATION IS DONE IN BITS, AND THAT IS WHAT MAKES THIS TOKEN
        // DISPLAYABLE. Walking the 13 disc offsets per set pixel is the obvious
        // way and it cost 10.5 M gas on its own — `grid` went 947 k → 12.2 M and
        // `tokenURI` 13.9 M → 24.8 M, against the ~50 M `eth_call` ceiling that
        // is the difference between a token a wallet renders and one it cannot.
        // A 32-wide row is a uint32, so the whole disc is five shifted ORs per
        // row: ±2 columns on the same row, ±1 on the rows above and below, and
        // the bare row two away. Thirty-two iterations instead of thirteen
        // thousand. Bits pushed past either end are simply dropped, which is
        // exactly the off-canvas clipping `_set` was doing by hand.
        uint32[32] memory rows;
        for (uint256 y; y < S; y++) {
            uint32 r;
            for (uint256 x; x < S; x++) {
                if (m[y * S + x] != 0) r |= uint32(1) << uint32(x);
            }
            rows[y] = r;
        }
        uint32[32] memory out;
        for (uint256 y; y < S; y++) {
            uint32 r = rows[y];
            if (r == 0) continue;
            uint32 d1 = r | (r << 1) | (r >> 1);
            uint32 d2 = d1 | (d1 << 1) | (d1 >> 1);
            out[y] |= d2;
            if (y >= 1) out[y - 1] |= d1;
            if (y + 1 < S) out[y + 1] |= d1;
            if (y >= 2) out[y - 2] |= r;
            if (y + 2 < S) out[y + 2] |= r;
        }
        // ⚠ ONE PASS, MASK BEFORE OUTLINE. The reference paints `edge` over the
        // whole dilated region — interior included — and then paints `dark` back
        // over the mask. Testing the mask first in a single pass lands on the
        // identical grid and touches each cell once.
        for (uint256 y; y < S; y++) {
            for (uint256 x; x < S; x++) {
                if ((rows[y] >> x) & 1 != 0) g[y * S + x] = bytes1(dark);
                else if ((out[y] >> x) & 1 != 0) g[y * S + x] = bytes1(edge);
            }
        }
    }

    /// A dome catches light on its upper-left. Two rows of the crown plus a
    /// crescent down the near side is enough at this size; more reads as a
    /// second colour rather than as shading.
    function _shade(
        bytes memory g,
        int256 cx,
        uint8[32] memory hw,
        int256 top,
        int256 bot,
        uint8 light
    ) internal pure {
        for (int256 y = top; y < _minI(top + 3, bot); y++) {
            int256 w = int256(uint256(hw[uint256(y)]));
            if (w == 0) continue;
            for (int256 x = cx - w + 1; x < cx + w; x++) _set(g, x, y, light);
        }
        for (int256 y = top; y < bot - 4; y++) {
            int256 w = int256(uint256(hw[uint256(y)]));
            if (w != 0) _set(g, cx - w + 1, y, light);
        }
    }

    // ── the sprout ───────────────────────────────────────────────────────────
    /// One frond: up from the crown, drifting sideways every second row.
    function _frond(
        bytes memory dst,
        int256 cx,
        int256 top,
        int256 ox,
        uint256 n,
        int256 drift,
        uint8 v
    ) internal pure {
        int256 x = cx + ox;
        int256 y = top - 1;
        for (uint256 i; i < n; i++) {
            _set(dst, x, y, v);
            y -= 1;
            if (drift != 0 && i % 2 == 1) x += drift;
        }
    }

    /// ⚠ WRITTEN TWICE INTO TWO DIFFERENT BUFFERS, and that is the point of the
    /// `dst`/`v` parameters. The tuft has to be in the creature's mask BEFORE
    /// the outline is dilated, and painted green AFTER the body fill — and both
    /// passes must land on the same pixels. One function called twice cannot
    /// disagree with itself; two copies of the coordinates would.
    function _sproutAt(bytes memory dst, int256 cx, int256 top, uint8 kind, uint8 v)
        internal
        pure
    {
        if (kind == 0) {
            // sprig — three short shoots
            _frond(dst, cx, top, 0, 4, 0, v);
            _frond(dst, cx, top, -2, 3, -1, v);
            _frond(dst, cx, top, 2, 3, 1, v);
        } else if (kind == 1) {
            // palm — the classic tuft, five fronds fanning wide
            _frond(dst, cx, top, 0, 4, 0, v);
            _frond(dst, cx, top, -2, 3, -1, v);
            _frond(dst, cx, top, 2, 3, 1, v);
            _frond(dst, cx, top, -4, 2, -1, v);
            _frond(dst, cx, top, 4, 2, 1, v);
        } else if (kind == 2) {
            // fern — two tall fronds with barbs
            _frond(dst, cx, top, -1, 6, -1, v);
            _frond(dst, cx, top, 1, 5, 1, v);
            for (int256 dy = 2; dy <= 4; dy += 2) {
                _set(dst, cx - 3, top - dy, v);
                _set(dst, cx + 3, top - dy, v);
            }
        } else if (kind == 3) {
            // bud — one fat shoot
            for (int256 dx = -1; dx <= 1; dx++) _frond(dst, cx, top, dx, 3, 0, v);
        } else if (kind == 4) {
            // tiara — a low band of short spikes
            for (int256 dx = -4; dx <= 4; dx += 2) _frond(dst, cx, top, dx, 2, 0, v);
        } else {
            // curl — one frond that leans over
            _frond(dst, cx, top, 0, 3, 0, v);
            int256[4] memory px = [int256(1), 2, 3, 4];
            int256[4] memory py = [int256(4), 5, 5, 4];
            for (uint256 i; i < 4; i++) _set(dst, cx + px[i], top - py[i], v);
        }
    }

    function _sproutPaint(bytes memory g, int256 cx, int256 top, uint8 kind, uint8 ground)
        internal
        pure
    {
        (uint8 leaf, uint8 stem) = _leafInk(ground);
        _sproutAt(g, cx, top, kind, leaf);
        for (int256 dx = -1; dx <= 1; dx++) _set(g, cx + dx, top, stem);
    }

    /// ⚠ HOW FAR ABOVE THE CROWN EACH TUFT REACHES. The aura is drawn FIRST — an
    /// aura is behind an animal — so the halo has to know the sprout's height
    /// before a single frond exists. `sprites._audit` measures the real topmost
    /// frond of every kind and asserts these numbers, because a taller sprout
    /// with a stale entry here puts the halo through the leaves, and that would
    /// be invisible in any render that did not roll both traits at once.
    function _sproutH(uint8 kind) internal pure returns (int256) {
        if (kind == 0) return 4; // sprig
        if (kind == 1) return 4; // palm
        if (kind == 2) return 6; // fern
        if (kind == 3) return 3; // bud
        if (kind == 4) return 2; // tiara
        return 5; // curl
    }

    // ── the eyes: TWO, and they are the face ─────────────────────────────────
    /// ⚠ SET WIDE, WITH A SMALL PUPIL. A Nu's eyes are most of its face and sit
    /// near the edges of the head; centred eyes with big pupils read as a
    /// cartoon bear. The socket ring is derived (see `_socket`) so neither eye
    /// can be lost in the hide beneath it.
    function _eyes(
        bytes memory g,
        int256 cx,
        int256 ecy,
        uint8 k,
        uint8 dark,
        uint8 light,
        uint8 lens,
        uint8 glow,
        uint8 band
    ) internal pure {
        uint8 sock = _socket(dark);
        for (int256 s = -1; s <= 1; s += 2) {
            int256 ex = cx + s * 5;
            for (int256 dy = -4; dy <= 4; dy++) {
                for (int256 dx = -4; dx <= 4; dx++) {
                    if (dx * dx + dy * dy <= 16) _set(g, ex + dx, ecy + dy, sock);
                }
            }
            if (k == 6) {
                // shut — a sleeping Nu, and the only value that draws no iris
                for (int256 dx = -3; dx <= 3; dx++) _set(g, ex + dx, ecy, light);
                _set(g, ex - 3, ecy - 1, light);
                _set(g, ex + 3, ecy - 1, light);
                continue;
            }
            for (int256 dy = -3; dy <= 3; dy++) {
                for (int256 dx = -3; dx <= 3; dx++) {
                    if (dx * dx + dy * dy <= 12) _set(g, ex + dx, ecy + dy, lens);
                }
            }
            _pupil(g, ex, ecy, s, k, band);
            _set(g, ex - 2, ecy - 2, glow); // the one specular pixel
        }
    }

    function _pupil(bytes memory g, int256 ex, int256 ecy, int256 s, uint8 k, uint8 band)
        internal
        pure
    {
        if (k == 0) {
            // ⚠ `wide` AND `round` MUST NOT BE THE SAME EYE. A 2x2 pupil beside
            // a 3x3 one is one pixel of difference on each axis, which at 32px
            // is no difference at all — two of seven values reading identically
            // is the dead-trait bug the catalog view exists to catch. `wide` is
            // a SMALL pupil thrown to the outer edge of the lens.
            for (int256 dy; dy < 2; dy++) {
                _set(g, ex + s, ecy + dy, 7);
                _set(g, ex + s * 2, ecy + dy, 7);
            }
        } else if (k == 1) {
            for (int256 dy = -2; dy <= 2; dy++) {
                for (int256 dx = -2; dx <= 2; dx++) {
                    if (dx * dx + dy * dy <= 4) _set(g, ex + dx, ecy + dy, 7);
                }
            }
        } else if (k == 2) {
            for (int256 dy = -3; dy <= 3; dy++) _set(g, ex, ecy + dy, 7);
            for (int256 dy = -2; dy <= 2; dy++) {
                _set(g, ex - 1, ecy + dy, 7);
                _set(g, ex + 1, ecy + dy, 7);
            }
        } else if (k == 3) {
            // ringed — a band in the iris's own darker sibling was invisible on
            // every warm iris, the two being 40 luma apart. A VOID annulus with
            // a lens core reads at any colour.
            for (int256 dy = -3; dy <= 3; dy++) {
                for (int256 dx = -3; dx <= 3; dx++) {
                    int256 d = dx * dx + dy * dy;
                    if (d >= 5 && d <= 9) _set(g, ex + dx, ecy + dy, 7);
                }
            }
            _set(g, ex, ecy, band);
        } else if (k == 4) {
            for (int256 dx = -3; dx <= 3; dx++) _set(g, ex + dx, ecy, 7);
            for (int256 dy = -3; dy <= 3; dy++) _set(g, ex, ecy + dy, 7);
        } else {
            int256[9] memory sx = [int256(0), 1, 2, 2, 1, 0, -1, -1, 0];
            int256[9] memory sy = [int256(-2), -2, -1, 0, 1, 1, 0, -1, 0];
            for (uint256 i; i < 9; i++) _set(g, ex + sx[i], ecy + sy[i], 7);
        }
    }

    /// ⚠ WIDE, FLAT AND LOW. A small mouth turns the creature into a face with a
    /// nose; the Nu's is a long line under both eyes with the corners tucked up,
    /// which is what carries the expression at 32px.
    function _mouth(bytes memory g, int256 cx, int256 my, uint8 sock) internal pure {
        for (int256 dx = -4; dx <= 4; dx++) _set(g, cx + dx, my, sock);
        _set(g, cx - 5, my - 1, sock);
        _set(g, cx + 5, my - 1, sock);
    }

    function _spreadInk(uint8 v) internal pure returns (uint8) {
        if (v == 0) return 1; // paper
        if (v == 1) return 3; // recessed
        if (v == 2) return 7; // ink
        if (v == 3) return 4; // line
        if (v == 4) return 6; // engrave
        if (v == 5) return 9; // gild — the saturated slot, not the dimmed one
        // ⚠ THE RUST GROUND IS THE DEEP RUST (13), NOT THE BRIGHT ONE. #CE422B
        // lands at 105 luma — dead centre — and a 2-tone contour cannot separate
        // from the middle: the dark outline gives 94 and the light one 98, while
        // the invariant wants 100. Grounds want to be deeper than the figure
        // anyway, so the ground takes the deep rust and the bright one stays
        // where it is seen, on the hide.
        if (v == 6) return 13; // rust
        if (v == 7) return 27; // teal ink
        return 28; // violet ink
    }

    /// A rolled iris: (lens, glow, band). The glow is also the AURA's colour, so
    /// a seat's halo matches its own eyes instead of everything being gold; the
    /// band is a darker sibling for the `ringed` variant's core and for an aura
    /// on a pale ground, where a highlight would be invisible.
    function _irisInk(uint8 v) internal pure returns (uint8 lens, uint8 glow, uint8 band) {
        if (v == 0) return (9, 24, 11); // gold — door-gold, still the commonest
        if (v == 1) return (18, 15, 17); // ice
        if (v == 2) return (16, 24, 12); // ember
        if (v == 3) return (22, 15, 21); // malachite
        if (v == 4) return (20, 15, 19); // arcane
        if (v == 5) return (26, 15, 25); // garnet
        return (23, 24, 10); // brass
    }

    /// A hide's two tones, ordered so the BODY tone is the one further from the
    /// ground — and the trait name still tells the truth.
    ///
    /// ⚠ THE HONEST FIX FOR A FILL THAT MATCHES ITS GROUND. `moss` on `engrave`
    /// and `ink` on `ink` are the same slot twice, so the body was painted in
    /// the background colour and the shading did all the work. Substituting some
    /// OTHER hide's colour would make the `hide` attribute a lie, so the hide's
    /// own two tones swap roles instead — an ink creature on ink ground is still
    /// ink, wearing its lighter ink. The swap only fires when it widens the gap,
    /// which is why `bone` on paper does NOT swap: both tones are pale, and
    /// there the dark contour is what separates it.
    function _hideInk(uint8 v, uint8 ground) internal pure returns (uint8 dark, uint8 light) {
        if (v == 0) (dark, light) = (30, 31); // nu — the blue, and the commonest
        else if (v == 1) (dark, light) = (6, 5); // moss
        else if (v == 2) (dark, light) = (7, 8); // ink
        else if (v == 3) (dark, light) = (4, 2); // bone
        else if (v == 4) (dark, light) = (10, 9); // gild
        else if (v == 5) (dark, light) = (12, 5); // rust
        else if (v == 6) (dark, light) = (5, 4); // faint
        // ⚠ BOTH TONES OF THE COLOURED HIDES ARE DEEP, deliberately. Pairing
        // them with bright highlights let the swap below fire on a dark ground
        // and fill 40% of the tile with saturated lilac or sky blue, which
        // out-shouts the eyes and reads as cartoon rather than as an animal.
        else if (v == 7) (dark, light) = (27, 17); // slate
        else if (v == 8) (dark, light) = (14, 4); // verdigris
        else (dark, light) = (28, 19); // plum
        uint256 lg = _lum(ground);
        if (_absDiff(_lum(light), lg) > _absDiff(_lum(dark), lg)) return (light, dark);
    }

    /// The hacker layer, drawn as engraved line and never as glow — and drawn
    /// BOLD. These used to be hairlines at the canvas edge that read as dirt;
    /// gear that breaks the silhouette is what makes a rare visible in a grid.
    function _gear(bytes memory g, int256 cx, uint8 k, int256 wide, int256 top, int256 ecy)
        internal
        pure
    {
        // ⚠ SIDE-MOUNTED GEAR IS CLAMPED, because the widest body is 13 and the
        // canvas has 15 columns to give. The dish reaches four columns past its
        // mast, so on a `squat` or `broad` seat it ran off the edge and rendered
        // as a mast with no dish on it — a 9%-rare trait drawing nothing, which
        // looks exactly like a trait nobody rolled.
        int256 side = _minI(wide, 11);
        if (k == 1) {
            for (int256 x = cx - 9; x <= cx + 9; x++) {
                _set(g, x, ecy - 5, 7);
                _set(g, x, ecy - 4, 9);
            }
            for (int256 x = cx - 9; x <= cx + 9; x += 4) _set(g, x, ecy - 5, 9);
        } else if (k == 2) {
            for (int256 s = -1; s <= 1; s += 2) {
                for (int256 y = ecy - 3; y <= ecy + 3; y++) {
                    for (int256 c; c < 3; c++) _set(g, cx + s * (wide + c), y, c == 0 ? 8 : 7);
                }
                for (int256 y = ecy - 1; y <= ecy + 1; y++) _set(g, cx + s * (wide + 1), y, 9);
            }
            // ⚠ THE BROW BAND RIDES THE FOREHEAD, NOT THE CROWN. At `top - 1` —
            // where the hood used to end and where the SPROUT now starts — a
            // headset silently shaved the tuft off every seat wearing both, and
            // `tiara`, which is only two rows tall, lost it entirely.
            for (int256 x = cx - wide + 2; x < cx + wide - 1; x++) _set(g, x, top + 2, 7);
            for (int256 x = cx - wide + 2; x < cx + wide - 1; x += 3) _set(g, x, top + 2, 9);
        } else if (k == 3) {
            for (int256 y = ecy - 2; y < ecy + 8; y++) {
                _set(g, cx + side + 2, y, 8);
                _set(g, cx + side + 3, y, 7);
            }
            for (int256 i; i < 5; i++) _set(g, cx + side + 3 + i, ecy + 7, 7);
            _set(g, cx + side + 2, ecy - 2, 9);
            _set(g, cx + side + 3, ecy - 2, 9);
        } else if (k == 4) {
            for (int256 y; y < 8; y++) {
                _set(g, cx + wide - 2, y, 8);
                _set(g, cx + wide - 1, y, 7);
            }
            for (int256 i; i < 3; i++) _set(g, cx + wide - 2, i, 9);
            _set(g, cx + wide - 4, 1, 9);
            _set(g, cx + wide, 2, 9);
        } else if (k == 5) {
            // ⚠ Outlined in ink, not filled in tan. A `4` fill (203 luma) on
            // paper (236) made the dish a smudge — the same figure-vs-ground
            // mistake as the hides, one layer up.
            for (int256 dy = -5; dy <= 5; dy++) {
                int256 w = 5 - (dy * dy) / 6;
                for (int256 dx; dx < w; dx++) _set(g, cx + side + dx, 8 + dy, 8);
                _set(g, cx + side + w - 1, 8 + dy, 7);
                if (dy == -5 || dy == 5) {
                    for (int256 dx; dx < w; dx++) _set(g, cx + side + dx, 8 + dy, 7);
                }
            }
            for (int256 y = 8; y < 17; y++) _set(g, cx + side, y, 7);
            for (int256 dy = -1; dy <= 1; dy++) _set(g, cx + side + 2, 8 + dy, 9);
        }
    }

    /// Behind the figure, in the corners a blob leaves open. These used to be
    /// two or three pixels in a thin wisp above the crown and read as dust.
    function _aura(
        bytes memory g,
        int256 cx,
        uint8 k,
        int256 top,
        uint8 sprout,
        uint8 ground,
        uint8 lit,
        uint8 band
    ) internal pure {
        if (k == 0) return;
        // ⚠ THE AURA TAKES THE SEAT'S OWN EYE COLOUR. It was gold-or-gild-ink off
        // the ground's luma, so a seat with ice eyes wore a gold halo and read
        // as two unrelated ideas. On a pale ground the darker band carries it,
        // because a highlight on paper is invisible.
        uint8 glow = _lum(ground) < 96 ? lit : band;
        if (k == 1) {
            for (int256 i; i < 6; i++) {
                int256 y = top - 1 - i;
                if (y < 0) break;
                int256 sp = 4 + i;
                int256[6] memory dxs = [-sp, -sp + 1, -sp + 2, sp - 2, sp - 1, sp];
                for (uint256 j; j < 6; j++) _set(g, cx + dxs[j] + (i % 2), y, i % 2 == 1 ? 8 : 5);
            }
        } else if (k == 2) {
            int256[8] memory px = [int256(-9), -11, 9, 11, -8, 10, -12, 12];
            int256[8] memory py = [int256(3), 7, 2, 6, 11, 11, 14, 13];
            for (uint256 i; i < 8; i++) {
                _set(g, cx + px[i], py[i], glow);
                _set(g, cx + px[i], py[i] + 1, glow);
            }
        } else if (k == 3) {
            int256[6] memory px = [int256(-10), 10, -12, 12, -11, 11];
            int256[6] memory py = [int256(4), 4, 9, 9, 14, 14];
            for (uint256 i; i < 6; i++) {
                for (uint256 c; c < 3; c++) {
                    _set(g, cx + px[i], py[i] + int256(c), (i + c) % 2 == 1 ? glow : 5);
                }
            }
        } else if (k == 4) {
            // ⚠ A HALO CLEARS THE SPROUT, NOT THE SKULL, and that is why this is
            // computed. Anchored three rows over the crown — where it sat for
            // the hooded figure, which had nothing on its head — it was buried:
            // the creature is ONE dilated mask, so a tuft and its 2px outline
            // already own every row above the head.
            int256 hy = top - _sproutH(sprout) - 2;
            if (hy < 1) hy = 1;
            for (int256 dx = -12; dx <= 12; dx++) {
                int256 dd = (dx * dx) / 14;
                int256 adx = dx < 0 ? -dx : dx;
                _set(g, cx + dx, hy + dd, glow);
                if (adx > 6) _set(g, cx + dx, hy + dd + 1, band);
            }
        }
    }

    /// The belly mark's inks, DERIVED against the hide they sit on.
    ///
    /// ⚠ A FIXED COLOUR LAID ON A ROLLED ONE always has combinations where it
    /// disappears — measured at 2,364 seats, 28.9% of the run, when door-gold
    /// sat on a gold robe at a gap of zero. Door-gold is kept wherever it reads,
    /// because it is the collection's through-line; only the mid-tones that
    /// collide fall back to engraved ink, which is the more woodcut answer.
    function _markInk(uint8 fill)
        internal
        pure
        returns (uint8 bright, uint8 dim, uint8 hot, uint8 core)
    {
        if (_absDiff(_lum(fill), _lum(9)) >= 45) return (9, 11, 12, 7);
        return (7, 6, 6, 1);
    }

    /// On the belly, and reading as an OBJECT the creature is wearing.
    function _mark(bytes memory g, int256 cx, uint8 k, uint8 fill, int256 my) internal pure {
        if (k == 0) return;
        (uint8 bright, uint8 dim, uint8 hot, uint8 core) = _markInk(fill);
        if (k == 1) {
            for (int256 dy = -3; dy <= 3; dy++) {
                int256 ay = dy < 0 ? -dy : dy;
                for (int256 dx = -4; dx <= 4; dx++) {
                    int256 ax = dx < 0 ? -dx : dx;
                    if (ax + ay * 2 <= 5) _set(g, cx + dx, my + dy, bright);
                    if (ax + ay * 2 <= 2) _set(g, cx + dx, my + dy, dim);
                }
            }
            _set(g, cx - 4, my, dim);
            _set(g, cx + 4, my, dim);
            _set(g, cx, my, core);
        } else if (k == 2) {
            for (int256 dx = -8; dx <= 8; dx++) _set(g, cx + dx, my, bright);
            for (int256 dx = -6; dx <= 6; dx += 4) {
                _set(g, cx + dx, my - 2, bright);
                _set(g, cx + dx, my + 2, bright);
                _set(g, cx + dx, my + 3, dim);
            }
        } else if (k == 3) {
            for (int256 dy = -3; dy <= 3; dy++) {
                int256 ay = dy < 0 ? -dy : dy;
                for (int256 dx = -3; dx <= 3; dx++) {
                    int256 ax = dx < 0 ? -dx : dx;
                    if (ax + ay == 3) _set(g, cx + dx, my + dy, bright);
                    else if (ax + ay == 1) _set(g, cx + dx, my + dy, dim);
                }
            }
        } else if (k == 4) {
            for (int256 dy = -3; dy <= 3; dy++) {
                for (int256 dx = -3; dx <= 3; dx++) {
                    if (dx * dx + dy * dy <= 9) _set(g, cx + dx, my + dy, hot);
                    if (dx * dx + dy * dy <= 3) _set(g, cx + dx, my + dy, bright);
                }
            }
        } else if (k == 5) {
            uint256 i;
            for (int256 dx = -7; dx <= 7; dx += 3) {
                for (int256 dy = -3; dy <= 3; dy++) {
                    _set(g, cx + dx, my + dy, i % 2 == 1 ? bright : dim);
                }
                i++;
            }
        }
    }

    function _frame(bytes memory g, uint8 k, uint8 ground) internal pure {
        if (k == 0) return;
        uint8 ink = k == 1 ? 6 : k == 3 ? 9 : 7;
        if (_lum(ground) < 96 && k != 3) ink = 4;
        for (uint256 i; i < S; i++) {
            g[i] = bytes1(ink);
            g[(S - 1) * S + i] = bytes1(ink);
            g[i * S] = bytes1(ink);
            g[i * S + S - 1] = bytes1(ink);
        }
        if (k == 2) {
            uint256[4] memory e = [uint256(0), 1, S - 2, S - 1];
            for (uint256 a; a < 4; a++) {
                for (uint256 b; b < 4; b++) g[e[a] * S + e[b]] = bytes1(uint8(9));
            }
        }
        if (k == 4) {
            for (uint256 i = 2; i < S - 2; i++) {
                g[2 * S + i] = bytes1(ink);
                g[(S - 3) * S + i] = bytes1(ink);
                g[i * S + 2] = bytes1(ink);
                g[i * S + S - 3] = bytes1(ink);
            }
        }
    }

    // ── the SVG ──────────────────────────────────────────────────────────────
    /// One <rect> per horizontal run of one colour.
    ///
    /// ⚠ WRITTEN INTO A PREALLOCATED BUFFER, NOT BUILT WITH `abi.encodePacked`
    /// IN A LOOP — the difference is whether this is callable. Concatenating
    /// reallocates and copies the WHOLE string every rect, which is quadratic
    /// in copying and in memory expansion: measured at 63 M gas. The common
    /// `eth_call` cap is 50 M, so that was not "expensive", it was a token
    /// wallets could not display.
    function svg(uint256 tokenId, string memory salt) public view returns (string memory) {
        return svgOf(grid(tokenId, salt));
    }

    function svgOf(bytes memory g) public view returns (string memory) {
        bytes memory buf = new bytes(64_000);
        uint256 n = _put(
            buf,
            0,
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" '
            'shape-rendering="crispEdges" width="512" height="512">'
        );
        for (uint256 y; y < S; y++) {
            uint256 x;
            while (x < S) {
                uint8 c = uint8(g[y * S + x]);
                uint256 run = 1;
                while (x + run < S && uint8(g[y * S + x + run]) == c) run++;
                if (c != 0) {
                    n = _put(buf, n, '<rect x="');
                    n = _putU(buf, n, x);
                    n = _put(buf, n, '" y="');
                    n = _putU(buf, n, y);
                    n = _put(buf, n, '" width="');
                    n = _putU(buf, n, run);
                    n = _put(buf, n, '" height="1" fill="#');
                    n = _put(buf, n, bytes(_hex(palette(c))));
                    n = _put(buf, n, '"/>');
                }
                x += run;
            }
        }
        n = _put(buf, n, "</svg>");
        return string(_take(buf, n));
    }

    /// Identical for every seat, on purpose: a pre-reveal face that varied by
    /// token would leak the deck it exists to hide.
    ///
    /// It is a Nu in silhouette with its sprout — enough to say what the
    /// collection is, showing no eyes, no hide colour and no gear. The tuft is
    /// GREEN because that is the one thing every seat has; the body is the void
    /// because a colour here would suggest a hide a given seat may not roll.
    function sealedSvg() public pure returns (string memory) {
        return string(
            abi.encodePacked(
                '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" ',
                'shape-rendering="crispEdges" width="512" height="512">',
                '<rect width="32" height="32" fill="#1e4a22"/>',
                '<rect x="14" y="3" width="4" height="4" fill="#77bb44"/>',
                '<rect x="11" y="5" width="10" height="2" fill="#77bb44"/>',
                '<rect x="8" y="7" width="16" height="4" fill="#0a0c0a"/>',
                '<rect x="6" y="11" width="20" height="12" fill="#0a0c0a"/>',
                '<rect x="8" y="23" width="16" height="3" fill="#0a0c0a"/>',
                '<rect x="8" y="26" width="5" height="3" fill="#0a0c0a"/>',
                '<rect x="19" y="26" width="5" height="3" fill="#0a0c0a"/>',
                "</svg>"
            )
        );
    }

    // ── tokenURI ─────────────────────────────────────────────────────────────
    /// @param salt `ScrySeat.revealedSalt()` — empty until the run closes.
    /// @param tier emitted as a NUMERIC stat, not a categorical trait, and that
    ///        is deliberate: tier is BUYABLE, so ranking it beside the sealed
    ///        cosmetic traits would let a marketplace's rarity tool report a
    ///        paid-for number as rarity.
    function tokenURI(uint256 tokenId, uint8 tier, uint8 door, string calldata salt)
        external
        view
        returns (string memory)
    {
        bool revealed = bytes(salt).length > 0;
        string memory image = revealed ? svg(tokenId, salt) : sealedSvg();

        bytes memory attrs = abi.encodePacked(
            '{"trait_type":"door","value":"',
            _door(door),
            '"},{"trait_type":"reveal","value":"',
            revealed ? "open" : "sealed",
            '"},{"display_type":"number","trait_type":"tier","value":',
            _u(tier),
            "}"
        );
        if (revealed) {
            uint8[N_LAYERS] memory t = traitsOf(tokenId, salt);
            for (uint8 i; i < N_LAYERS; i++) {
                attrs = abi.encodePacked(
                    attrs, ',{"trait_type":"', layerName(i), '","value":"', names(i, t[i]), '"}'
                );
            }
        }

        bytes memory json = abi.encodePacked(
            '{"name":"',
            collectionName,
            " #",
            _u(tokenId),
            '","description":"',
            collectionDescription,
            '","image":"data:image/svg+xml;base64,',
            _b64(bytes(image)),
            '","attributes":[',
            attrs,
            "]}"
        );
        return string(abi.encodePacked("data:application/json;base64,", _b64(json)));
    }

    /// ERC-7572 — inline like everything else here.
    /// The collection's own mark: a hand-picked seat, drawn by the same renderer.
    ///
    /// ⚠ NOT `sealedSvg()`, which is what `contractURI` used to serve. The sealed
    /// card is a placeholder that exists to hide the deck for the length of the
    /// mint; handing it to a marketplace as the COLLECTION image left the store
    /// front showing a placeholder, before the reveal and forever after, next to
    /// collections with real logos. The vector is constant and hand-picked, so
    /// it discloses nothing about any token.
    ///
    /// A blue Nu on near-black with a palm tuft, haloed, a ward on the belly.
    function emblemTraits() public pure returns (uint8[N_LAYERS] memory t) {
        t[L_SPREAD] = 2; // ink — near-black, so the blue and the gold carry
        t[L_FRAME] = 0; // none
        t[L_HIDE] = 0; // nu — the collection's own blue
        t[L_BODY] = 0; // round
        t[L_EYE] = 0; // wide
        t[L_GEAR] = 0; // none — the mark carries it, not hardware
        t[L_MARK] = 3; // ward
        t[L_AURA] = 4; // halo
        t[L_IRIS] = 0; // gold — door-gold is the collection's through-line
        t[L_SPROUT] = 1; // palm — the tuft that says what this is
    }

    function emblemSvg() public view returns (string memory) {
        return svgOf(gridOf(emblemTraits()));
    }

    function contractURI() external view returns (string memory) {
        bytes memory json = abi.encodePacked(
            '{"name":"',
            collectionName,
            '","description":"',
            collectionDescription,
            '","external_link":"https://scry.moreright.xyz/mint.html"',
            ',"image":"data:image/svg+xml;base64,',
            _b64(bytes(emblemSvg())),
            '"}'
        );
        return string(abi.encodePacked("data:application/json;base64,", _b64(json)));
    }

    // ── reads for the site + the parity test ─────────────────────────────────
    function layerVariants(uint8 i)
        external
        pure
        returns (string[] memory ns, uint16[] memory ws)
    {
        ws = weights(i);
        ns = new string[](ws.length);
        for (uint8 v; v < ws.length; v++) ns[v] = names(i, v);
    }

    /// Every weight row sums to exactly 10,000 bps. A test asserts this rather
    /// than a comment claiming it.
    function layerSumsAreExact() external pure returns (bool) {
        for (uint8 i; i < N_LAYERS; i++) {
            uint16[] memory w = weights(i);
            uint256 s;
            for (uint256 j; j < w.length; j++) s += w[j];
            if (s != 10_000) return false;
        }
        return true;
    }

    // ── helpers ──────────────────────────────────────────────────────────────
    function _door(uint8 d) internal pure returns (string memory) {
        if (d == 1) return "snapshot";
        if (d == 2) return "play";
        if (d == 3) return "build";
        if (d == 4) return "paid";
        if (d == 5) return "treasury";
        return "unknown";
    }

    function _put(bytes memory b, uint256 n, bytes memory s) internal pure returns (uint256) {
        for (uint256 i; i < s.length; i++) b[n + i] = s[i];
        return n + s.length;
    }

    function _putU(bytes memory b, uint256 n, uint256 v) internal pure returns (uint256) {
        if (v == 0) {
            b[n] = "0";
            return n + 1;
        }
        uint256 len;
        for (uint256 t = v; t != 0; t /= 10) len++;
        for (uint256 i = len; i > 0; i--) {
            b[n + i - 1] = bytes1(uint8(48 + (v % 10)));
            v /= 10;
        }
        return n + len;
    }

    function _take(bytes memory b, uint256 n) internal pure returns (bytes memory o) {
        o = new bytes(n);
        for (uint256 i; i < n; i++) o[i] = b[i];
    }

    function _u(uint256 v) internal pure returns (string memory) {
        if (v == 0) return "0";
        uint256 n = v;
        uint256 len;
        while (n != 0) {
            len++;
            n /= 10;
        }
        bytes memory s = new bytes(len);
        while (v != 0) {
            s[--len] = bytes1(uint8(48 + (v % 10)));
            v /= 10;
        }
        return string(s);
    }

    function _hex(bytes3 c) internal pure returns (string memory) {
        bytes memory H = "0123456789abcdef";
        bytes memory o = new bytes(6);
        for (uint256 i; i < 3; i++) {
            o[i * 2] = H[uint8(c[i]) >> 4];
            o[i * 2 + 1] = H[uint8(c[i]) & 0x0F];
        }
        return string(o);
    }

    /// OpenZeppelin's `Base64.encode` (MIT), inlined rather than vendored
    /// because this repo carries its own minimal primitives instead of the OZ
    /// tree. ⚠ Not a micro-optimisation: the readable byte-loop version cost
    /// 24.4 M gas across the two encoding passes a token needs — more than
    /// drawing the whole picture.
    function _b64(bytes memory data) internal pure returns (string memory) {
        if (data.length == 0) return "";
        string memory table = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        string memory result = new string(4 * ((data.length + 2) / 3));
        assembly {
            let tablePtr := add(table, 1)
            let resultPtr := add(result, 32)
            for {
                let dataPtr := data
                let endPtr := add(data, mload(data))
            } lt(dataPtr, endPtr) {} {
                dataPtr := add(dataPtr, 3)
                let input := mload(dataPtr)
                mstore8(resultPtr, mload(add(tablePtr, and(shr(18, input), 0x3F))))
                resultPtr := add(resultPtr, 1)
                mstore8(resultPtr, mload(add(tablePtr, and(shr(12, input), 0x3F))))
                resultPtr := add(resultPtr, 1)
                mstore8(resultPtr, mload(add(tablePtr, and(shr(6, input), 0x3F))))
                resultPtr := add(resultPtr, 1)
                mstore8(resultPtr, mload(add(tablePtr, and(input, 0x3F))))
                resultPtr := add(resultPtr, 1)
            }
            switch mod(mload(data), 3)
            case 1 {
                mstore8(sub(resultPtr, 1), 0x3d)
                mstore8(sub(resultPtr, 2), 0x3d)
            }
            case 2 { mstore8(sub(resultPtr, 1), 0x3d) }
        }
        return result;
    }
}

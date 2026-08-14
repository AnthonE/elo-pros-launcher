// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryGameTicket.sol";
import "../src/ScryFeeSplitter.sol";
import "./MockToken.sol";
import "./Base64Decode.sol";

/// Stands in for a title that outgrows the built-in document. `setRenderer(0)`
/// must always take the built-in back — that is the posted door out.
contract FakeRenderer {
    function tokenURI(uint256) external pure returns (string memory) {
        return "renderer://token";
    }

    function contractURI() external pure returns (string memory) {
        return "renderer://collection";
    }
}

/// The ticket is the copy is the licence (`GATES.md` §4 rev. 2026-08-08).
/// What these pin, by section:
///   - the three paid rails: exact amounts, closed-at-zero, and WHERE the
///     money lands (SCRY → the splitter in one hop, USDG → proceeds, ETH →
///     sweep-to-proceeds by anyone)
///   - the half-burn is proven against the REAL ScryFeeSplitter at the
///     posted 5000/4000/1000 split — not asserted from a comment
///   - the two free doors: comp on the record, merkle claims bounded by a
///     posted allowance that never resets
///   - entitled() — the one question the depot asks
///   - the royalty is a constant zero and ownership renounce freezes knobs
contract ScryGameTicketTest is Test {
    ScryGameTicket t;
    MockToken scry;
    MockToken usdg;
    ScryFeeSplitter splitter;

    address bank = address(0xB4A2);
    address ops = address(0x0125);
    address proceeds = address(0xDE7);
    address buyer = address(0xB07E5);
    address stranger = address(0x57A);

    uint256 constant PRICE_WEI = 0.003 ether;
    uint256 constant PRICE_SCRY = 700_000e18;
    uint256 constant PRICE_USDG = 10e6; // USDG runs 6 decimals on 4663

    function setUp() public {
        scry = new MockToken("Scry", "SCRY");
        usdg = new MockToken("USD Glo", "USDG");

        address[] memory to = new address[](3);
        uint16[] memory bps = new uint16[](3);
        to[0] = SCRY_BURN;
        bps[0] = 5000;
        to[1] = bank;
        bps[1] = 4000;
        to[2] = ops;
        bps[2] = 1000;
        splitter = new ScryFeeSplitter(IERC20(address(scry)), to, bps);

        t = new ScryGameTicket(
            "Gates", "GATES", "gates", IERC20(address(scry)), IERC20(address(usdg)),
            address(splitter), proceeds, 0, "https://scry.moreright.xyz", address(0)
        );
        t.setPrices(1000, PRICE_WEI, PRICE_SCRY, PRICE_USDG);

        vm.deal(buyer, 1 ether);
        scry.mint(buyer, PRICE_SCRY * 3);
        usdg.mint(buyer, PRICE_USDG * 3);
    }

    // ── the ETH rail ─────────────────────────────────────────────────────────
    function test_eth_buy_mints_at_the_exact_posted_amount() public {
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        assertEq(t.ownerOf(id), buyer);
        assertEq(t.railOf(id), 1);
        assertTrue(t.entitled(buyer));
    }

    function test_eth_buy_refuses_over_and_underpayment() public {
        vm.startPrank(buyer);
        vm.expectRevert(bytes("wrong payment"));
        t.buyWithETH{value: PRICE_WEI - 1}();
        vm.expectRevert(bytes("wrong payment"));
        t.buyWithETH{value: PRICE_WEI + 1}();
        vm.stopPrank();
    }

    function test_eth_accumulates_and_anyone_sweeps_to_proceeds_only() public {
        vm.prank(buyer);
        t.buyWithETH{value: PRICE_WEI}();
        assertEq(address(t).balance, PRICE_WEI);
        vm.prank(stranger); // sweep is permissionless and cannot redirect
        t.sweep();
        assertEq(proceeds.balance, PRICE_WEI);
        assertEq(address(t).balance, 0);
    }

    function test_sweep_with_nothing_refuses() public {
        vm.expectRevert(bytes("nothing to sweep"));
        t.sweep();
    }

    // ── the SCRY rail: one hop to the splitter, half burns for real ──────────
    function test_scry_buy_moves_the_coin_straight_to_the_splitter() public {
        vm.startPrank(buyer);
        scry.approve(address(t), PRICE_SCRY);
        uint256 id = t.buyWithSCRY();
        vm.stopPrank();
        assertEq(t.ownerOf(id), buyer);
        assertEq(t.railOf(id), 2);
        assertEq(scry.balanceOf(address(t)), 0); // the ticket never holds SCRY
        assertEq(scry.balanceOf(address(splitter)), PRICE_SCRY);
    }

    function test_half_of_a_scry_buy_burns_under_the_posted_split() public {
        vm.startPrank(buyer);
        scry.approve(address(t), PRICE_SCRY);
        t.buyWithSCRY();
        vm.stopPrank();
        splitter.distribute(); // permissionless — the burn is real only after this runs
        assertEq(scry.balanceOf(SCRY_BURN), PRICE_SCRY / 2);
        assertEq(scry.balanceOf(bank), PRICE_SCRY * 4000 / 10_000);
        assertEq(scry.balanceOf(ops), PRICE_SCRY - PRICE_SCRY / 2 - PRICE_SCRY * 4000 / 10_000);
        assertEq(splitter.totalBurned(), PRICE_SCRY / 2);
    }

    function test_scry_buy_without_approval_refuses() public {
        vm.prank(buyer);
        vm.expectRevert();
        t.buyWithSCRY();
    }

    // ── the USDG rail ────────────────────────────────────────────────────────
    function test_usdg_buy_lands_on_proceeds_in_one_hop() public {
        vm.startPrank(buyer);
        usdg.approve(address(t), PRICE_USDG);
        uint256 id = t.buyWithUSDG();
        vm.stopPrank();
        assertEq(t.railOf(id), 3);
        assertEq(usdg.balanceOf(proceeds), PRICE_USDG);
        assertEq(usdg.balanceOf(address(t)), 0);
    }

    // ── closed rails ─────────────────────────────────────────────────────────
    function test_a_zero_amount_closes_that_rail_alone() public {
        t.setPrices(1000, 0, PRICE_SCRY, PRICE_USDG);
        vm.startPrank(buyer);
        vm.expectRevert(bytes("rail closed"));
        t.buyWithETH{value: PRICE_WEI}();
        scry.approve(address(t), PRICE_SCRY);
        t.buyWithSCRY(); // the other rails stay open
        vm.stopPrank();
    }

    function test_all_three_at_zero_close_the_sale() public {
        t.setPrices(0, 0, 0, 0);
        vm.startPrank(buyer);
        vm.expectRevert(bytes("rail closed"));
        t.buyWithETH{value: 0}();
        vm.expectRevert(bytes("rail closed"));
        t.buyWithSCRY();
        vm.expectRevert(bytes("rail closed"));
        t.buyWithUSDG();
        vm.stopPrank();
    }

    // ── free copies: on the record, or on a posted list ──────────────────────
    function test_comp_mint_is_the_owners_and_entitles() public {
        t.compMint(stranger, 2);
        assertEq(t.balanceOf(stranger), 2);
        assertEq(t.railOf(1), 4);
        assertTrue(t.entitled(stranger));
        vm.prank(stranger);
        vm.expectRevert(bytes("not owner"));
        t.compMint(stranger, 1);
    }

    function _leaf(address who, uint256 allowance) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(who, allowance));
    }

    function _pair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        return a <= b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
    }

    function test_free_claim_is_bounded_by_the_posted_allowance_forever() public {
        // a two-leaf tree: buyer may claim 1, stranger may claim 2
        bytes32 la = _leaf(buyer, 1);
        bytes32 lb = _leaf(stranger, 2);
        t.setFreeRoot(_pair(la, lb));

        bytes32[] memory proofA = new bytes32[](1);
        proofA[0] = lb;
        vm.prank(buyer);
        uint256 id = t.claimFree(1, proofA);
        assertEq(t.railOf(id), 5);
        vm.prank(buyer);
        vm.expectRevert(bytes("already claimed yours"));
        t.claimFree(1, proofA);

        // republishing the SAME wallet at the same allowance re-opens nothing
        t.setFreeRoot(_pair(la, lb));
        vm.prank(buyer);
        vm.expectRevert(bytes("already claimed yours"));
        t.claimFree(1, proofA);
    }

    function test_free_claim_refuses_a_forged_membership() public {
        bytes32 la = _leaf(buyer, 1);
        bytes32 lb = _leaf(stranger, 2);
        t.setFreeRoot(_pair(la, lb));
        bytes32[] memory proofA = new bytes32[](1);
        proofA[0] = lb;
        vm.prank(buyer);
        vm.expectRevert(bytes("not on the free list"));
        t.claimFree(2, proofA); // right wallet, inflated allowance
        vm.prank(stranger);
        vm.expectRevert(bytes("not on the free list"));
        t.claimFree(1, proofA); // wrong wallet on buyer's proof
    }

    function test_a_cleared_root_closes_the_free_door() public {
        t.setFreeRoot(bytes32(0));
        bytes32[] memory none = new bytes32[](0);
        vm.prank(buyer);
        vm.expectRevert(bytes("free door closed"));
        t.claimFree(1, none);
    }

    /// THE CROSS-LANGUAGE VECTOR (2026-08-10).
    ///
    /// Every other free-door test above builds its tree in Solidity, so all of
    /// them would still pass if the OFF-CHAIN builder disagreed with this
    /// contract about the encoding. The builder is what actually posts a root:
    /// `meter/free_copies.py` composes a holder cohort into `freeRoot` and the
    /// per-wallet proofs the origin serves. If the two ever drift, the first
    /// symptom is a holder clicking *claim your free copy* and getting `not on
    /// the free list` — the worst possible place to discover it.
    ///
    /// So the numbers below are PINNED OUTPUT from that builder, not computed
    /// here. Four wallets, allowances 1/1/2/1, sorted by address (the builder
    /// sorts, which is what makes a root reproducible from public snapshots).
    /// Regenerate with:
    ///
    ///   python3 meter/free_copies.py build --game gates --allowlist <cohort>
    ///
    /// A change to `_leaf`, to the sort, or to the sorted-pair rule on either
    /// side turns this red. That is the entire job.
    function test_a_root_built_off_chain_by_free_copies_py_verifies_here() public {
        address a1 = 0x00000000000000000000000000000000000000A1;
        address b2 = 0x00000000000000000000000000000000000000b2;
        address c3 = 0x00000000000000000000000000000000000000C3;
        address d4 = 0x00000000000000000000000000000000000000D4;

        t.setFreeRoot(0xae87da8b5220cd5677558c2c0f95cd41f92d7908b748982f5f0a4d2582384c2b);

        bytes32[] memory p1 = new bytes32[](2);
        p1[0] = 0xdccbb842812100f47f450aa3af879fc4962acf8644304e922be85dddae6efb76;
        p1[1] = 0xaa2f182dcb1702412f7e648d7ef8e4e7831df83bb0cffa331c9b1731ffba6f1c;
        vm.prank(a1);
        assertEq(t.railOf(t.claimFree(1, p1)), 5, "a1 claims on a python-built proof");

        bytes32[] memory p3 = new bytes32[](2);
        p3[0] = 0x42f50381948776ec16bff25205e25d50e47ed5ff93fd7bdc01a8b9205ed9a760;
        p3[1] = 0x744fb05849b8025777b3125b5cc426f52c0102df6a8ef1fcc8445131a4b5bf29;
        // c3's allowance is 2 in the pinned cohort, so it claims twice and stops
        vm.prank(c3);
        t.claimFree(2, p3);
        vm.prank(c3);
        t.claimFree(2, p3);
        vm.prank(c3);
        vm.expectRevert(bytes("already claimed yours"));
        t.claimFree(2, p3);

        // and the allowance in the leaf is load-bearing: c3's own proof at the
        // wrong count is a different leaf, so it is simply not in the tree
        bytes32[] memory p2 = new bytes32[](2);
        p2[0] = 0xd7d4e4b823e955a9c09e7ccb2e990a82dec59385d2eb23beecdc01071bae1352;
        p2[1] = 0xaa2f182dcb1702412f7e648d7ef8e4e7831df83bb0cffa331c9b1731ffba6f1c;
        vm.prank(b2);
        vm.expectRevert(bytes("not on the free list"));
        t.claimFree(2, p2); // b2 is listed for 1, claiming 2
        vm.prank(b2);
        assertEq(t.railOf(t.claimFree(1, p2)), 5, "b2 at its posted allowance");

        // d4 is in the same tree and unaffected by the others' claims
        bytes32[] memory p4 = new bytes32[](2);
        p4[0] = 0xea02820cddb244be5c903b48a739de5c2d19e0ac72de0cc071a32f9012a47508;
        p4[1] = 0x744fb05849b8025777b3125b5cc426f52c0102df6a8ef1fcc8445131a4b5bf29;
        vm.prank(d4);
        assertEq(t.railOf(t.claimFree(1, p4)), 5, "d4 claims independently");
    }

    // ── the licence properties ───────────────────────────────────────────────
    function test_entitled_follows_the_token_not_the_buyer() public {
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        vm.prank(buyer);
        t.transferFrom(buyer, stranger, id);
        assertFalse(t.entitled(buyer)); // sold the copy, lost the licence
        assertTrue(t.entitled(stranger)); // resale is the point
    }

    function test_royalty_is_a_constant_zero() public {
        (address to, uint256 cut) = t.royaltyInfo(1, 1 ether);
        assertEq(to, proceeds);
        assertEq(cut, 0);
    }

    function test_supply_cap_binds_every_door_when_set() public {
        ScryGameTicket capped = new ScryGameTicket(
            "Gates", "GATES", "gates", IERC20(address(scry)), IERC20(address(usdg)),
            address(splitter), proceeds, 2, "https://scry.moreright.xyz", address(0)
        );
        capped.setPrices(1000, PRICE_WEI, 0, 0);
        MockNFT hive = new MockNFT();
        hive.mint(buyer, 7);
        capped.setFreeCollection(address(hive), 1);
        capped.compMint(stranger, 2);
        vm.prank(buyer);
        vm.expectRevert(bytes("supply cap reached"));
        capped.buyWithETH{value: PRICE_WEI}();
        // "every door" includes the collection door — an eligibility check that
        // passes does not make a mint the cap forbids
        vm.prank(buyer);
        vm.expectRevert(bytes("supply cap reached"));
        capped.claimFreeFor(address(hive), _ids(7));
    }

    function test_renounce_freezes_every_knob_at_its_posted_value() public {
        t.transferOwnership(address(0));
        vm.expectRevert(bytes("not owner"));
        t.setPrices(1000, 1, 1, 1);
        vm.expectRevert(bytes("not owner"));
        t.setFreeRoot(bytes32(uint256(1)));
        vm.expectRevert(bytes("not owner"));
        t.setFreeCollection(address(0xBEEF), 1);
        // the posted rails keep selling at their frozen prices
        vm.prank(buyer);
        t.buyWithETH{value: PRICE_WEI}();
    }

    function test_prices_move_in_one_call_and_are_posted_by_event() public {
        vm.expectEmit(false, false, false, true);
        emit ScryGameTicket.PricesPosted(1000, 1, 2, 3);
        t.setPrices(1000, 1, 2, 3);
        assertEq(t.priceUsdCents(), 1000);
        assertEq(t.priceWei(), 1);
        assertEq(t.priceScry(), 2);
        assertEq(t.priceUsdg(), 3);
    }

    // ── metadata: on chain, so the copy outlives the server ──────────────────
    //
    // These used to assert two urls on scry's own origin. That was the whole
    // bug: the one bearer asset here rendered its name and image through our
    // web server, and that route HAS served `{}` for a week with every suite
    // green. What is asserted now is that a viewer needs nothing from us.

    function _json(string memory uri) internal pure returns (string memory) {
        bytes memory b = bytes(uri);
        bytes memory prefix = bytes("data:application/json;base64,");
        require(b.length > prefix.length, "not a data uri");
        bytes memory tail = new bytes(b.length - prefix.length);
        for (uint256 i; i < tail.length; ++i) {
            tail[i] = b[i + prefix.length];
        }
        return string(Base64Decode.decode(string(tail)));
    }

    function test_tokenURI_is_a_self_contained_document_needing_no_server() public {
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        string memory uri = t.tokenURI(id);
        assertTrue(
            _startsWith(uri, "data:application/json;base64,"),
            "a copy must render with the meter offline - no http, no gateway"
        );
        string memory json = _json(uri);
        assertTrue(_contains(json, '"name":"Gates #1"'), json);
        assertTrue(_contains(json, '"trait_type":"game","value":"gates"'), json);
        assertTrue(_contains(json, '"trait_type":"acquired","value":"bought"'), json);
        // the image falls back to the built-in mark, so nothing renders blank
        assertTrue(_contains(json, '"image":"data:image/svg+xml;base64,'), json);
        // baseURI is the SITE base and appears in exactly one field
        assertTrue(_contains(json, '"external_url":"https://scry.moreright.xyz/game.html?slug=gates"'), json);
    }

    function test_contractURI_is_self_contained_too() public view {
        string memory json = _json(t.contractURI());
        assertTrue(_contains(json, '"name":"Gates on Scryward"'), json);
        assertTrue(_contains(json, '"image":"data:image/svg+xml;base64,'), json);
    }

    /// ⚠ THE TWO DOCUMENTS SPELL THE LINK DIFFERENTLY AND THIS IS THE ONLY
    /// CHECK THAT SAYS SO. A collection is ERC-7572 (`external_link`) and a
    /// token is ERC-721 metadata (`external_url`); one shared helper emitted
    /// `external_url` for both, so the collection page's link home was a key
    /// no marketplace reads. Nothing caught it: the document still parsed, the
    /// page just rendered one field short, and the test above asserted only
    /// the name — the "assert the field the user sees" trap, in a suite that
    /// was otherwise green. Assert the key, not merely that a link is present.
    function test_the_collection_and_the_token_each_use_their_own_link_key() public {
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        string memory collection = _json(t.contractURI());
        string memory token = _json(t.tokenURI(id));
        string memory url = "https://scry.moreright.xyz/game.html?slug=gates";

        assertTrue(_contains(collection, string.concat('"external_link":"', url, '"')), collection);
        assertFalse(_contains(collection, '"external_url"'), "ERC-7572 has no external_url");

        assertTrue(_contains(token, string.concat('"external_url":"', url, '"')), token);
        assertFalse(_contains(token, '"external_link"'), "ERC-721 metadata has no external_link");
    }

    /// The link is the one field that leans on `baseURI`, so an unset base has
    /// to drop it rather than emit an empty or malformed one — in both
    /// documents, which is the half a shared helper makes easy to forget.
    function test_no_base_drops_the_link_from_both_documents() public {
        t.setBaseURI("");
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        assertFalse(_contains(_json(t.contractURI()), "external"), "no base, no link");
        assertFalse(_contains(_json(t.tokenURI(id)), "external"), "no base, no link");
    }

    function test_owner_posts_the_name_and_art_a_marketplace_shows() public {
        t.setMetadata("Wake with nothing on a hostile island.", "ipfs://bafyfake/capsule.png");
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        string memory json = _json(t.tokenURI(id));
        assertTrue(_contains(json, '"description":"Wake with nothing on a hostile island."'), json);
        assertTrue(_contains(json, '"image":"ipfs://bafyfake/capsule.png"'), json);
        assertFalse(_contains(json, "data:image/svg+xml"), "a posted image replaces the fallback mark");
    }

    function test_a_quote_in_posted_text_cannot_break_the_document() public {
        // The failure this guards is silent: a bare quote makes a document no
        // wallet can parse, and nothing on chain would revert to say so.
        t.setMetadata('a "survival" game \\ etc', "");
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        string memory json = _json(t.tokenURI(id));
        assertTrue(_contains(json, 'a \\"survival\\" game \\\\ etc'), json);
    }

    function test_posted_text_is_bounded_because_the_view_is_rebuilt_every_read() public {
        bytes memory tooLong = new bytes(1025);
        for (uint256 i; i < tooLong.length; ++i) {
            tooLong[i] = "a";
        }
        vm.expectRevert(bytes("blurb <= 1024"));
        t.setMetadata(string(tooLong), "");
    }

    function test_only_the_owner_posts_metadata() public {
        vm.prank(stranger);
        vm.expectRevert(bytes("not owner"));
        t.setMetadata("mine now", "");
    }

    function test_a_comp_and_a_free_claim_say_so_on_the_token_itself() public {
        t.compMint(stranger, 1);
        assertTrue(_contains(_json(t.tokenURI(1)), '"value":"gift"'), "a favour is visible or it corrodes the ticket");
    }

    function test_a_renderer_takes_over_and_zero_gives_the_built_in_back() public {
        FakeRenderer r = new FakeRenderer();
        t.setRenderer(address(r));
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        assertEq(t.tokenURI(id), "renderer://token");
        assertEq(t.contractURI(), "renderer://collection");
        t.setRenderer(address(0));
        assertTrue(_startsWith(t.tokenURI(id), "data:application/json;base64,"), "zero restores the built-in");
    }

    /// The hosted path's poke: under a renderer an art swap is a file save, so
    /// no storage moves and no event fires and a marketplace never re-reads.
    /// This emits both events and touches nothing — including the renderer,
    /// which is what separates it from the `setRenderer(address(this))` trick
    /// it replaces.
    function test_refreshMetadata_pokes_a_marketplace_and_changes_nothing() public {
        FakeRenderer r = new FakeRenderer();
        t.setRenderer(address(r));
        t.setMetadata("a blurb", "https://example.test/a.svg");

        vm.expectEmit(false, false, false, true);
        emit ScryGameTicket.BatchMetadataUpdate(1, type(uint256).max);
        t.refreshMetadata();

        assertEq(t.renderer(), address(r), "the renderer is untouched");
        assertEq(t.blurb(), "a blurb", "and so is every posted field");
        assertEq(t.image(), "https://example.test/a.svg");
    }

    function test_only_the_owner_pokes_a_marketplace() public {
        vm.prank(stranger);
        vm.expectRevert(bytes("not owner"));
        t.refreshMetadata();
    }

    function test_tokenURI_still_refuses_a_token_that_does_not_exist() public {
        vm.expectRevert(bytes("no such ticket"));
        t.tokenURI(999);
    }

    function test_erc4906_is_declared_because_metadata_can_change() public view {
        assertTrue(t.supportsInterface(0x49064906));
    }

    function _startsWith(string memory s, string memory p) internal pure returns (bool) {
        bytes memory b = bytes(s);
        bytes memory q = bytes(p);
        if (b.length < q.length) return false;
        for (uint256 i; i < q.length; ++i) {
            if (b[i] != q[i]) return false;
        }
        return true;
    }

    function _contains(string memory s, string memory needle) internal pure returns (bool) {
        bytes memory b = bytes(s);
        bytes memory q = bytes(needle);
        if (q.length == 0 || b.length < q.length) return false;
        for (uint256 i; i <= b.length - q.length; ++i) {
            bool hit = true;
            for (uint256 j; j < q.length; ++j) {
                if (b[i + j] != q[j]) {
                    hit = false;
                    break;
                }
            }
            if (hit) return true;
        }
        return false;
    }

    function test_erc721_transfer_rules_hold() public {
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        vm.prank(stranger);
        vm.expectRevert(bytes("not authorized"));
        t.transferFrom(buyer, stranger, id);
        vm.prank(buyer);
        t.approve(stranger, id);
        vm.prank(stranger);
        t.transferFrom(buyer, stranger, id);
        assertEq(t.ownerOf(id), stranger);
        assertEq(t.getApproved(id), address(0)); // approval cleared on transfer
    }

    // ── the collection door — hold a token, take a game ──────────────────────
    //
    // The door that needs no snapshot, no proofs file and no re-post. What
    // these pin is the ONE property that made it worth new bytecode: the count
    // rides the TOKEN ID, so it survives a sale and cannot be farmed by
    // scattering a collection across wallets.

    function _hive() internal returns (MockNFT hive) {
        hive = new MockNFT();
        hive.mint(buyer, 7);
        hive.mint(stranger, 9);
        t.setFreeCollection(address(hive), 1);
    }

    function _ids(uint256 a) internal pure returns (uint256[] memory ids) {
        ids = new uint256[](1);
        ids[0] = a;
    }

    function test_a_holder_claims_with_no_proof_and_no_list() public {
        MockNFT hive = _hive();
        vm.prank(buyer);
        assertEq(t.claimFreeFor(address(hive), _ids(7)), 1);
        assertEq(t.balanceOf(buyer), 1);
        assertEq(t.railOf(1), 5); // the same free mark the merkle door leaves
        assertEq(t.freeClaimedByToken(address(hive), 7), 1);
    }

    /// THE WHOLE REASON THIS DOOR EXISTS. Seat #7 takes its game, then sells.
    /// The buyer of the seat gets nothing more — the seat's copy was spent, and
    /// it stays spent through any number of hands.
    function test_the_count_rides_the_token_not_the_wallet() public {
        MockNFT hive = _hive();
        vm.prank(buyer);
        t.claimFreeFor(address(hive), _ids(7));

        vm.prank(buyer);
        hive.transferFrom(buyer, stranger, 7);
        vm.prank(stranger);
        vm.expectRevert(bytes("already claimed for this one"));
        t.claimFreeFor(address(hive), _ids(7));

        // and the new holder's OWN seat is untouched by any of it
        vm.prank(stranger);
        assertEq(t.claimFreeFor(address(hive), _ids(9)), 1);
    }

    function test_a_token_you_do_not_hold_claims_nothing() public {
        MockNFT hive = _hive();
        vm.prank(stranger);
        vm.expectRevert(bytes("not yours"));
        t.claimFreeFor(address(hive), _ids(7));
    }

    function test_shutting_the_door_forgets_no_spent_token() public {
        MockNFT hive = _hive();
        vm.prank(buyer);
        t.claimFreeFor(address(hive), _ids(7));

        t.setFreeCollection(address(hive), 0);
        vm.prank(stranger);
        vm.expectRevert(bytes("collection door closed"));
        t.claimFreeFor(address(hive), _ids(9));

        // reopening at the same allowance re-opens nobody who already took one
        t.setFreeCollection(address(hive), 1);
        vm.prank(buyer);
        vm.expectRevert(bytes("already claimed for this one"));
        t.claimFreeFor(address(hive), _ids(7));
    }

    function test_raising_perToken_hands_a_spent_seat_exactly_the_difference() public {
        MockNFT hive = _hive();
        vm.prank(buyer);
        t.claimFreeFor(address(hive), _ids(7));
        t.setFreeCollection(address(hive), 3);
        uint256[] memory twice = new uint256[](2);
        twice[0] = 7;
        twice[1] = 7; // a duplicate is just the same claim made twice
        vm.prank(buyer);
        assertEq(t.claimFreeFor(address(hive), twice), 2);
        assertEq(t.freeClaimedByToken(address(hive), 7), 3);
        vm.prank(buyer);
        vm.expectRevert(bytes("already claimed for this one"));
        t.claimFreeFor(address(hive), _ids(7));
    }

    function test_a_batch_claim_is_bounded_and_nonempty() public {
        MockNFT hive = _hive();
        uint256[] memory none = new uint256[](0);
        vm.prank(buyer);
        vm.expectRevert(bytes("bad id count"));
        t.claimFreeFor(address(hive), none);

        uint256[] memory tooMany = new uint256[](33);
        vm.prank(buyer);
        vm.expectRevert(bytes("bad id count"));
        t.claimFreeFor(address(hive), tooMany);
    }

    function test_a_collection_claim_grants_no_coin() public {
        MockToken coin = new MockToken("Gates Coin", "OBOL");
        coin.mint(address(t), 1_000e18);
        t.setGrant(IERC20(address(coin)), 100e18);
        MockNFT hive = _hive();
        vm.prank(buyer);
        t.claimFreeFor(address(hive), _ids(7));
        // a gift of a licence is not a gift of somebody's raise — same rule the
        // comp and merkle doors keep
        assertEq(coin.balanceOf(buyer), 0);
        assertEq(t.grantDelivered(), 0);
    }

    function test_the_doors_enumerate_for_a_surface() public {
        MockNFT hive = _hive();
        MockNFT other = new MockNFT();
        t.setFreeCollection(address(other), 2);
        t.setFreeCollection(address(hive), 5); // re-posting does not duplicate the row
        assertEq(t.freeCollectionCount(), 2);
        (address c0, uint256 p0) = t.freeCollectionAt(0);
        assertEq(c0, address(hive));
        assertEq(p0, 5);

        vm.prank(buyer);
        t.claimFreeFor(address(hive), _ids(7));
        uint256[] memory both = new uint256[](2);
        both[0] = 7;
        both[1] = 9;
        uint256[] memory left = t.freeRemainingFor(address(hive), both);
        assertEq(left[0], 4);
        assertEq(left[1], 5);
    }

    function test_only_the_owner_opens_a_collection_door() public {
        MockNFT hive = new MockNFT();
        vm.prank(stranger);
        vm.expectRevert(bytes("not owner"));
        t.setFreeCollection(address(hive), 1);
    }

    /// ⚠ MEASURED, NOT ESTIMATED — AND THE TWO LIVE IN SEPARATE TESTS ON
    /// PURPOSE. Measuring both inside one test made the collection door look
    /// 34k cheaper than it is: whichever claim runs second finds `nextTokenId`
    /// and the claimer's balance already WARM, and pays ~100 gas for slots the
    /// first one paid 5,000–22,100 for. Each door therefore gets its own
    /// transaction, as the first mint on a fresh contract, which is the only
    /// way the two columns mean the same thing.
    ///
    /// Two costs a message call still cannot see — one owed to each column:
    ///   + 6,656  merkle — a real tx pays 16 gas per nonzero calldata byte, and
    ///                     a 13-node proof (an 8,192-leaf tree) is 416 bytes
    ///   + 2,600  collection — the collection address is COLD in a real tx;
    ///                     here it was deployed in the same test, so it is warm
    function test_GAS_the_merkle_door() public {
        // a genuine 13-deep proof: pick the siblings, fold them into the root
        bytes32[] memory proof = new bytes32[](13);
        bytes32 node = _leaf(buyer, 1);
        for (uint256 i = 0; i < 13; i++) {
            proof[i] = keccak256(abi.encodePacked("sibling", i));
            node = node < proof[i]
                ? keccak256(abi.encodePacked(node, proof[i]))
                : keccak256(abi.encodePacked(proof[i], node));
        }
        t.setFreeRoot(node);

        uint256 g = gasleft();
        vm.prank(buyer);
        t.claimFree(1, proof);
        uint256 used = g - gasleft();

        console2.log("merkle claim, 13-node proof, execution:", used);
        console2.log("  + proof calldata a real tx also pays: ", uint256(13 * 32 * 16));
        assertLt(used, 130_000);
    }

    function test_GAS_the_collection_door() public {
        MockNFT hive = _hive();
        uint256 g = gasleft();
        vm.prank(buyer);
        t.claimFreeFor(address(hive), _ids(7));
        uint256 used = g - gasleft();

        console2.log("collection claim, one id, execution:  ", used);
        console2.log("  + cold collection address in a real tx:", uint256(2600));
        assertLt(used, 130_000);
    }

    /// The batch is where the door earns its keep: a holder with six seats pays
    /// the fixed cost once. This is the number that decides whether a claim
    /// button offers "claim all" or one row per seat.
    function test_GAS_the_collection_door_batched() public {
        MockNFT hive = new MockNFT();
        for (uint256 i = 1; i <= 6; i++) {
            hive.mint(buyer, i);
        }
        t.setFreeCollection(address(hive), 1);
        uint256[] memory six = new uint256[](6);
        for (uint256 i = 0; i < 6; i++) {
            six[i] = i + 1;
        }
        uint256 g = gasleft();
        vm.prank(buyer);
        t.claimFreeFor(address(hive), six);
        uint256 used = g - gasleft();
        console2.log("collection claim, SIX ids in one call:", used);
        console2.log("  per game claimed:                   ", used / 6);
    }
}


// ── minimal payable-in NFTs, for the custom rails ────────────────────────────

contract MockNFT {
    mapping(uint256 => address) public ownerOf;
    mapping(address => mapping(address => bool)) public isApprovedForAll;

    function mint(address to, uint256 id) external {
        ownerOf[id] = to;
    }

    function setApprovalForAll(address op, bool ok) external {
        isApprovedForAll[msg.sender][op] = ok;
    }

    function transferFrom(address from, address to, uint256 id) external {
        require(ownerOf[id] == from, "not owner");
        require(msg.sender == from || isApprovedForAll[from][msg.sender], "not approved");
        ownerOf[id] = to;
    }
}

contract Mock1155 {
    mapping(address => mapping(uint256 => uint256)) public balanceOf;
    mapping(address => mapping(address => bool)) public isApprovedForAll;

    function mint(address to, uint256 id, uint256 n) external {
        balanceOf[to][id] += n;
    }

    function setApprovalForAll(address op, bool ok) external {
        isApprovedForAll[msg.sender][op] = ok;
    }

    function safeTransferFrom(address from, address to, uint256 id, uint256 n, bytes calldata) external {
        require(msg.sender == from || isApprovedForAll[from][msg.sender], "not approved");
        require(balanceOf[from][id] >= n, "insufficient");
        balanceOf[from][id] -= n;
        balanceOf[to][id] += n;
    }
}

/// The rail registry — what the owner may change, what they may not, and the
/// two NFT shapes. The point of a rail is that accepting a new asset is a CALL
/// rather than a redeploy.
contract ScryGameTicketRailsTest is Test {
    ScryGameTicket t;
    MockToken scry;
    MockToken usdg;
    MockToken partner;
    MockNFT nft;
    Mock1155 multi;

    address sink = address(0x5117);
    address proceeds = address(0xDE7);
    address buyer = address(0xB07E5);
    address stranger = address(0x57A);

    function setUp() public {
        scry = new MockToken("Scry", "SCRY");
        usdg = new MockToken("USD Glo", "USDG");
        partner = new MockToken("Partner", "PTNR");
        nft = new MockNFT();
        multi = new Mock1155();
        t = new ScryGameTicket(
            "Gates", "GATES", "gates", IERC20(address(scry)), IERC20(address(usdg)),
            address(0xF33), proceeds, 0, "https://scry.moreright.xyz", address(0)
        );
    }

    // ── the reserved three keep their meaning ────────────────────────────────

    function test_the_seeded_rails_are_the_three_every_surface_reads() public view {
        (uint8 k1,,,, , bool o1, string memory l1) = t.railInfo(t.RAIL_ETH());
        (uint8 k2, address tok2,,, address s2,, string memory l2) = t.railInfo(t.RAIL_SCRY());
        (, address tok3,,, address s3,, string memory l3) = t.railInfo(t.RAIL_USDG());
        assertEq(k1, uint8(ScryGameTicket.AssetKind.NATIVE));
        assertEq(l1, "eth");
        assertFalse(o1, "a seeded rail starts SHUT - a deploy posts no price");
        assertEq(k2, uint8(ScryGameTicket.AssetKind.ERC20));
        assertEq(tok2, address(scry));
        assertEq(s2, address(0xF33), "SCRY lands on the splitter, never here");
        assertEq(l2, "scry");
        assertEq(tok3, address(usdg));
        assertEq(s3, proceeds);
        assertEq(l3, "usdg");
    }

    /// The wall. Repointing rail 2 would leave every surface captioned "half
    /// of every SCRY buy burns" over a coin that does nothing of the kind.
    function test_setRail_refuses_the_reserved_ids() public {
        for (uint256 id = 1; id <= 5; id++) {
            vm.expectRevert(bytes("rail reserved"));
            t.setRail(id, ScryGameTicket.AssetKind.ERC20, address(partner), 0, 1e18, sink, "nope");
        }
    }

    function test_a_second_native_rail_is_refused() public {
        vm.expectRevert(bytes("native is rail 1"));
        t.setRail(6, ScryGameTicket.AssetKind.NATIVE, address(1), 0, 1 ether, sink, "eth2");
    }

    function test_only_the_owner_posts_a_rail() public {
        vm.prank(stranger);
        vm.expectRevert(bytes("not owner"));
        t.setRail(6, ScryGameTicket.AssetKind.ERC20, address(partner), 0, 5e18, sink, "ptnr");
    }

    // ── a partner coin, added without a redeploy ─────────────────────────────

    function test_a_custom_erc20_rail_sells_a_copy() public {
        t.setRail(6, ScryGameTicket.AssetKind.ERC20, address(partner), 0, 5e18, sink, "ptnr");
        partner.mint(buyer, 5e18);
        vm.startPrank(buyer);
        partner.approve(address(t), 5e18);
        uint256 id = t.buyWithRail(6, new uint256[](0));
        vm.stopPrank();
        assertEq(t.ownerOf(id), buyer);
        assertTrue(t.entitled(buyer));
        assertEq(partner.balanceOf(sink), 5e18, "the partner coin lands on the rail's sink in one hop");
        assertEq(partner.balanceOf(address(t)), 0, "the ticket takes no custody");
        assertEq(t.railOf(id), 6, "provenance records WHICH rail bought it");
    }

    // ── an NFT as payment ────────────────────────────────────────────────────

    function test_an_nft_rail_takes_the_collection_and_any_ids_in_it() public {
        t.setRail(7, ScryGameTicket.AssetKind.ERC721, address(nft), 0, 2, sink, "stonks");
        nft.mint(buyer, 41);
        nft.mint(buyer, 42);
        vm.startPrank(buyer);
        nft.setApprovalForAll(address(t), true);
        uint256[] memory ids = new uint256[](2);
        ids[0] = 41;
        ids[1] = 42;
        uint256 id = t.buyWithRail(7, ids);
        vm.stopPrank();
        assertEq(t.ownerOf(id), buyer);
        assertEq(nft.ownerOf(41), sink);
        assertEq(nft.ownerOf(42), sink);
        assertEq(t.railOf(id), 7);
    }

    function test_an_nft_rail_wants_exactly_the_posted_count() public {
        t.setRail(7, ScryGameTicket.AssetKind.ERC721, address(nft), 0, 2, sink, "stonks");
        nft.mint(buyer, 41);
        vm.startPrank(buyer);
        nft.setApprovalForAll(address(t), true);
        uint256[] memory one = new uint256[](1);
        one[0] = 41;
        vm.expectRevert(bytes("wrong number of nfts"));
        t.buyWithRail(7, one);
        vm.stopPrank();
    }

    /// The same token twice would otherwise be caught only by the second
    /// transfer reverting — which does NOT happen when the sink is the buyer.
    function test_the_same_nft_cannot_pay_twice() public {
        t.setRail(7, ScryGameTicket.AssetKind.ERC721, address(nft), 0, 2, buyer, "self");
        nft.mint(buyer, 41);
        vm.startPrank(buyer);
        nft.setApprovalForAll(address(t), true);
        uint256[] memory dupe = new uint256[](2);
        dupe[0] = 41;
        dupe[1] = 41;
        vm.expectRevert(bytes("duplicate nft"));
        t.buyWithRail(7, dupe);
        vm.stopPrank();
    }

    function test_an_erc1155_rail_moves_the_posted_quantity_of_one_id() public {
        t.setRail(8, ScryGameTicket.AssetKind.ERC1155, address(multi), 9, 3, sink, "pack");
        multi.mint(buyer, 9, 5);
        vm.startPrank(buyer);
        multi.setApprovalForAll(address(t), true);
        uint256 id = t.buyWithRail(8, new uint256[](0));
        vm.stopPrank();
        assertEq(t.ownerOf(id), buyer);
        assertEq(multi.balanceOf(sink, 9), 3);
        assertEq(multi.balanceOf(buyer, 9), 2);
    }

    // ── shutting, and the native/ERC20 confusion ─────────────────────────────

    function test_closeRail_shuts_a_custom_rail_without_forgetting_it() public {
        t.setRail(6, ScryGameTicket.AssetKind.ERC20, address(partner), 0, 5e18, sink, "ptnr");
        t.closeRail(6);
        (,,, uint256 amount,, bool open, string memory label) = t.railInfo(6);
        assertFalse(open);
        assertEq(amount, 0);
        assertEq(label, "ptnr", "a shut rail keeps its name - it is shut, not deleted");
        partner.mint(buyer, 5e18);
        vm.startPrank(buyer);
        partner.approve(address(t), 5e18);
        vm.expectRevert(bytes("rail closed"));
        t.buyWithRail(6, new uint256[](0));
        vm.stopPrank();
    }

    function test_closeRail_shuts_the_scry_leg_and_priceScry_reads_zero() public {
        t.setPrices(1000, 0.003 ether, 700_000e18, 0);
        assertEq(t.priceScry(), 700_000e18);
        t.closeRail(t.RAIL_SCRY());
        assertEq(t.priceScry(), 0, "the legacy getter is a VIEW over the rail, so it follows");
        assertEq(t.priceWei(), 0.003 ether, "and the others are untouched");
    }

    function test_sending_eth_to_a_token_rail_is_refused() public {
        t.setRail(6, ScryGameTicket.AssetKind.ERC20, address(partner), 0, 5e18, sink, "ptnr");
        partner.mint(buyer, 5e18);
        vm.deal(buyer, 1 ether);
        vm.startPrank(buyer);
        partner.approve(address(t), 5e18);
        vm.expectRevert(bytes("not a native rail"));
        t.buyWithRail{value: 1 wei}(6, new uint256[](0));
        vm.stopPrank();
    }

    function test_an_undefined_rail_is_shut() public {
        vm.expectRevert(bytes("rail closed"));
        t.buyWithRail(99, new uint256[](0));
    }

    // ── enumeration, so a surface can render the set ─────────────────────────

    function test_rails_enumerate_for_a_surface() public {
        assertEq(t.railCount(), 3, "the three seeded rails");
        t.setRail(6, ScryGameTicket.AssetKind.ERC20, address(partner), 0, 5e18, sink, "ptnr");
        assertEq(t.railCount(), 4);
        assertEq(t.railIdAt(3), 6);
        t.setRail(6, ScryGameTicket.AssetKind.ERC20, address(partner), 0, 9e18, sink, "ptnr");
        assertEq(t.railCount(), 4, "re-posting an existing rail does not duplicate its id");
    }

    /// Renounce has to freeze the NEW knobs too, or the door it closes is ajar.
    function test_renounce_freezes_the_rail_registry() public {
        // ⚠ `ethRail` is hoisted deliberately. `t.RAIL_ETH()` inside the call's
        // argument list is a staticcall, and it CONSUMES the one-shot
        // expectRevert before the call under test runs — the same shape the
        // card-layer suite hit with a pranked `KIND_*()` getter.
        // Written inline, this test passes for the wrong reason.
        uint256 ethRail = t.RAIL_ETH();
        t.transferOwnership(address(0));
        vm.expectRevert(bytes("not owner"));
        t.setRail(6, ScryGameTicket.AssetKind.ERC20, address(partner), 0, 5e18, sink, "ptnr");
        vm.expectRevert(bytes("not owner"));
        t.closeRail(ethRail);
    }
}


/// The presale shape (`COPIES.md` §0): a buy is a copy AND the game's coin,
/// and graduation hands admin to the team. What these pin is the pair of
/// failure modes that cost somebody money — a grant that cannot pay taking the
/// money anyway, and a handover that bricks the contract on a typo.
contract ScryGameTicketRaiseTest is Test {
    ScryGameTicket t;
    MockToken scry;
    MockToken usdg;
    MockToken coin; // the game's own token — the raise's other half

    address proceeds = address(0xDE7);
    address buyer = address(0xB07E5);
    address buyer2 = address(0xB0B2);
    address devteam = address(0xDE4);
    address stranger = address(0x57A);

    uint256 constant PRICE_WEI = 0.003 ether;
    uint256 constant GRANT = 10_000e18;

    function setUp() public {
        scry = new MockToken("Scry", "SCRY");
        usdg = new MockToken("USD Glo", "USDG");
        coin = new MockToken("Gates Coin", "GCOIN");
        t = new ScryGameTicket(
            "Gates", "GATES", "gates", IERC20(address(scry)), IERC20(address(usdg)),
            address(0xF33), proceeds, 0, "https://scry.moreright.xyz", address(0)
        );
        t.setPrices(1000, PRICE_WEI, 0, 0);
        vm.deal(buyer, 1 ether);
        vm.deal(buyer2, 1 ether);
    }

    function _armGrant(uint256 copies) internal {
        t.setGrant(IERC20(address(coin)), GRANT);
        coin.mint(address(t), GRANT * copies); // the raise is FUNDED, never minted by the ticket
    }

    // ── a copy and the coin, in one buy ──────────────────────────────────────

    function test_a_buy_delivers_the_copy_and_the_coin_together() public {
        _armGrant(2);
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        assertEq(t.ownerOf(id), buyer, "the copy");
        assertEq(coin.balanceOf(buyer), GRANT, "and the coin, same transaction");
        assertEq(t.grantDelivered(), GRANT, "the raise is auditable on chain");
        assertEq(t.grantsRemaining(), 1, "one copy of reserve left");
    }

    function test_no_grant_posted_is_an_ordinary_licence_sale() public {
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        assertEq(t.ownerOf(id), buyer);
        assertEq(t.grantDelivered(), 0);
        assertEq(t.grantsRemaining(), 0, "no grant offered reads zero, not 'infinite'");
    }

    /// The wall. A card reading "a copy and 10,000 GCOIN" over a dry reserve is
    /// the card-advertising-a-reward-the-rule-cannot-pay bug WITH the money
    /// already taken. The sale stops at the door instead.
    function test_an_armed_grant_that_cannot_pay_REFUSES_the_buy() public {
        _armGrant(1);
        vm.prank(buyer);
        t.buyWithETH{value: PRICE_WEI}(); // drains the reserve exactly
        assertEq(t.grantsRemaining(), 0);

        uint256 before = buyer2.balance;
        vm.prank(buyer2);
        vm.expectRevert(bytes("grant reserve empty"));
        t.buyWithETH{value: PRICE_WEI}();
        assertEq(buyer2.balance, before, "and the money never left the buyer");
        assertEq(t.totalMinted(), 1, "no copy was sold either");
    }

    /// Otherwise the owner could comp copies to themselves and walk the raise
    /// out of the reserve.
    function test_comps_and_free_claims_grant_nothing() public {
        _armGrant(3);
        t.compMint(devteam, 1);
        assertEq(coin.balanceOf(devteam), 0, "a gift of a licence is not a gift of the raise");
        assertEq(t.grantsRemaining(), 3, "and it did not touch the reserve");
    }

    function test_closing_the_grant_leaves_the_sale_running() public {
        _armGrant(2);
        t.setGrant(IERC20(address(0)), 0);
        vm.prank(buyer);
        uint256 id = t.buyWithETH{value: PRICE_WEI}();
        assertEq(t.ownerOf(id), buyer);
        assertEq(coin.balanceOf(buyer), 0);
    }

    function test_only_the_owner_posts_or_recovers_a_grant() public {
        _armGrant(2);
        vm.startPrank(stranger);
        vm.expectRevert(bytes("not owner"));
        t.setGrant(IERC20(address(coin)), 1);
        vm.expectRevert(bytes("not owner"));
        t.recoverGrant(stranger, 1);
        vm.stopPrank();
    }

    function test_unsold_reserve_comes_back() public {
        _armGrant(2);
        t.recoverGrant(devteam, GRANT * 2);
        assertEq(coin.balanceOf(devteam), GRANT * 2);
        assertEq(t.grantsRemaining(), 0);
    }

    /// ETH has exactly one door and `recoverGrant` must not become a second.
    function test_recoverGrant_cannot_reach_the_eth() public {
        _armGrant(1);
        vm.prank(buyer);
        t.buyWithETH{value: PRICE_WEI}();
        assertEq(address(t).balance, PRICE_WEI);
        t.recoverGrant(devteam, 0); // the only asset it can name is the grant token
        assertEq(address(t).balance, PRICE_WEI, "the ETH is untouched");
        assertEq(devteam.balance, 0);
    }

    // ── graduation: handing admin to the team ────────────────────────────────

    function test_handover_is_two_steps_and_the_old_owner_rules_until_accepted() public {
        t.transferOwnership(devteam);
        assertEq(t.owner(), address(this), "offering is not giving");
        assertEq(t.pendingOwner(), devteam);

        t.setPrices(1000, PRICE_WEI, 0, 0); // still ours

        vm.prank(devteam);
        t.acceptOwnership();
        assertEq(t.owner(), devteam);
        assertEq(t.pendingOwner(), address(0));

        vm.prank(devteam);
        t.setPrices(2000, PRICE_WEI, 0, 0); // theirs now
        assertEq(t.priceUsdCents(), 2000);

        vm.expectRevert(bytes("not owner"));
        t.setPrices(1, 1, 0, 0); // and no longer ours
    }

    /// The whole reason for two steps: on one step this address owns the
    /// contract forever with nobody holding the key.
    function test_a_mistyped_handover_costs_nothing_and_is_recoverable() public {
        address typo = address(0xDEADBEEF);
        t.transferOwnership(typo);
        assertEq(t.owner(), address(this), "the contract is not stranded");

        t.transferOwnership(devteam); // simply offer it again
        vm.prank(typo);
        vm.expectRevert(bytes("not the pending owner"));
        t.acceptOwnership();

        vm.prank(devteam);
        t.acceptOwnership();
        assertEq(t.owner(), devteam);
    }

    function test_a_stranger_cannot_accept_an_offer_made_to_someone_else() public {
        t.transferOwnership(devteam);
        vm.prank(stranger);
        vm.expectRevert(bytes("not the pending owner"));
        t.acceptOwnership();
        assertEq(t.owner(), address(this));
    }

    function test_a_handover_can_be_withdrawn() public {
        t.transferOwnership(devteam);
        t.cancelHandover();
        assertEq(t.pendingOwner(), address(0));
        vm.prank(devteam);
        vm.expectRevert(bytes("not the pending owner"));
        t.acceptOwnership();
    }

    /// Renounce stays ONE step — it only removes power, and it must also kill
    /// any offer standing at the time or the contract could be un-renounced.
    function test_renounce_is_immediate_and_cancels_a_pending_offer() public {
        t.transferOwnership(devteam);
        t.transferOwnership(address(0));
        assertEq(t.owner(), address(0));
        assertEq(t.pendingOwner(), address(0));
        vm.prank(devteam);
        vm.expectRevert(bytes("not the pending owner"));
        t.acceptOwnership();
    }
}

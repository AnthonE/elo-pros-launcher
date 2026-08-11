// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryTicketUrlRenderer.sol";
import "../src/ScryGameTicket.sol";
import "../src/ScryFeeSplitter.sol";
import "./MockToken.sol";

/// The URL renderer is the other end of the `setRenderer` hinge, so what these
/// pin is mostly about the SEAM rather than string building:
///   - the two URLs are exactly what `meter/tickets.py` already serves, path
///     for path — a renderer that is one segment off produces a collection of
///     404s that looks fine on chain
///   - it survives being wired into a real ticket, and `setRenderer(0)` still
///     takes the built-in document back (the posted door out of a dark origin)
///   - neither view can revert, for any token id, including ones never minted
///     — a renderer that throws bricks every token at once
contract ScryTicketUrlRendererTest is Test {
    ScryTicketUrlRenderer r;
    address owner = address(this);
    address alice = address(0xA11CE);

    function setUp() public {
        r = new ScryTicketUrlRenderer(owner, "https://scry.moreright.xyz", "gates");
    }

    function test_the_urls_are_the_paths_the_meter_serves() public view {
        assertEq(r.tokenURI(1), "https://scry.moreright.xyz/ticket/gates/1/metadata");
        assertEq(r.tokenURI(7), "https://scry.moreright.xyz/ticket/gates/7/metadata");
        assertEq(r.contractURI(), "https://scry.moreright.xyz/ticket/gates/collection");
    }

    /// The digits matter: a token id is a path segment, and `_u` is the only
    /// arithmetic in the file.
    function test_the_id_renders_for_every_shape_of_number() public view {
        assertEq(r.tokenURI(0), "https://scry.moreright.xyz/ticket/gates/0/metadata");
        assertEq(r.tokenURI(10), "https://scry.moreright.xyz/ticket/gates/10/metadata");
        assertEq(r.tokenURI(1000), "https://scry.moreright.xyz/ticket/gates/1000/metadata");
        assertEq(
            r.tokenURI(type(uint256).max),
            "https://scry.moreright.xyz/ticket/gates/115792089237316195423570985008687907853269984665640564039457584007913129639935/metadata"
        );
    }

    function testFuzz_the_views_never_revert(uint256 id) public view {
        r.tokenURI(id);
        r.contractURI();
    }

    function test_the_base_moves_and_only_the_owner_moves_it() public {
        r.setBase("https://cdn.example.com");
        assertEq(r.tokenURI(3), "https://cdn.example.com/ticket/gates/3/metadata");

        vm.prank(alice);
        vm.expectRevert(ScryTicketUrlRenderer.NotOwner.selector);
        r.setBase("https://evil.example.com");
    }

    /// An empty base would emit `/ticket/gates/1/metadata`, which a
    /// marketplace resolves against ITS OWN host — a 404 nobody here can see.
    function test_an_empty_base_is_refused_at_both_doors() public {
        vm.expectRevert(ScryTicketUrlRenderer.EmptyBase.selector);
        r.setBase("");

        vm.expectRevert(ScryTicketUrlRenderer.EmptyBase.selector);
        new ScryTicketUrlRenderer(owner, "", "gates");
    }

    function test_ownership_is_two_step() public {
        r.transferOwnership(alice);
        assertEq(r.owner(), owner, "an offer is not a handover");

        vm.prank(address(0xBEEF));
        vm.expectRevert(ScryTicketUrlRenderer.NotPending.selector);
        r.acceptOwnership();

        vm.prank(alice);
        r.acceptOwnership();
        assertEq(r.owner(), alice);
    }

    /// The seam, end to end: a real ticket delegates to this, and the posted
    /// door out still works.
    function test_a_real_ticket_delegates_and_zero_gives_the_built_in_back() public {
        MockToken scry = new MockToken("Scry", "SCRY");
        MockToken usdg = new MockToken("USD Glo", "USDG");
        address[] memory to = new address[](1);
        uint16[] memory bps = new uint16[](1);
        to[0] = address(0xB0B);
        bps[0] = 10000;
        ScryFeeSplitter split = new ScryFeeSplitter(IERC20(address(scry)), to, bps);

        ScryGameTicket t = new ScryGameTicket(
            "Gates", "GATES-COPY", "gates", IERC20(address(scry)), IERC20(address(usdg)),
            address(split), address(0xDE7), 0, "https://scry.moreright.xyz", address(0)
        );
        t.compMint(alice, 1);
        uint256 id = 1;

        t.setRenderer(address(r));
        assertEq(t.tokenURI(id), "https://scry.moreright.xyz/ticket/gates/1/metadata");
        assertEq(t.contractURI(), "https://scry.moreright.xyz/ticket/gates/collection");

        t.setRenderer(address(0));
        assertEq(
            _startsWith(t.tokenURI(id), "data:application/json;base64,"),
            true,
            "setRenderer(0) must take the built-in document back"
        );
    }

    function _startsWith(string memory s, string memory p) internal pure returns (bool) {
        bytes memory b = bytes(s);
        bytes memory q = bytes(p);
        if (b.length < q.length) return false;
        for (uint256 i = 0; i < q.length; i++) {
            if (b[i] != q[i]) return false;
        }
        return true;
    }
}

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryGameTicket.sol";
import "./MockToken.sol";

contract MockNFT {
    mapping(uint256 => address) public ownerOf;

    function mint(address to, uint256 id) external {
        ownerOf[id] = to;
    }

    function transferFrom(address from, address to, uint256 id) external {
        require(ownerOf[id] == from, "not owner");
        ownerOf[id] = to;
    }
}

contract Mock1155 {
    mapping(address => mapping(uint256 => uint256)) public balanceOf;

    function mint(address to, uint256 id, uint256 n) external {
        balanceOf[to][id] += n;
    }

    function safeTransferFrom(address from, address to, uint256 id, uint256 n, bytes calldata) external {
        require(balanceOf[from][id] >= n, "balance");
        balanceOf[from][id] -= n;
        balanceOf[to][id] += n;
    }
}

/// Can money get OUT of a ticket? One question, asset by asset.
///
/// The contract does not custody payments at all — every non-native rail
/// transfers buyer → sink inside `_buy`, so there is normally nothing here to
/// withdraw. These tests pin the exceptions: what a misconfigured sink does,
/// which assets can be walked back out, and which never could.
contract TicketFundsCanLeaveTest is Test {
    ScryGameTicket t;
    MockToken scry;
    MockToken usdg;
    MockToken coinA;

    address proceeds = address(0xDE7);
    address sink = address(0xF33);
    address buyer = address(0xB07E5);
    address devteam = address(0xDE4);

    uint256 constant PRICE_WEI = 0.003 ether;
    uint256 constant GRANT = 10_000e18;

    function setUp() public {
        scry = new MockToken("Scry", "SCRY");
        usdg = new MockToken("USD Glo", "USDG");
        coinA = new MockToken("Coin A", "AAA");
        t = new ScryGameTicket(
            "Gates", "GATES", "gates", IERC20(address(scry)), IERC20(address(usdg)),
            sink, proceeds, 0, "https://scry.moreright.xyz", address(0)
        );
        t.setPrices(1000, PRICE_WEI, 0, 0);
        vm.deal(buyer, 1 ether);
    }

    // ── the normal path: nothing is ever held ────────────────────────────────

    function test_an_erc20_rail_never_custodies_anything() public {
        MockToken partner = new MockToken("Partner", "PTR");
        partner.mint(buyer, 1_000e18);
        t.setRail(6, ScryGameTicket.AssetKind.ERC20, address(partner), 0, 100e18, devteam, "partner");

        vm.startPrank(buyer);
        partner.approve(address(t), type(uint256).max);
        t.buyWithRail(6, new uint256[](0));
        vm.stopPrank();

        assertEq(partner.balanceOf(devteam), 100e18, "buyer -> sink, one hop");
        assertEq(partner.balanceOf(address(t)), 0, "the ticket held nothing at any point");
    }

    function test_eth_always_has_its_one_door_and_anyone_may_open_it() public {
        vm.prank(buyer);
        t.buyWithETH{value: PRICE_WEI}();
        assertEq(address(t).balance, PRICE_WEI);

        vm.prank(address(0xBEEF)); // permissionless on purpose
        t.sweep();
        assertEq(proceeds.balance, PRICE_WEI, "ETH reached the immutable proceeds");
        assertEq(address(t).balance, 0);
    }

    // ── the escape hatch, and its limit ──────────────────────────────────────

    /// Any ERC-20 that ends up here can be walked out, because `setGrant` is
    /// repointable and `recoverGrant` moves whatever it currently names. This
    /// is what makes a closed or switched grant reversible rather than fatal.
    function test_any_erc20_can_be_walked_out_by_repointing_the_grant() public {
        t.setGrant(IERC20(address(coinA)), GRANT);
        coinA.mint(address(t), GRANT * 2);

        t.setGrant(IERC20(address(0)), 0); // "close the grant" — looks terminal
        vm.expectRevert(bytes("no grant token"));
        t.recoverGrant(devteam, GRANT * 2);

        t.setGrant(IERC20(address(coinA)), 1); // ...but it is reversible
        t.recoverGrant(devteam, GRANT * 2);
        assertEq(coinA.balanceOf(devteam), GRANT * 2, "recovered in full");
        assertEq(coinA.balanceOf(address(t)), 0);
    }

    // ── the guard that closes the only one-way loss ──────────────────────────

    /// ⚠ THE REASON THE GUARD EXISTS. There is no ERC-721 or ERC-1155 transfer
    /// anywhere in this contract, so an NFT paid into a self-sinking rail could
    /// never move again — unlike an ERC-20, which the grant hatch above frees.
    function test_a_rail_may_not_sink_into_the_ticket_itself() public {
        MockNFT nft = new MockNFT();
        vm.expectRevert(bytes("sink is this contract"));
        t.setRail(6, ScryGameTicket.AssetKind.ERC721, address(nft), 0, 1, address(t), "nft");

        Mock1155 multi = new Mock1155();
        vm.expectRevert(bytes("sink is this contract"));
        t.setRail(7, ScryGameTicket.AssetKind.ERC1155, address(multi), 7, 5, address(t), "1155");

        vm.expectRevert(bytes("sink is this contract"));
        t.setRail(8, ScryGameTicket.AssetKind.ERC20, address(coinA), 0, 1e18, address(t), "erc20");
    }

    /// The immutable pair gets the same refusal, where it matters more: there
    /// is no setter to fix either one after construction.
    function test_the_immutable_sinks_may_not_be_the_ticket_itself() public {
        // Precomputed CREATE address for the next deployment from this test
        // contract — the constructor's `address(this)` is that, not ours.
        address next = vm.computeCreateAddress(address(this), vm.getNonce(address(this)));

        vm.expectRevert(bytes("sink is this contract"));
        new ScryGameTicket(
            "Gates", "GATES", "gates", IERC20(address(scry)), IERC20(address(usdg)),
            next, proceeds, 0, "https://scry.moreright.xyz", address(0)
        );

        next = vm.computeCreateAddress(address(this), vm.getNonce(address(this)));
        vm.expectRevert(bytes("proceeds is this contract"));
        new ScryGameTicket(
            "Gates", "GATES", "gates", IERC20(address(scry)), IERC20(address(usdg)),
            sink, next, 0, "https://scry.moreright.xyz", address(0)
        );
    }
}

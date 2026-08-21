// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {EloTrove} from "../src/EloTrove.sol";
import {MockNft, PausableNft, SoulboundNft, GachaLike, MockToken, SkimToken} from "./TroveMocks.sol";

/// @notice The canonical gate for `EloTrove`.
///
///         ⚠ The same claims are ALSO executed by `contracts/test_trove_evm.py`
///         on py-evm, because this box has no foundry (`ITEM-CONTRACT.md` §3a).
///         That runner proves the behaviour; this file is what
///         `./deploy_town.sh test` runs and it is the one that counts. If the
///         two ever disagree, this one is right.
contract EloTroveTest is Test {
    EloTrove trove;
    MockNft skins;
    MockNft art;
    GachaLike gacha;

    address constant ARTIST = address(0xA1);
    address constant HOUSE = address(0xB2);
    address constant BUYER = address(0xC3);
    address constant STRANGER = address(0xD4);

    function setUp() public {
        trove = new EloTrove("https://elopros.com/api/trove/");
        skins = new MockNft();
        art = new MockNft();
        gacha = new GachaLike();
    }

    function _wrap(MockNft c, address who, uint256 id) internal returns (uint256) {
        c.mint(who, id);
        vm.prank(who);
        c.approve(address(trove), id);
        vm.prank(who);
        return trove.wrap(address(c), id);
    }

    function test_wrapEscrowsAndMints() public {
        uint256 w = _wrap(skins, ARTIST, 7);
        assertEq(trove.ownerOf(w), ARTIST);
        assertEq(skins.ownerOf(7), address(trove));
        (address coll, uint256 tid, address dep, bool live, bool fungible) = trove.contentsOf(w);
        assertEq(coll, address(skins));
        assertEq(tid, 7);
        assertEq(dep, ARTIST);
        assertTrue(live);
        assertFalse(fungible);
    }

    /// THE WHOLE POINT. Two different collections, one wrapper address — so
    /// `EloGacha`'s `mapping(address => Pool)` sees ONE pool holding both.
    function test_twoCollectionsShareOneAddress() public {
        uint256 a = _wrap(skins, ARTIST, 7);
        uint256 b = _wrap(art, HOUSE, 99);
        assertTrue(address(skins) != address(art));
        (address ca,,,,) = trove.contentsOf(a);
        (address cb,,,,) = trove.contentsOf(b);
        assertTrue(ca != cb);
        assertEq(trove.ownerOf(a), ARTIST);
        assertEq(trove.ownerOf(b), HOUSE);
        assertEq(trove.totalSupply(), 2);
    }

    /// `EloGacha.deposit`'s exact pull, then a buyer unwrapping into the real
    /// NFT. The wrapper was never the prize.
    function test_gachaPullThenBuyerUnwraps() public {
        uint256 w = _wrap(skins, ARTIST, 7);
        vm.prank(ARTIST);
        trove.approve(address(gacha), w);
        vm.prank(ARTIST);
        gacha.pull(address(trove), w);
        assertEq(trove.ownerOf(w), address(gacha));

        gacha.push(address(trove), BUYER, w);
        assertEq(trove.ownerOf(w), BUYER);

        vm.prank(BUYER);
        trove.unwrap(w);
        assertEq(skins.ownerOf(7), BUYER);
        assertEq(trove.totalSupply(), 0);
        (,,, bool live,) = trove.contentsOf(w);
        assertFalse(live);
    }

    function test_depositorCannotRecall() public {
        uint256 w = _wrap(skins, ARTIST, 7);
        vm.prank(ARTIST);
        trove.transferFrom(ARTIST, BUYER, w);
        vm.prank(ARTIST);
        vm.expectRevert(EloTrove.NotTheOwner.selector);
        trove.unwrap(w);
    }

    function test_strangerCannotUnwrap() public {
        uint256 w = _wrap(skins, ARTIST, 7);
        vm.prank(STRANGER);
        vm.expectRevert(EloTrove.NotTheOwner.selector);
        trove.unwrap(w);
    }

    /// A collection that cannot survive the gacha's pull cannot survive the
    /// wrap either — and the failure landing HERE is the point, because the
    /// alternative is a position stranded mid-draw jamming the whole pool.
    function test_pausedCollectionCannotWrap() public {
        PausableNft p = new PausableNft();
        p.mint(HOUSE, 1);
        vm.prank(HOUSE);
        p.approve(address(trove), 1);
        p.setPaused(true);
        vm.prank(HOUSE);
        vm.expectRevert();
        trove.wrap(address(p), 1);

        p.setPaused(false);
        vm.prank(HOUSE);
        uint256 w = trove.wrap(address(p), 1);
        (address coll,,,,) = trove.contentsOf(w);
        assertEq(coll, address(p));
    }

    function test_soulboundCannotWrap() public {
        SoulboundNft s = new SoulboundNft();
        s.mint(HOUSE, 1);
        vm.prank(HOUSE);
        vm.expectRevert();
        trove.wrap(address(s), 1);
    }

    function test_cannotWrapTwice() public {
        _wrap(skins, ARTIST, 7);
        vm.prank(ARTIST);
        vm.expectRevert(EloTrove.AlreadyWrapped.selector);
        trove.wrap(address(skins), 7);
    }

    function test_notAContract() public {
        vm.prank(HOUSE);
        vm.expectRevert(EloTrove.NotAContract.selector);
        trove.wrap(HOUSE, 1);
    }

    function test_rewrapGetsAFreshIdAndTheOldOneStaysBurned() public {
        uint256 w = _wrap(skins, ARTIST, 7);
        vm.prank(ARTIST);
        trove.unwrap(w);
        vm.prank(ARTIST);
        skins.approve(address(trove), 7);
        vm.prank(ARTIST);
        uint256 w2 = trove.wrap(address(skins), 7);
        assertTrue(w2 > w);
        vm.expectRevert(EloTrove.NotWrapped.selector);
        trove.ownerOf(w);
    }

    // ── donated ERC-20s (operator, 2026-08-11) ──────────────────────────────
    function test_wrapTokens() public {
        MockToken coin = new MockToken();
        coin.mint(HOUSE, 1000e18);
        vm.prank(HOUSE);
        coin.approve(address(trove), 500e18);
        vm.prank(HOUSE);
        uint256 w = trove.wrapTokens(address(coin), 500e18);

        (address c, uint256 amt, address dep, bool live, bool fungible) = trove.contentsOf(w);
        assertEq(c, address(coin));
        assertEq(amt, 500e18); // the AMOUNT, not a token id
        assertEq(dep, HOUSE);
        assertTrue(live);
        assertTrue(fungible);
        assertEq(coin.balanceOf(address(trove)), 500e18);

        vm.prank(HOUSE);
        trove.unwrap(w);
        assertEq(coin.balanceOf(HOUSE), 1000e18);
    }

    /// `wrapperOf` is a 721 guard and must not apply to a fungible: ten people
    /// may each wrap 500 of the same coin.
    function test_sameTokenWrapsManyTimes() public {
        MockToken coin = new MockToken();
        coin.mint(HOUSE, 1000e18);
        vm.startPrank(HOUSE);
        coin.approve(address(trove), 1000e18);
        uint256 a = trove.wrapTokens(address(coin), 500e18);
        uint256 b = trove.wrapTokens(address(coin), 250e18);
        vm.stopPrank();
        assertTrue(a != b);
        assertEq(trove.totalSupply(), 2);
    }

    /// A fee-on-transfer token would mint a wrapper promising more than the
    /// trove holds. `pullExact` refuses rather than crediting the short amount.
    function test_feeOnTransferTokenIsRefused() public {
        SkimToken skim = new SkimToken();
        skim.mint(HOUSE, 1000e18);
        vm.prank(HOUSE);
        skim.approve(address(trove), 100e18);
        vm.prank(HOUSE);
        vm.expectRevert();
        trove.wrapTokens(address(skim), 100e18);
    }

    function test_zeroAmountRefused() public {
        MockToken coin = new MockToken();
        vm.prank(HOUSE);
        vm.expectRevert(EloTrove.ZeroAmount.selector);
        trove.wrapTokens(address(coin), 0);
    }

    function test_interfaces() public view {
        assertTrue(trove.supportsInterface(0x01ffc9a7)); // ERC-165
        assertTrue(trove.supportsInterface(0x80ac58cd)); // ERC-721
        assertFalse(trove.supportsInterface(0xd9b67a26)); // NOT ERC-1155
        assertFalse(trove.supportsInterface(0x2a55205a)); // NOT ERC-2981
    }

    /// The wrapper is an ordinary transferable ERC-721 in every direction, and
    /// nothing in its transfer path can revert for a reason of its own.
    function test_ordinaryTransfers() public {
        uint256 w = _wrap(skins, ARTIST, 7);
        vm.prank(ARTIST);
        trove.setApprovalForAll(STRANGER, true);
        vm.prank(STRANGER);
        trove.transferFrom(ARTIST, BUYER, w);
        assertEq(trove.ownerOf(w), BUYER);
        assertEq(trove.balanceOf(ARTIST), 0);
        assertEq(trove.balanceOf(BUYER), 1);
    }

    function testFuzz_wrapUnwrapRoundTrips(uint256 tokenId) public {
        vm.assume(tokenId != 0);
        uint256 w = _wrap(skins, ARTIST, tokenId);
        assertEq(skins.ownerOf(tokenId), address(trove));
        vm.prank(ARTIST);
        trove.unwrap(w);
        assertEq(skins.ownerOf(tokenId), ARTIST);
        assertEq(trove.totalSupply(), 0);
    }
}

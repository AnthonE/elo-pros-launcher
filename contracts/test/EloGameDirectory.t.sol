// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/EloGameDirectory.sol";
import "../src/EloGameTicketFactory.sol";
import "../src/EloGameTicket.sol";
import "../src/EloFeeSplitter.sol";
import "./MockToken.sol";

/// The shelf as chain state. What these pin, by section:
///   - a stranger walks every listing from ONE address, out of storage rather
///     than events, because `eth_getLogs` is refused past ~100 blocks here
///   - the house says WHO is listed; a title's own steward says WHAT its row
///     shows, and the chain stops at two links
///   - the directory can never touch a price, because it never owns a ticket
///   - the known-code signal cannot be claimed by a row that has not earned it
///   - the catalog anchor is append-only and may not rewind
contract EloGameDirectoryTest is Test {
    EloGameDirectory dir;
    EloGameTicketFactory factory;
    MockToken reserve;
    MockToken usdg;
    EloFeeSplitter splitter;

    address keeper = address(0xC0FFEE);
    address studio = address(0x5D10);
    address agent = address(0xA6E7);
    address stranger = address(0x57A);
    address proceeds = address(0xDE7);

    bytes ticketCode;

    function setUp() public {
        reserve = new MockToken("Scry", "SCRY");
        usdg = new MockToken("USD Glo", "USDG");
        address[] memory to = new address[](1);
        uint16[] memory bps = new uint16[](1);
        to[0] = RESERVE_BURN;
        bps[0] = 10_000;
        splitter = new EloFeeSplitter(IERC20(address(reserve)), to, bps);

        ticketCode = type(EloGameTicket).creationCode;
        factory = new EloGameTicketFactory(keccak256(ticketCode));
        dir = new EloGameDirectory(address(this));
        dir.setTrustedFactory(address(factory), true);
        dir.setKeeper(keeper, true);
    }

    function _params(string memory game, address owner_)
        internal
        view
        returns (EloGameTicketFactory.TicketParams memory p)
    {
        p = EloGameTicketFactory.TicketParams({
            name: "Gates",
            symbol: "GATES-COPY",
            game: game,
            reserve: address(reserve),
            usdg: address(usdg),
            reserveSink: address(splitter),
            proceeds: proceeds,
            supplyCap: 0,
            baseURI: "https://elopros.com",
            owner: owner_
        });
    }

    function _uris() internal pure returns (string[] memory u) {
        u = new string[](2);
        u[0] = "https://elopros.com/api/store/catalog";
        u[1] = "ipfs://bafyfakecatalogroot/gates.json";
    }

    // ── the factory: a known-code signal, not a gate ─────────────────────────

    function test_anyone_may_deploy_because_gating_it_would_gate_nothing() public {
        vm.prank(stranger);
        address t = factory.deploy(ticketCode, _params("indie", stranger), bytes32(0));
        assertTrue(factory.deployedByFactory(t));
        assertEq(EloGameTicket(t).owner(), stranger, "the deployer owns it when no owner is named");
        // and it confers no shelf place whatsoever
        assertEq(dir.count(), 0, "deploying is not listing");
    }

    function test_the_factory_refuses_code_that_is_not_the_pinned_build() public {
        bytes memory notTheTicket = type(MockToken).creationCode;
        vm.expectRevert(bytes("unexpected ticket code"));
        factory.deploy(notTheTicket, _params("gates", address(this)), bytes32(0));
    }

    function test_predict_answers_before_the_address_exists() public {
        EloGameTicketFactory.TicketParams memory p = _params("gates", address(this));
        address predicted = factory.predict(ticketCode, p, bytes32(uint256(7)));
        assertEq(predicted.code.length, 0, "nothing is there yet");
        address actual = factory.deploy(ticketCode, p, bytes32(uint256(7)));
        assertEq(actual, predicted, "a store page can be prepared before the broadcast");
    }

    function test_the_same_salt_twice_fails_loudly_rather_than_returning_zero() public {
        EloGameTicketFactory.TicketParams memory p = _params("gates", address(this));
        factory.deploy(ticketCode, p, bytes32(uint256(1)));
        vm.expectRevert(bytes("create2 failed"));
        factory.deploy(ticketCode, p, bytes32(uint256(1)));
    }

    function test_the_ticket_owner_is_set_at_birth_so_no_contract_holds_a_live_sale() public {
        address t = factory.deploy(ticketCode, _params("gates", studio), bytes32(0));
        assertEq(EloGameTicket(t).owner(), studio);
        assertEq(EloGameTicket(t).pendingOwner(), address(0), "no second step is owed");
        // the studio can price its own game immediately
        vm.prank(studio);
        EloGameTicket(t).setPrices(1000, 1 ether, 0, 0);
        assertEq(EloGameTicket(t).priceWei(), 1 ether);
    }

    function test_deployments_are_enumerable_from_state() public {
        factory.deploy(ticketCode, _params("a", address(this)), bytes32(uint256(1)));
        factory.deploy(ticketCode, _params("b", address(this)), bytes32(uint256(2)));
        assertEq(factory.deploymentCount(), 2);
        assertEq(factory.deployments(0, 0).length, 2, "limit 0 means to the end");
        assertEq(factory.deployments(1, 5).length, 1, "a limit past the end clamps");
        assertEq(factory.deployments(9, 1).length, 0, "paging past the end stops, it does not revert");
    }

    // ── listing is a hand act, and the whole shelf reads from one address ────

    function test_a_keeper_lists_and_a_stranger_walks_the_whole_shelf() public {
        address t = factory.deploy(ticketCode, _params("gates", studio), bytes32(0));
        vm.prank(keeper);
        dir.register("gates", t, address(0), address(0), studio, address(factory), keccak256("catalog-bytes"), _uris());

        assertEq(dir.count(), 1);
        (string[] memory slugs, EloGameDirectory.Listing[] memory rows) = dir.page(0, 0);
        assertEq(slugs[0], "gates");
        assertEq(rows[0].ticket, t);
        assertEq(rows[0].steward, studio);
        assertEq(rows[0].viaFactory, address(factory));
        assertEq(rows[0].contentHash, keccak256("catalog-bytes"));
        assertEq(dir.urisOf("gates").length, 2);
        assertEq(dir.urisOf("gates")[1], "ipfs://bafyfakecatalogroot/gates.json");
    }

    function test_a_stranger_may_read_but_never_list() public {
        address t = factory.deploy(ticketCode, _params("gates", studio), bytes32(0));
        vm.prank(stranger);
        vm.expectRevert(bytes("not keeper"));
        dir.register("gates", t, address(0), address(0), studio, address(0), bytes32(0), _uris());
    }

    function test_a_slug_cannot_be_taken_twice() public {
        address t = factory.deploy(ticketCode, _params("gates", studio), bytes32(0));
        dir.register("gates", t, address(0), address(0), studio, address(0), bytes32(0), _uris());
        vm.expectRevert(bytes("slug taken"));
        dir.register("gates", t, address(0), address(0), studio, address(0), bytes32(0), _uris());
    }

    function test_a_hand_deployed_ticket_lists_exactly_like_a_factory_one() public {
        EloGameTicket hand = new EloGameTicket(
            "Indie", "INDIE-COPY", "indie", IERC20(address(reserve)), IERC20(address(usdg)),
            address(splitter), proceeds, 0, "https://example.com", studio
        );
        dir.register("indie", address(hand), address(0), address(0), studio, address(0), bytes32(0), _uris());
        assertEq(dir.listingOf("indie").viaFactory, address(0), "the factory is a signal, never a requirement");
    }

    function test_a_row_cannot_claim_a_known_build_it_does_not_have() public {
        EloGameTicket hand = new EloGameTicket(
            "Indie", "INDIE-COPY", "indie", IERC20(address(reserve)), IERC20(address(usdg)),
            address(splitter), proceeds, 0, "https://example.com", studio
        );
        vm.expectRevert(bytes("not from that factory"));
        dir.register("indie", address(hand), address(0), address(0), studio, address(factory), bytes32(0), _uris());
    }

    function test_an_untrusted_factory_is_refused_even_if_it_attests() public {
        EloGameTicketFactory rogue = new EloGameTicketFactory(keccak256(ticketCode));
        address t = rogue.deploy(ticketCode, _params("gates", studio), bytes32(0));
        vm.expectRevert(bytes("factory not trusted"));
        dir.register("gates", t, address(0), address(0), studio, address(rogue), bytes32(0), _uris());
    }

    // ── listNew: deploy and shelve in one act ────────────────────────────────

    function test_listNew_deploys_and_shelves_in_one_transaction() public {
        vm.prank(keeper);
        address t = dir.listNew(
            "gates",
            ticketCode,
            _params("gates", studio),
            bytes32(uint256(3)),
            address(factory),
            address(0),
            studio,
            keccak256("bytes"),
            _uris()
        );
        assertEq(dir.listingOf("gates").ticket, t);
        assertEq(EloGameTicket(t).owner(), studio, "the studio owns its own sale from block one");
        assertTrue(factory.deployedByFactory(t));
    }

    function test_listNew_refuses_to_leave_the_directory_owning_a_live_sale() public {
        // owner 0 means "the caller" at the factory, and the caller here is the
        // directory. A directory that owned a ticket could set its prices.
        vm.prank(keeper);
        vm.expectRevert(bytes("name the ticket's owner"));
        dir.listNew(
            "gates",
            ticketCode,
            _params("gates", address(0)),
            bytes32(0),
            address(factory),
            address(0),
            studio,
            bytes32(0),
            _uris()
        );
    }

    function test_the_directory_owns_no_ticket_so_it_can_never_touch_a_price() public {
        vm.prank(keeper);
        address t = dir.listNew(
            "gates",
            ticketCode,
            _params("gates", studio),
            bytes32(uint256(4)),
            address(factory),
            address(0),
            studio,
            bytes32(0),
            _uris()
        );
        assertTrue(EloGameTicket(t).owner() != address(dir));
        vm.prank(address(dir));
        vm.expectRevert(bytes("not owner"));
        EloGameTicket(t).setPrices(1000, 1, 0, 0);
    }

    // ── the house says who; the studio says what ─────────────────────────────

    function _listed() internal returns (address t) {
        t = factory.deploy(ticketCode, _params("gates", studio), bytes32(0));
        dir.register("gates", t, address(0), address(0), studio, address(factory), keccak256("v1"), _uris());
    }

    function test_the_studio_edits_its_own_row() public {
        _listed();
        string[] memory u = new string[](1);
        u[0] = "ipfs://bafyv2/gates.json";
        vm.prank(studio);
        dir.setContent("gates", keccak256("v2"), u);
        assertEq(dir.listingOf("gates").contentHash, keccak256("v2"));
        assertEq(dir.urisOf("gates").length, 1, "a fresh uri set replaces, it does not append");
    }

    function test_a_studio_may_not_edit_somebody_elses_row() public {
        _listed();
        address t2 = factory.deploy(ticketCode, _params("other", stranger), bytes32(uint256(9)));
        dir.register("other", t2, address(0), address(0), stranger, address(factory), bytes32(0), _uris());
        vm.prank(studio);
        vm.expectRevert(bytes("not steward"));
        dir.setContent("other", keccak256("hijack"), _uris());
    }

    function test_a_steward_hands_off_and_the_chain_stops_there() public {
        _listed();
        vm.prank(studio);
        dir.setSteward("gates", agent);
        assertEq(dir.listingOf("gates").steward, agent);
        // the old steward is now a stranger to the row
        vm.prank(studio);
        vm.expectRevert(bytes("not steward"));
        dir.setContent("gates", keccak256("v3"), _uris());
        // and a steward still cannot create a listing
        vm.prank(agent);
        vm.expectRevert(bytes("not keeper"));
        dir.register("new-game", address(0xBEEF), address(0), address(0), agent, address(0), bytes32(0), _uris());
    }

    function test_a_studio_points_its_row_at_its_coin_and_pool() public {
        _listed();
        vm.prank(studio);
        dir.setPointers("gates", address(0xC01), address(0xF00));
        assertEq(dir.listingOf("gates").coin, address(0xC01));
        assertEq(dir.listingOf("gates").pool, address(0xF00));
    }

    function test_delisting_never_erases_that_it_was_listed() public {
        _listed();
        vm.prank(keeper);
        dir.setDelisted("gates", true);
        assertTrue(dir.listingOf("gates").delisted);
        assertEq(dir.count(), 1, "a shelf that can forget is a shelf nobody can check");
        assertEq(dir.listingOf("gates").listedAt > 0, true);
    }

    function test_only_a_keeper_delists() public {
        _listed();
        vm.prank(studio);
        vm.expectRevert(bytes("not keeper"));
        dir.setDelisted("gates", true);
    }

    function test_editing_a_row_that_does_not_exist_refuses() public {
        vm.prank(keeper);
        vm.expectRevert(bytes("no such listing"));
        dir.setContent("nope", bytes32(0), _uris());
    }

    // ── the anchor over storekeeper's signed edit log ────────────────────────

    function test_the_anchor_pins_the_log_head_and_may_not_rewind() public {
        vm.prank(keeper);
        dir.anchorCatalog(keccak256("head-12"), 12);
        assertEq(dir.catalogLength(), 12);
        vm.prank(keeper);
        dir.anchorCatalog(keccak256("head-40"), 40);
        assertEq(dir.catalogHead(), keccak256("head-40"));
        vm.prank(keeper);
        vm.expectRevert(bytes("length must not rewind"));
        dir.anchorCatalog(keccak256("old"), 39);
    }

    function test_only_a_keeper_anchors() public {
        vm.prank(stranger);
        vm.expectRevert(bytes("not keeper"));
        dir.anchorCatalog(keccak256("x"), 1);
    }

    // ── the authority root is readable, which is the point ───────────────────

    function test_the_keeper_set_is_readable_and_never_double_lists() public {
        assertEq(dir.keepers().length, 2, "the owner and the one added in setUp");
        dir.setKeeper(keeper, false);
        assertEq(dir.keepers().length, 1);
        dir.setKeeper(keeper, true);
        assertEq(dir.keepers().length, 2, "a re-grant is one holder, not two");
    }

    function test_handover_is_two_steps_and_renounce_is_one() public {
        dir.transferOwnership(studio);
        assertEq(dir.owner(), address(this), "still ours until they accept");
        vm.prank(studio);
        dir.acceptOwnership();
        assertEq(dir.owner(), studio);

        vm.prank(studio);
        dir.transferOwnership(address(0));
        assertEq(dir.owner(), address(0), "renounce only removes power, so it needs no acceptance");
    }

    function test_a_stranger_cannot_accept_an_offer_made_to_someone_else() public {
        dir.transferOwnership(studio);
        vm.prank(stranger);
        vm.expectRevert(bytes("not the pending owner"));
        dir.acceptOwnership();
    }

    // ── bounds ───────────────────────────────────────────────────────────────

    function test_uris_are_bounded_in_count_and_length() public {
        address t = factory.deploy(ticketCode, _params("gates", studio), bytes32(0));
        string[] memory many = new string[](9);
        for (uint256 i; i < 9; ++i) {
            many[i] = "https://example.com";
        }
        vm.expectRevert(bytes("at most 8 uris"));
        dir.register("gates", t, address(0), address(0), studio, address(0), bytes32(0), many);
    }

    function test_an_empty_slug_is_refused() public {
        address t = factory.deploy(ticketCode, _params("gates", studio), bytes32(0));
        vm.expectRevert(bytes("slug 1..64"));
        dir.register("", t, address(0), address(0), studio, address(0), bytes32(0), _uris());
    }

    function test_paging_past_the_end_returns_empty_rather_than_reverting() public {
        _listed();
        (string[] memory slugs, EloGameDirectory.Listing[] memory rows) = dir.page(5, 10);
        assertEq(slugs.length, 0);
        assertEq(rows.length, 0);
    }

    function test_the_directory_cannot_receive_ether() public {
        vm.deal(address(this), 1 ether);
        (bool ok,) = address(dir).call{value: 1 ether}("");
        assertFalse(ok, "nothing here is payable, so nothing can accumulate");
    }
}

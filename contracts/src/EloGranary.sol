// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SpoilsToken} from "./SpoilsToken.sol";

/// @title EloGranary - the single mint chokepoint for a SpoilsToken
/// @notice SpoilsToken has exactly one `minter` slot. The moment two organs
///         need to mint the same coin (the harvest ledger funding + the
///         Gardener farm), that slot must point at a hub, not at either of
///         them. The granary IS that hub: it holds the minter role and
///         portions mint room out to granted minters, each behind its own
///         POSTED daily cap. This is where the emission throttle lives,
///         enforced on chain for every distributor at once.
///
///         Grant mints CLAMP instead of reverting: a grantee asking for more
///         than today's remaining budget receives the remainder and is told
///         how much it got. Downstream accounting (the Gardener's stash)
///         carries the shortfall; nothing bricks at the day boundary.
///
///         Score-blind by construction, same as the token underneath: the
///         granary knows budgets and calendars, never scores. No meter
///         output keys any mint (CLAUDE.md, the line that never moves).
contract EloGranary {
    SpoilsToken public immutable spoils;
    address public steward;
    address public pendingSteward; // two-step handoff; see transferSteward

    struct Grant {
        uint256 dailyCap; // max mint per UTC day; 0 with enabled=false = no grant
        uint256 mintedToday; // spent against today's cap
        uint256 day; // UTC day index the counters refer to
        bool enabled;
    }

    mapping(address => Grant) public grants;

    event GrantSet(address indexed minter, uint256 dailyCap);
    event GrantRevoked(address indexed minter);
    event GrantMint(address indexed minter, address indexed to, uint256 requested, uint256 minted);
    event StewardMint(address indexed to, uint256 amount);
    event MinterRotatedAway(address indexed next);
    event StewardTransferred(address indexed from, address indexed to);
    event StewardTransferProposed(address indexed from, address indexed to);

    modifier onlySteward() {
        require(msg.sender == steward, "steward only");
        _;
    }

    constructor(SpoilsToken _spoils) {
        spoils = _spoils;
        steward = msg.sender;
    }

    // -- granted mints (the farm and friends) --------------------------------

    /// Mint up to `amount` to `to`, clamped to the caller's remaining daily
    /// budget. Returns what was actually minted; the caller carries the rest.
    function mint(address to, uint256 amount) external returns (uint256 minted) {
        Grant storage g = grants[msg.sender];
        require(g.enabled, "no grant");
        uint256 today = block.timestamp / 1 days;
        if (g.day != today) {
            g.day = today;
            g.mintedToday = 0;
        }
        uint256 room = g.dailyCap > g.mintedToday ? g.dailyCap - g.mintedToday : 0;
        minted = amount < room ? amount : room;
        if (minted > 0) {
            g.mintedToday += minted;
            spoils.mint(to, minted);
        }
        emit GrantMint(msg.sender, to, amount, minted);
    }

    /// Remaining mint room for `who` today (view, for UIs and tests).
    function availableToday(address who) external view returns (uint256) {
        Grant storage g = grants[who];
        if (!g.enabled) return 0;
        if (g.day != block.timestamp / 1 days) return g.dailyCap;
        return g.dailyCap > g.mintedToday ? g.dailyCap - g.mintedToday : 0;
    }

    // -- steward acts (the operator's ledger mints and the knobs) ------------

    /// Direct steward mint: funding EloHarvest against the public game
    /// ledger (POOLS.md section 3 step 3). Deliberate human act, not budgeted;
    /// the public record of what it funds is the merkle root it backs.
    function stewardMint(address to, uint256 amount) external onlySteward {
        spoils.mint(to, amount);
        emit StewardMint(to, amount);
    }

    function setGrant(address who, uint256 dailyCap) external onlySteward {
        require(who != address(0), "zero minter");
        Grant storage g = grants[who];
        g.dailyCap = dailyCap;
        g.enabled = true;
        emit GrantSet(who, dailyCap);
    }

    function revokeGrant(address who) external onlySteward {
        delete grants[who];
        emit GrantRevoked(who);
    }

    /// Escape hatch: hand the SpoilsToken minter role onward (a successor
    /// hub, or retirement). After this the granary can mint nothing.
    function rotateMinterAway(address next) external onlySteward {
        spoils.setMinter(next);
        emit MinterRotatedAway(next);
    }

    /// Two-step, deliberately. Every steward-only function — `stewardMint`,
    /// `setGrant`, `revokeGrant`, `rotateMinterAway` — becomes permanently
    /// unreachable if this is pointed at an address nobody controls, and
    /// because the granary holds the SpoilsToken minter slot (`setMinter` is
    /// `onlyMinter`) the coin's mint authority welds to a granary nobody can
    /// drive: no new distributor, no further `stewardMint` to fund a harvest
    /// root. A single-step handoff to a typo does that in one transaction, and
    /// `deploy_town.sh` never calls this, so the rotate phase's nonce/code
    /// typo filter never sees it. The successor must prove it can transact.
    function transferSteward(address to) external onlySteward {
        require(to != address(0), "zero steward");
        pendingSteward = to;
        emit StewardTransferProposed(steward, to);
    }

    /// The successor claims. This is the proof the address is controlled.
    function acceptSteward() external {
        require(msg.sender == pendingSteward, "not pending steward");
        emit StewardTransferred(steward, pendingSteward);
        steward = pendingSteward;
        pendingSteward = address(0);
    }

    /// Withdraw a pending handoff before it is claimed.
    function cancelStewardTransfer() external onlySteward {
        emit StewardTransferProposed(steward, address(0));
        pendingSteward = address(0);
    }
}

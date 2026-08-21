// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SpoilsToken} from "./SpoilsToken.sol";

/// @title EloShrine - burn-to-level votive offerings (the Meditation Circle rip)
/// @notice The on-chain mirror of the Agora's shrine - "burn MYRRH as a pure
///         offering: no payout, no buff, just the public record" -
///         with the mechanic DFK's Meditation Circle proved: burning the
///         economy's coins to climb a posted ladder is a sink people climb
///         for its own sake. Offer spoils (burned forever, supply retired),
///         accrue votive points at posted per-token rates, hold a rank on
///         the seven-grade ladder.
///
///         The grades are the Mithraic seven - the oldest leveling ladder
///         in the language, and the house crew already keeps Mithra as its
///         Oath-Keeper: Corax, Nymphus, Miles, Leo, Perses, Heliodromus,
///         Pater. Ancient-base naming, the usual claim: the oldest words
///         carry the least drift.
///
///         What a rank IS: a public, on-chain votive record. What it is
///         NOT, stated here so nobody back-fills it silently:
///         - NOT reputation. EloReputation is soulbound, earned by jobs
///           and slashed by the court - never bought. A burned offering
///           buys standing at the shrine and nothing else.
///         - NOT access, odds, payouts, or weight. No reserve system reads
///           the shrine to price, gate, emit, or measure anything. The
///           meter is score-blind the other way around too: no meter
///           number keys a rank (CLAUDE.md, the line that never moves).
///         - NOT yield. Points only ever go up, and they buy nothing.
///
///         The ladder is a pure function of lifetime points: level n costs
///         `baseStep * n` more points than level n-1 (triangular cumulative
///         thresholds), posted at deploy, immutable. The offering menu is
///         append-only: rates are posted, never edited; a closed offering
///         refuses new burns only.
contract EloShrine {
    struct Offering {
        SpoilsToken spoils; // the coin this altar takes (burned on offer)
        uint256 pointsPerToken; // votive points per whole token (1e18 wei)
        bool enabled; // a closed altar refuses NEW offerings only
    }

    uint256 public immutable baseStep; // points from rank 0 to rank 1 (1e18-scaled)
    address public owner;

    Offering[] public offerings;
    mapping(address => bool) public offeringAdded; // no duplicate altars
    mapping(address => uint256) public votiveOf; // lifetime points, monotone
    uint256 public totalVotive; // town-wide, monotone

    event OfferingAdded(uint256 indexed offering, address indexed spoils, uint256 pointsPerToken);
    event OfferingEnabled(uint256 indexed offering, bool enabled);
    event Offered(address indexed who, uint256 indexed offering, uint256 amount, uint256 points, uint256 level);
    event OwnerTransferred(address indexed from, address indexed to);

    modifier onlyOwner() {
        require(msg.sender == owner, "owner only");
        _;
    }

    constructor(uint256 _baseStep) {
        require(_baseStep > 0, "zero step");
        baseStep = _baseStep;
        owner = msg.sender;
    }

    // -- the offering -------------------------------------------------------
    /// Burn `amount` of an altar's spoils as a votive offering. The coins
    /// leave supply forever (SpoilsToken.burnFrom - approve first); the
    /// points are the only thing received, and they buy nothing.
    function offer(uint256 offeringId, uint256 amount) external returns (uint256 level) {
        require(amount > 0, "zero");
        Offering storage o = offerings[offeringId];
        require(o.enabled, "altar closed");
        uint256 points = (amount * o.pointsPerToken);
        o.spoils.burnFrom(msg.sender, amount);
        votiveOf[msg.sender] += points;
        totalVotive += points;
        level = levelOf(msg.sender);
        emit Offered(msg.sender, offeringId, amount, points, level);
    }

    // -- the ladder ---------------------------------------------------------
    /// Rank held by `who`: the largest L with baseStep * L * (L+1) / 2
    /// <= lifetime points (each rank costs baseStep * rank more than the
    /// last, the Meditation Circle's rising price). Closed form via isqrt,
    /// so a mountain of offerings never costs a mountain of gas.
    function levelOf(address who) public view returns (uint256) {
        uint256 v = votiveOf[who] / baseStep; // whole steps banked
        // L(L+1)/2 <= v  =>  L = floor((sqrt(8v + 1) - 1) / 2)
        return (_isqrt(8 * v + 1) - 1) / 2;
    }

    /// Cumulative points required to hold rank `level`.
    function pointsForLevel(uint256 level) public view returns (uint256) {
        return (baseStep * level * (level + 1)) / 2;
    }

    /// The Mithraic grade for a rank. Rank 7 is Pater; the ladder keeps
    /// counting above it (Pater is not a cap, just the last new name).
    function rankName(uint256 level) public pure returns (string memory) {
        if (level == 0) return "Profane";
        if (level == 1) return "Corax";
        if (level == 2) return "Nymphus";
        if (level == 3) return "Miles";
        if (level == 4) return "Leo";
        if (level == 5) return "Perses";
        if (level == 6) return "Heliodromus";
        return "Pater";
    }

    function offeringCount() external view returns (uint256) {
        return offerings.length;
    }

    // Babylonian integer square root: floor(sqrt(x)). Converges in <= 255
    // halvings; the seed overshoots so the loop only ever descends.
    function _isqrt(uint256 x) internal pure returns (uint256 z) {
        if (x == 0) return 0;
        z = x;
        uint256 y = (x + 1) / 2;
        while (y < z) {
            z = y;
            y = (x / y + y) / 2;
        }
    }

    // -- operator knobs (posted, append-only) -------------------------------
    function addOffering(SpoilsToken spoils, uint256 pointsPerToken) external onlyOwner {
        require(!offeringAdded[address(spoils)], "altar exists");
        require(pointsPerToken > 0, "zero rate");
        offeringAdded[address(spoils)] = true;
        offerings.push(Offering({spoils: spoils, pointsPerToken: pointsPerToken, enabled: true}));
        emit OfferingAdded(offerings.length - 1, address(spoils), pointsPerToken);
    }

    function setOfferingEnabled(uint256 offeringId, bool enabled) external onlyOwner {
        offerings[offeringId].enabled = enabled;
        emit OfferingEnabled(offeringId, enabled);
    }

    function transferOwner(address to) external onlyOwner {
        require(to != address(0), "zero owner");
        emit OwnerTransferred(owner, to);
        owner = to;
    }
}

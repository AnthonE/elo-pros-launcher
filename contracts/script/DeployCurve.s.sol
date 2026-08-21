// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloCurve.sol";

/// Deploy a title's launch curve — the two-way venue that exists before the
/// pool does.
///
/// ⚠ THE FIGURES ARE STAGED IN contracts/curve.env.example (added 2026-08-18).
///   Every CURVE_* variable was typed from this header before that existed.
///
/// ⚠ EVERY FIGURE HERE IS WELDED AT CONSTRUCTION. `EloCurve` has no owner, no
/// steward and no setter of any kind. There is nothing to fix afterwards: a
/// wrong number is a redeploy, and a redeploy after anyone has bought is a
/// second market in the same coin. Read the preview this script prints before
/// you broadcast, not after.
///
///   export CURVE_COIN=0x...        # the game's coin
///   export CURVE_QUOTE=0x...       # ELO. GATES.md 2.1 - the RESERVE, and only
///                                  #   the reserve. NOT SCRY: that is the retired
///                                  #   one (ONE-SHOT.md 2) and this is welded.
///   export CURVE_NPM=0x...         # canonical v3 NonfungiblePositionManager
///   export CURVE_CREATOR_FEES=0x...   # the studio's cut of trading fees
///   export CURVE_PROTOCOL_FEES=0x...  # ours
///   export CURVE_QUOTE_POOL_FEES=0x...# the ELO leg, once graduated
///   export CURVE_COIN_POOL_FEES=0x... # the coin leg - the game's own splitter
///   export CURVE_VIRTUAL_QUOTE=...    # sets the OPENING price
///   export CURVE_ALLOC=...            # coin the curve may sell
///   export CURVE_POOL_ALLOC=...       # coin held back, becomes the pool
///   export CURVE_THRESHOLD=...        # ELO raised at which it closes
///   # export CURVE_POOL_FEE=10000     # the ELO/coin tier. Default 1%
///   # export CURVE_FEE_BPS=100        # trading fee. Default 1%
///   # export CURVE_PROTOCOL_SHARE=1000# our share OF that fee. Default 10%
///   # export CURVE_MAX_GAP_BPS=500    # refuse if the two prices differ by more
///
/// THE ORDER, and only step 1 is irreversible:
///
///   1. this script                                   -> EloCurve
///   2. transfer CURVE_ALLOC + CURVE_POOL_ALLOC of the
///      coin to it                                    (FUNDED, never minted)
///   3. anyone calls arm()                            -> it trades
///   4. buys accumulate; anyone calls graduate() once closed
///   5. optionally DeployLaunch + DeployGameTicket for the copy sale, which
///      pairs INTO the pool the curve opened. `EloLaunchpad.seed()` works
///      unchanged after graduation: `createAndInitializePoolIfNecessary`
///      returns the existing pool and the vault mints its own position at the
///      live price, so the `sqrtPriceX96` argument is ignored.
///
/// ⚠ THE GAP IS THE ONE NUMBER TO ARGUE ABOUT. The curve closes at a marginal
/// price set by the virtual reserve, the allocation and the threshold; the pool
/// opens at `threshold / poolAlloc`. If the pool opens far below the curve's
/// close, the last buyer on the curve eats the difference the moment trading
/// moves. This script computes both from the constructor's own figures and
/// refuses over `CURVE_MAX_GAP_BPS`, because that check is worthless after the
/// broadcast — nothing here has a setter.
///
/// ⚠ NOT FORK-TESTED. `EloCurve`'s suite runs against MOCK v3. It pins this
/// contract's own arithmetic and its exits; it proves nothing about tick math,
/// the ratio a real mint consumes, or what `collect` returns. Fork against the
/// canonical addresses before this holds a wei.
contract DeployCurve is Script {
    function run() external {
        address coin = vm.envAddress("CURVE_COIN");
        address quote = vm.envAddress("CURVE_QUOTE");
        address npm = vm.envAddress("CURVE_NPM");
        address creatorFees = vm.envAddress("CURVE_CREATOR_FEES");
        address protocolFees = vm.envAddress("CURVE_PROTOCOL_FEES");
        address quotePoolFees = vm.envAddress("CURVE_QUOTE_POOL_FEES");
        address coinPoolFees = vm.envAddress("CURVE_COIN_POOL_FEES");

        uint256 virtualQuote = _mustUint("CURVE_VIRTUAL_QUOTE");
        uint256 curveAlloc = _mustUint("CURVE_ALLOC");
        uint256 poolAlloc = _mustUint("CURVE_POOL_ALLOC");
        uint256 threshold = _mustUint("CURVE_THRESHOLD");

        uint256 poolFee = vm.envOr("CURVE_POOL_FEE", uint256(10000));
        uint256 feeBps = vm.envOr("CURVE_FEE_BPS", uint256(100));
        uint256 protocolShare = vm.envOr("CURVE_PROTOCOL_SHARE", uint256(1000));
        uint256 maxGapBps = vm.envOr("CURVE_MAX_GAP_BPS", uint256(500));

        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        EloCurve curve = new EloCurve(
            IERC20(coin),
            IERC20(quote),
            ICurvePositionManager(npm),
            uint24(poolFee),
            virtualQuote,
            curveAlloc,
            poolAlloc,
            threshold,
            uint16(feeBps),
            uint16(protocolShare),
            creatorFees,
            protocolFees,
            quotePoolFees,
            coinPoolFees
        );
        vm.stopBroadcast();

        (uint256 closePrice, uint256 openPrice, uint256 coinLeft) = curve.graduationPreview();

        console2.log("EloCurve", address(curve));
        console2.log("  quote (the reserve - ELO)", quote);
        console2.log("  coin", coin);
        console2.log("  opening price, reserve per coin x1e18", (virtualQuote * 1e18) / curveAlloc);
        console2.log("  curve closes at, reserve per coin x1e18", closePrice);
        console2.log("  pool opens at,   reserve per coin x1e18", openPrice);
        console2.log("  coin left on the curve at close (BURNED at graduation)", coinLeft);
        console2.log("  fund it with", curveAlloc + poolAlloc);
        console2.log("  NEXT: transfer that coin here, then anyone calls arm()");

        uint256 gap = closePrice > openPrice ? closePrice - openPrice : openPrice - closePrice;
        uint256 gapBps = closePrice == 0 ? 0 : (gap * 10_000) / closePrice;
        console2.log("  gap between the two prices, bps", gapBps);
        if (openPrice < closePrice) {
            console2.log("  ^ the pool opens BELOW the curve's close - the last buyer eats this");
        }
        require(gapBps <= maxGapBps, "the two prices are too far apart - resize poolAlloc, or raise CURVE_MAX_GAP_BPS deliberately");
    }

    /// Demand the figure rather than defaulting it. Every one of these is
    /// immutable, so an omission would weld a number nobody chose — which is
    /// exactly how `SEAT_RESERVED_UNTIL` nearly welded "never expires" in
    /// silence (`SENTENCES.md` 2026-08-12).
    function _mustUint(string memory key) internal view returns (uint256 v) {
        v = vm.envUint(key);
        require(v > 0, string.concat(key, " is unset, and it is welded at construction"));
    }
}

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../src/ScryBank.sol";
import "../src/ScryFeeSplitter.sol";
import "../src/ScryHarvest.sol";

/// Minimal ERC-20 standing in for the fair-launched SCRY.
contract MockSCRY {
    string public constant name = "scry";
    string public constant symbol = "SCRY";
    uint8 public constant decimals = 18;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        totalSupply += amount;
        balanceOf[to] += amount;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "bal");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(allowance[from][msg.sender] >= amount, "allow");
        require(balanceOf[from] >= amount, "bal");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }
}

contract ScryBankTest is Test {
    MockSCRY scry;
    ScryBank bank;
    address alice = address(0xA11CE);
    address bob = address(0xB0B);
    address games = address(0x6A3E5);

    function setUp() public {
        scry = new MockSCRY();
        bank = new ScryBank(IERC20(address(scry)));
        scry.mint(alice, 1_000_000e18);
        scry.mint(bob, 1_000_000e18);
        scry.mint(games, 1_000_000e18);
        vm.prank(alice);
        scry.approve(address(bank), type(uint256).max);
        vm.prank(bob);
        scry.approve(address(bank), type(uint256).max);
    }

    /// xSCRY to address(0) would leave `totalSupply` unchanged while the shares
    /// became unredeemable — an invisible donation to every other holder,
    /// wearing the shape of a transfer. `SpoilsToken._move` is byte-identical
    /// to this one except for this guard; the bank was the odd one out, and it
    /// welds with no admin so the guard went in pre-broadcast (2026-07-28).
    function test_transferToZeroIsRefused() public {
        vm.prank(alice);
        uint256 shares = bank.enter(100e18);
        vm.prank(alice);
        vm.expectRevert(bytes("zero to"));
        bank.transfer(address(0), shares);
    }

    /// The empty-bank guard on `leave()` had no test: every `leave()` in the
    /// suite was preceded by an `enter()` in the same test, so the branch was
    /// never entered. Cosmetic — it upgrades a panic 0x12 to a message and
    /// moves no value either way — but an untested guard is an unpinned one.
    function test_leaveOnAnEmptyBankRevertsWithAMessage() public {
        vm.prank(alice);
        vm.expectRevert(bytes("bank is empty"));
        bank.leave(1);
    }

    function test_enterLeaveRoundTrip() public {
        vm.prank(alice);
        uint256 shares = bank.enter(100e18);
        assertEq(shares, 100e18 - bank.MINIMUM_SHARES());
        vm.prank(alice);
        uint256 out = bank.leave(shares);
        // loses only the locked minimum-shares slice of the seed
        assertApproxEqAbs(out, 100e18, bank.MINIMUM_SHARES() + 1);
        assertEq(bank.balanceOf(alice), 0);
    }

    function test_feeInflowRaisesRedemption() public {
        vm.prank(alice);
        uint256 aliceShares = bank.enter(1000e18);
        uint256 rateBefore = bank.redemptionRate();
        // a game routes rake into the bank — plain transfer, no call needed
        vm.prank(games);
        scry.transfer(address(bank), 500e18);
        assertGt(bank.redemptionRate(), rateBefore);
        vm.prank(alice);
        uint256 out = bank.leave(aliceShares);
        assertGt(out, 1000e18); // alice's stake grew from fees alone
    }

    function test_proRataBetweenStakers() public {
        vm.prank(alice);
        bank.enter(1000e18);
        vm.prank(bob);
        uint256 bobShares = bank.enter(1000e18);
        vm.prank(games);
        scry.transfer(address(bank), 200e18);
        vm.prank(bob);
        uint256 bobOut = bank.leave(bobShares);
        // bob holds just under half the shares (alice seeded the locked
        // minimum), so just under half the 200 fee accrues to him
        assertApproxEqRel(bobOut, 1100e18, 0.001e18);
    }

    function test_seedTooSmallReverts() public {
        vm.prank(alice);
        vm.expectRevert(bytes("seed too small"));
        bank.enter(999);
    }

    function test_leaveWithoutSharesReverts() public {
        // renamed from the vacuous `test_noAdminSurface` (TEST-AUDIT.md): what
        // it actually pins is that a holder of zero shares cannot pull from
        // the pool — the bank's only outflow is a share burn, and the burn's
        // balance check is the guard
        vm.prank(alice);
        bank.enter(1000e18);
        vm.prank(bob);
        vm.expectRevert(bytes("balance"));
        bank.leave(1); // bob has no shares
    }

    // ── conservation: what leaves the bank == what the user receives ==
    //    the exact pro-rata slice, and the shares actually burn ───────────
    function test_leaveConservation() public {
        vm.prank(alice);
        uint256 shares = bank.enter(1000e18);
        vm.prank(games);
        scry.transfer(address(bank), 500e18); // service fees swell the pool
        uint256 poolBefore = scry.balanceOf(address(bank));
        uint256 tsBefore = bank.totalSupply();
        uint256 aliceBefore = scry.balanceOf(alice);
        vm.prank(alice);
        uint256 out = bank.leave(shares);
        // exact pro-rata: (1000e18-1000 shares) * 1500e18 pool / 1000e18 supply
        assertEq(out, 1500e18 - 1500);
        assertEq(scry.balanceOf(alice) - aliceBefore, out); // user-received
        assertEq(poolBefore - scry.balanceOf(address(bank)), out); // bank delta
        assertEq(bank.balanceOf(alice), 0); // shares burned
        assertEq(tsBefore - bank.totalSupply(), shares);
        uint256 minShares = bank.MINIMUM_SHARES();
        assertEq(bank.totalSupply(), minShares); // only the locked seed remains
    }

    // ── the donation/inflation attack the MINIMUM_SHARES comment claims to
    //    defend against — proven, both arms ───────────────────────────────
    function test_donationAttackIsUneconomic() public {
        address attacker = address(0xA77AC2);
        scry.mint(attacker, 101e18);
        vm.startPrank(attacker);
        scry.approve(address(bank), type(uint256).max);
        bank.enter(1001); // attacker keeps 1 share; 1000 locked to the bank
        scry.transfer(address(bank), 100e18); // donate: 1 share ≈ 0.1 SCRY now
        vm.stopPrank();
        // victim deposits 100e18: shares = 100e18 * 1001 / (100e18 + 1001)
        // = 1000 exactly — NOT rounded to zero
        vm.prank(bob);
        uint256 victimShares = bank.enter(100e18);
        assertEq(victimShares, 1000);
        uint256 bobBefore = scry.balanceOf(bob);
        vm.prank(bob);
        uint256 victimOut = bank.leave(victimShares);
        // the victim's rounding loss is bounded by ONE share's value (~0.1
        // SCRY here): they exit with >= deposit minus dust, never a wipeout
        assertGe(victimOut, 100e18 - 0.1e18);
        assertEq(scry.balanceOf(bob) - bobBefore, victimOut);
        // and the attack is a money-loser: the attacker's single share
        // redeems a sliver of the 100e18 donated — the locked MINIMUM_SHARES
        // (1000 of 1001 supply) absorbed the rest, unrecoverably
        vm.prank(attacker);
        uint256 attackerOut = bank.leave(1);
        assertLt(attackerOut, 1e18); // got back < 1 of the 100 donated
    }

    function test_donationCannotMintZeroSharesToVictim() public {
        // the other arm: when the donation is so large a deposit WOULD round
        // to zero shares, enter() refuses outright instead of silently
        // minting nothing and gifting the deposit to earlier holders
        address attacker = address(0xA77AC2);
        scry.mint(attacker, 1_000_001e18);
        vm.startPrank(attacker);
        scry.approve(address(bank), type(uint256).max);
        bank.enter(1001);
        scry.transfer(address(bank), 1_000_000e18); // 100e18*1001/1e24 → 0
        vm.stopPrank();
        vm.prank(bob);
        vm.expectRevert(bytes("amount too small for pool"));
        bank.enter(100e18);
    }

    // ── tiny share burn against a big pool: pinned — the payout floors but
    //    can never be zero. Because the pool starts 1:1 with supply and fee
    //    inflows/round-downs only raise pool/totalSupply, pool >= totalSupply
    //    always, so shares*pool/totalSupply >= shares >= 1 wei. The sub-share
    //    remainder stays in the pool for the other stakers — floored, not
    //    destroyed. ─────────────────────────────────────────────────────────
    function test_leaveTinySharesNeverPaysZero() public {
        vm.prank(alice);
        bank.enter(1000e18);
        vm.prank(games);
        scry.transfer(address(bank), 500_000e18); // 501 SCRY-wei per share-wei
        uint256 bankBefore = scry.balanceOf(address(bank));
        vm.prank(alice);
        uint256 out = bank.leave(1); // the tiniest possible burn
        assertEq(out, 501); // floor(1 * 501000e18 / 1000e18) — nonzero
        assertEq(bankBefore - scry.balanceOf(address(bank)), 501);
        assertEq(bank.balanceOf(alice), 1000e18 - 1000 - 1);
    }

    // ── THE SEED HAZARD. A LAUNCH rule, not a contract bug — and the one
    //    thing about this bank that costs real money if it is misread. ──────

    /// `TREASURY.md` §P9 earmarks 9,000,000 SCRY to "swell the pool" by plain
    /// transfer, and reasons about the exposure as a SNIPE — enter just before
    /// the transfer, leave just after, capture a pro-rata slice. Into an EMPTY
    /// bank that framing is wrong in a way that matters: the loss is not a
    /// slice, it is the whole tranche, and it needs no timing at all.
    ///
    /// `enter()` reads `pool` BEFORE the pull, and the `totalSupply == 0`
    /// branch mints `amount - MINIMUM_SHARES` no matter what is already
    /// sitting in the contract. So the first depositor's shares are priced 1:1
    /// against their own deposit while the pool those shares redeem against
    /// holds the donation too. One SCRY takes the nine million.
    ///
    /// MINIMUM_SHARES is no defence here and was never claimed to be: 1000
    /// WEI of shares against a 1e18 deposit is a rounding artefact, not a
    /// haircut. Pinned as a test so the seed tooling's refuse-while-empty gate
    /// can never be removed as paranoia.
    function test_seedIntoAnEmptyBankIsCapturedWholeByTheFirstDepositor() public {
        uint256 seed = 9_000_000e18;
        scry.mint(games, seed);
        vm.prank(games);
        scry.transfer(address(bank), seed); // the §P9 shape, into an empty bank
        assertEq(bank.totalSupply(), 0, "no stakers yet");

        vm.prank(alice);
        uint256 shares = bank.enter(1e18); // one SCRY is the whole entry price
        assertEq(shares, 1e18 - bank.MINIMUM_SHARES());

        vm.prank(alice);
        uint256 out = bank.leave(shares);
        assertGt(out, seed, "the depositor took more than the whole tranche");
        // she took the seed AND her own deposit back, less only the locked
        // MINIMUM_SHARES sliver — under 0.01 SCRY of the 9,000,001 moved
        assertApproxEqAbs(out, seed + 1e18, 0.01e18);
    }

    /// The safe shape, and the reason the rule is "never transfer while the
    /// bank is empty" rather than "never transfer a lump": with a stake
    /// already in place, the tranche raises the rate for whoever was there,
    /// and a depositor arriving afterwards buys in AT the raised rate and
    /// captures none of it.
    function test_seedAfterAStakeAccruesProRataInsteadOfBeingCaptured() public {
        vm.prank(alice);
        uint256 aliceShares = bank.enter(100_000e18);

        uint256 seed = 900_000e18;
        scry.mint(games, seed);
        vm.prank(games);
        scry.transfer(address(bank), seed); // rate is now ~10 SCRY per xSCRY

        vm.prank(bob);
        uint256 bobShares = bank.enter(100_000e18);
        vm.prank(bob);
        uint256 bobOut = bank.leave(bobShares);
        assertApproxEqRel(bobOut, 100_000e18, 0.0001e18, "bob paid the raised rate");

        vm.prank(alice);
        uint256 aliceOut = bank.leave(aliceShares);
        // the whole tranche accrued to the stake that was actually there
        assertApproxEqRel(aliceOut, 1_000_000e18, 0.0001e18);
    }

    // ── the ERC-20 face of xSCRY: transfer / approve / transferFrom ──────
    function test_xscryErc20Face() public {
        vm.prank(alice);
        uint256 shares = bank.enter(1000e18);
        vm.prank(alice);
        bank.transfer(bob, 400e18);
        assertEq(bank.balanceOf(bob), 400e18);
        assertEq(bank.balanceOf(alice), shares - 400e18);
        vm.prank(bob);
        bank.approve(alice, 150e18);
        assertEq(bank.allowance(bob, alice), 150e18);
        vm.prank(alice);
        bank.transferFrom(bob, games, 150e18);
        assertEq(bank.balanceOf(games), 150e18);
        assertEq(bank.balanceOf(bob), 250e18);
        assertEq(bank.allowance(bob, alice), 0); // finite allowance spent
        vm.prank(alice);
        vm.expectRevert(bytes("allowance"));
        bank.transferFrom(bob, games, 1);
        vm.prank(bob);
        vm.expectRevert(bytes("balance"));
        bank.transfer(alice, 251e18);
    }
}

/// SCRY variant whose transfer RETURNS FALSE for one cursed recipient (a
/// frozen/blacklisting token, or a contract that refuses receipt) — used to
/// pin FeeSplitter's documented all-or-nothing distribution.
contract MockSCRYCursed {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    address public cursed;

    function setCursed(address a) external {
        cursed = a;
    }

    function mint(address to, uint256 a) external {
        balanceOf[to] += a;
    }

    function transfer(address to, uint256 a) external returns (bool) {
        if (to == cursed) return false; // receipt fails, no revert (return-false path)
        require(balanceOf[msg.sender] >= a, "bal");
        balanceOf[msg.sender] -= a;
        balanceOf[to] += a;
        return true;
    }

    function transferFrom(address f, address t, uint256 a) external returns (bool) {
        require(allowance[f][msg.sender] >= a, "allow");
        require(balanceOf[f] >= a, "bal");
        allowance[f][msg.sender] -= a;
        balanceOf[f] -= a;
        balanceOf[t] += a;
        return true;
    }

    function approve(address s, uint256 a) external returns (bool) {
        allowance[msg.sender][s] = a;
        return true;
    }
}

contract ScryFeeSplitterTest is Test {
    MockSCRY scry;
    ScryFeeSplitter split;
    address bankSink = address(0xBA2C);
    address prizes = address(0x9219E5);
    address ops = address(0x095);
    address payer = address(0x9A7E2);

    function setUp() public {
        scry = new MockSCRY();
        address[] memory to = new address[](3);
        uint16[] memory bps = new uint16[](3);
        to[0] = bankSink;
        bps[0] = 5000;
        to[1] = prizes;
        bps[1] = 3000;
        to[2] = ops;
        bps[2] = 2000;
        split = new ScryFeeSplitter(IERC20(address(scry)), to, bps);
        scry.mint(payer, 1_000_000e18);
    }

    function test_distributeSplitsByBps() public {
        vm.prank(payer);
        scry.transfer(address(split), 1000e18);
        split.distribute();
        assertEq(scry.balanceOf(bankSink), 500e18);
        assertEq(scry.balanceOf(prizes), 300e18);
        assertEq(scry.balanceOf(ops), 200e18);
        assertEq(scry.balanceOf(address(split)), 0);
    }

    // ── native value: the ERC-2981 path (2026-07-28) ────────────────────────

    /// This contract is `ScryEidolon`'s royalty receiver, welded IMMUTABLE from
    /// that constructor. With no `receive()` a marketplace pushing a native
    /// royalty made a zero-calldata call that reverted — failing the sale, or
    /// silently dropping the royalty. `DeployGacha` already probes for exactly
    /// this and refuses to broadcast; `DeployScryEidolon` has no such probe.
    function test_nativeRoyaltyPushIsAccepted() public {
        vm.deal(payer, 10 ether);
        vm.prank(payer);
        (bool ok,) = address(split).call{value: 1 ether}("");
        assertTrue(ok, "a native royalty push must not revert the sale");
        assertEq(address(split).balance, 1 ether);
    }

    /// Native value is NOT covered by the posted split — the split is
    /// denominated in SCRY, and pushing native to four outlets would let one
    /// reverting fallback brick distribution for all of them. It leaves by the
    /// operator's posted override, which is disclosed in HELD-KEYS.md.
    function test_nativeIsNotSweptIntoTheScrySplit() public {
        vm.deal(address(split), 3 ether);
        vm.prank(payer);
        scry.transfer(address(split), 1000e18);
        split.distribute();
        assertEq(address(split).balance, 3 ether, "distribute() never touches native");
        assertEq(scry.balanceOf(bankSink), 500e18, "and the SCRY split is unchanged");
    }

    function test_rescueNativeIsOperatorOnly() public {
        vm.deal(address(split), 2 ether);
        vm.prank(payer);
        vm.expectRevert(bytes("not operator"));
        split.rescueNative(payable(ops), 0);

        uint256 before = ops.balance;
        split.rescueNative(payable(ops), 0); // this test contract is the operator
        assertEq(ops.balance - before, 2 ether, "amount 0 pulls the full balance");
        assertEq(address(split).balance, 0);
    }

    function test_rescueNativeRefusesZeroTargetAndEmptyBalance() public {
        vm.expectRevert(bytes("zero to"));
        split.rescueNative(payable(address(0)), 0);
        vm.expectRevert(bytes("nothing to rescue"));
        split.rescueNative(payable(ops), 0);
    }

    function test_dustGoesToLastRecipient() public {
        vm.prank(payer);
        scry.transfer(address(split), 7); // indivisible
        split.distribute();
        assertEq(scry.balanceOf(address(split)), 0); // nothing sticks
        assertEq(scry.balanceOf(bankSink) + scry.balanceOf(prizes) + scry.balanceOf(ops), 7);
    }

    // ── THE FURNACE (FEES.md §3.1) ────────────────────────────────────────
    // The SCRY half of the burn is "just another recipient", which is exactly
    // why it needs tests: an advertised burn rate that can drift from what
    // distribute() actually does is worse than having no burn at all.

    /// A split with a BURN line really sends SCRY to 0xdEaD, and says so.
    function test_burnCutSendsToDeadAndAccounts() public {
        ScryFeeSplitter b = _burnSplit(5000, 5000); // 50% burn / 50% ops
        assertEq(b.burnBps(), 5000, "advertised rate");
        assertEq(b.totalBurned(), 0, "nothing burned before a distribute");

        vm.prank(payer);
        scry.transfer(address(b), 1000e18);
        vm.expectEmit(true, true, true, true);
        emit ScryFeeSplitter.Burned(500e18, 500e18);
        b.distribute();

        assertEq(scry.balanceOf(b.BURN()), 500e18, "the coin is at 0xdEaD");
        assertEq(b.totalBurned(), 500e18, "cumulative");

        // it accumulates rather than overwriting
        vm.prank(payer);
        scry.transfer(address(b), 1000e18);
        b.distribute();
        assertEq(b.totalBurned(), 1000e18);
        assertEq(scry.balanceOf(b.BURN()), 1000e18);
    }

    /// THE ROW. The advertised rate is READ OFF the posted split, so it cannot
    /// drift from behaviour. Repost the split without a burn and burnBps() must
    /// go to 0 — a stored-beside-it rate would still be claiming 50%.
    function test_burnBpsTracksTheSplitAndCannotGoStale() public {
        ScryFeeSplitter b = _burnSplit(5000, 5000);
        assertEq(b.burnBps(), 5000);

        address[] memory to = new address[](1);
        uint16[] memory bps = new uint16[](1);
        to[0] = ops;
        bps[0] = 10000;
        b.setSplit(to, bps);

        assertEq(b.burnBps(), 0, "the burn is gone and the number says so");

        vm.prank(payer);
        scry.transfer(address(b), 1000e18);
        b.distribute();
        assertEq(scry.balanceOf(b.BURN()), 0, "and nothing more is burned");
        assertEq(b.totalBurned(), 0, "the cumulative total never lied either");
    }

    /// A split with no BURN line is an honest zero, not a revert. Deploying
    /// without a burn is a valid configuration; it just may not be advertised.
    function test_noBurnLineIsAnHonestZero() public {
        assertEq(split.burnBps(), 0);
        vm.prank(payer);
        scry.transfer(address(split), 1000e18);
        split.distribute(); // must not revert
        assertEq(split.totalBurned(), 0);
    }

    /// burnBps() SUMS its lines. A split posting two burn cuts advertises the
    /// total, and `+=` vs `=` in distribute() is the difference between a
    /// counter and a bug nobody sees until the published total is wrong.
    function test_twoBurnLinesSumRatherThanOverwrite() public {
        address[] memory to = new address[](3);
        uint16[] memory bps = new uint16[](3);
        to[0] = SCRY_BURN;
        bps[0] = 3000;
        to[1] = ops;
        bps[1] = 4000;
        to[2] = SCRY_BURN;
        bps[2] = 3000;
        ScryFeeSplitter b = new ScryFeeSplitter(IERC20(address(scry)), to, bps);
        assertEq(b.burnBps(), 6000, "both lines counted");

        vm.prank(payer);
        scry.transfer(address(b), 1000e18);
        b.distribute();
        assertEq(b.totalBurned(), 600e18, "summed, not overwritten by the last");
        assertEq(scry.balanceOf(b.BURN()), 600e18);
    }

    /// The dust rule and the burn interact: BURN as the LAST recipient takes
    /// the rounding remainder, and the accounted total must equal what moved.
    function test_burnAsLastRecipientTakesDustAndAccountsIt() public {
        address[] memory to = new address[](2);
        uint16[] memory bps = new uint16[](2);
        to[0] = ops;
        bps[0] = 5000;
        to[1] = SCRY_BURN;
        bps[1] = 5000;
        ScryFeeSplitter b = new ScryFeeSplitter(IERC20(address(scry)), to, bps);

        vm.prank(payer);
        scry.transfer(address(b), 7); // indivisible
        b.distribute();
        assertEq(scry.balanceOf(address(b)), 0, "nothing sticks");
        assertEq(scry.balanceOf(b.BURN()), 4, "burn took the dust");
        assertEq(b.totalBurned(), 4, "and the counter matches the transfer");
    }

    /// The disclosure that makes the advertisement honest: rescue() bypasses
    /// the split, so the burn is the DEFAULT path and not an unbreakable one.
    /// Pinned as a test so nobody can claim otherwise without going red.
    function test_rescueBypassesTheBurnAndThatIsTheDisclosedTruth() public {
        ScryFeeSplitter b = _burnSplit(10000, 0); // 100% burn, on paper
        assertEq(b.burnBps(), 10000);

        vm.prank(payer);
        scry.transfer(address(b), 1000e18);
        b.rescue(IERC20(address(scry)), ops, 0); // operator pulls it instead

        assertEq(scry.balanceOf(b.BURN()), 0, "a 100% burn split burned nothing");
        assertEq(b.totalBurned(), 0, "and the counter does not pretend it did");
        assertEq(scry.balanceOf(ops), 1000e18);
    }

    /// bps 0 for ops => one-line split; helper keeps the cases readable.
    function _burnSplit(uint16 burnBpsIn, uint16 opsBps) internal returns (ScryFeeSplitter) {
        uint256 n = opsBps > 0 ? 2 : 1;
        address[] memory to = new address[](n);
        uint16[] memory bps = new uint16[](n);
        to[0] = SCRY_BURN;
        bps[0] = burnBpsIn;
        if (opsBps > 0) {
            to[1] = ops;
            bps[1] = opsBps;
        }
        return new ScryFeeSplitter(IERC20(address(scry)), to, bps);
    }

    function test_badBpsSumReverts() public {
        address[] memory to = new address[](1);
        uint16[] memory bps = new uint16[](1);
        to[0] = ops;
        bps[0] = 9999;
        vm.expectRevert(bytes("bps must sum to 10000"));
        new ScryFeeSplitter(IERC20(address(scry)), to, bps);
    }

    function test_setSplitDistributesUnderOldRulesFirst() public {
        vm.prank(payer);
        scry.transfer(address(split), 1000e18);
        address[] memory to = new address[](1);
        uint16[] memory bps = new uint16[](1);
        to[0] = ops;
        bps[0] = 10000;
        split.setSplit(to, bps); // held fees pay out 50/30/20 first
        assertEq(scry.balanceOf(bankSink), 500e18);
        // new fees follow the new split
        vm.prank(payer);
        scry.transfer(address(split), 100e18);
        split.distribute();
        assertEq(scry.balanceOf(ops), 200e18 + 100e18);
    }

    /// setSplit with NOTHING held must not revert. `distribute()` requires
    /// `amount > 0`, and setSplit calls it through `this.` — so without the
    /// balance check in front of it the split would be un-repostable in
    /// exactly the state where reposting is cleanest (right after a
    /// distribution, nothing owed to anyone under the old rules).
    function test_setSplitWithNothingHeldDoesNotRevert() public {
        assertEq(scry.balanceOf(address(split)), 0);
        address[] memory to = new address[](1);
        uint16[] memory bps = new uint16[](1);
        to[0] = ops;
        bps[0] = 10000;
        split.setSplit(to, bps); // must not revert "nothing to distribute"
        assertEq(split.splitLength(), 1);
    }

    /// Two cuts pointing at the SAME address are legal and silently merge:
    /// the money is right, but the POSTED TABLE then reads as more outlets
    /// than the split actually has. That is why the deploy phase demands a
    /// distinct SCRY_PRIZE_ESCROW instead of letting it default to ops — a
    /// four-line split where two lines are one wallet advertises a prize
    /// programme that is really just ops.
    function test_duplicateRecipientMergesAndOverstatesTheOutletCount() public {
        address[] memory to = new address[](3);
        uint16[] memory bps = new uint16[](3);
        to[0] = SCRY_BURN;
        bps[0] = 5000;
        to[1] = ops; // "prizes"
        bps[1] = 3000;
        to[2] = ops; // ops — the same wallet
        bps[2] = 2000;
        ScryFeeSplitter d = new ScryFeeSplitter(IERC20(address(scry)), to, bps);
        assertEq(d.splitLength(), 3, "the table posts three outlets");

        vm.prank(payer);
        scry.transfer(address(d), 1000e18);
        d.distribute();
        assertEq(scry.balanceOf(SCRY_BURN), 500e18);
        // ...but only TWO addresses were ever paid, and one took both cuts
        assertEq(scry.balanceOf(ops), 500e18);
    }

    function test_onlyOperatorSetsSplit() public {
        address[] memory to = new address[](1);
        uint16[] memory bps = new uint16[](1);
        to[0] = ops;
        bps[0] = 10000;
        vm.prank(payer);
        vm.expectRevert(bytes("not operator"));
        split.setSplit(to, bps);
    }

    // ── a zero-bps recipient is allowed (a parked slot): gets exactly 0,
    //    the others land exact, and splitLength reports the posted table ──
    function test_zeroBpsRecipientAllowedGetsNothing() public {
        address parked = address(0x9A2CED);
        address[] memory to = new address[](3);
        uint16[] memory bps = new uint16[](3);
        to[0] = bankSink;
        bps[0] = 5000;
        to[1] = parked;
        bps[1] = 0;
        to[2] = ops;
        bps[2] = 5000;
        ScryFeeSplitter zsplit = new ScryFeeSplitter(IERC20(address(scry)), to, bps);
        assertEq(zsplit.splitLength(), 3);
        vm.prank(payer);
        scry.transfer(address(zsplit), 1000e18);
        zsplit.distribute();
        assertEq(scry.balanceOf(bankSink), 500e18);
        assertEq(scry.balanceOf(parked), 0); // zero-bps slot receives nothing
        assertEq(scry.balanceOf(ops), 500e18);
        assertEq(scry.balanceOf(address(zsplit)), 0);
    }

    // ── governance surface: rescue (an operator drain that BYPASSES the
    //    split), all-or-nothing distribution, operator hand-off. ───────────

    /// rescue() is an operator drain that skips the posted split entirely —
    /// TEST-AUDIT called it the most serious untested path in the money core.
    /// Pin it tight: auth, both zero-guards, and the exact bypassing effect.
    function test_rescue_drains_bypassing_split_and_guards() public {
        address parked = address(0x9A2CED);
        scry.mint(address(split), 1_000e18);
        vm.prank(payer); // non-operator refused
        vm.expectRevert(bytes("not operator"));
        split.rescue(IERC20(address(scry)), parked, 1);
        vm.expectRevert(bytes("zero to")); // zero recipient refused
        split.rescue(IERC20(address(scry)), address(0), 1);
        // partial rescue: exact amount straight out, split untouched
        uint256 got = split.rescue(IERC20(address(scry)), parked, 400e18);
        assertEq(got, 400e18);
        assertEq(scry.balanceOf(parked), 400e18);
        assertEq(scry.balanceOf(address(split)), 600e18);
        // amount == 0 is documented "drain the full balance" (bypassing the
        // split entirely) — pin that it empties the contract to the wei.
        uint256 all = split.rescue(IERC20(address(scry)), parked, 0);
        assertEq(all, 600e18);
        assertEq(scry.balanceOf(parked), 1_000e18);
        assertEq(scry.balanceOf(address(split)), 0);
        // and with nothing left, the zero-amount path has nothing to pull
        vm.expectRevert(bytes("nothing to rescue"));
        split.rescue(IERC20(address(scry)), parked, 0);
    }

    /// distribute() is all-or-nothing: one recipient whose receipt fails
    /// bricks the WHOLE distribution, atomically. The earlier good transfer
    /// (index 0) rolls back — nothing partially settles.
    function test_distribute_all_or_nothing_when_a_recipient_fails() public {
        MockSCRYCursed c = new MockSCRYCursed();
        address good = address(0x600D);
        address bad = address(0x0BAD);
        address[] memory to = new address[](2);
        uint16[] memory bps = new uint16[](2);
        to[0] = good; // paid FIRST, must still roll back
        bps[0] = 5000;
        to[1] = bad;
        bps[1] = 5000;
        ScryFeeSplitter s = new ScryFeeSplitter(IERC20(address(c)), to, bps);
        c.setCursed(bad);
        c.mint(address(s), 1_000e18);
        vm.expectRevert(bytes("transfer failed"));
        s.distribute();
        assertEq(c.balanceOf(good), 0); // atomic: earlier cut undone
        assertEq(c.balanceOf(address(s)), 1_000e18); // pot intact, parked
    }

    /// transferOperator — hand-off + guards; the old operator loses rescue.
    function test_feeSplitter_transferOperator() public {
        vm.prank(payer);
        vm.expectRevert(bytes("not operator"));
        split.transferOperator(payer);
        vm.expectRevert(bytes("zero"));
        split.transferOperator(address(0));
        split.transferOperator(payer);
        assertEq(split.operator(), payer);
        vm.expectRevert(bytes("not operator")); // old operator refused
        split.rescue(IERC20(address(scry)), payer, 1);
        address[] memory to = new address[](1);
        uint16[] memory bps = new uint16[](1);
        to[0] = payer;
        bps[0] = 10000;
        vm.prank(payer);
        split.setSplit(to, bps); // new operator works
        assertEq(split.splitLength(), 1);
    }

    /// THE 2300-GAS STIPEND, PINNED. `test_nativeRoyaltyPushIsAccepted` above
    /// uses a plain `.call{value:}` which forwards ALL remaining gas — it
    /// proves `receive()` exists, not that it is reachable the way a royalty
    /// actually arrives. Solidity's `transfer`/`send` give the callee the bare
    /// 2300-gas stipend and nothing more, and that is what a marketplace paying
    /// an ERC-2981 royalty the old way uses.
    ///
    /// Measured 2026-07-29: the body costs 1,498 gas (one LOG2 + a 32-byte
    /// word = 1,381, plus dispatch), leaving 802 of headroom. That headroom is
    /// the whole margin, and it is not obviously safe — one more indexed field
    /// costs 375 and one more data word 256 (both still fit), but A SECOND
    /// EVENT of the same shape costs 1,381 and would push it to ~2,879. This
    /// test is what makes that a red suite instead of a dead royalty:
    /// `ScryEidolon.feeSplitter` is IMMUTABLE, so if this ever stops fitting,
    /// the payee cannot be changed and every push-royalty marketplace either
    /// fails the sale or drops the fee.
    function test_nativeRoyaltyFitsInTheBare2300Stipend() public {
        RoyaltyPusher mkt = new RoyaltyPusher();
        vm.deal(address(mkt), 10 ether);

        // `transfer` reverts on failure, so reaching the next line is the proof
        mkt.pushViaTransfer(payable(address(split)), 1 ether);
        assertEq(address(split).balance, 1 ether, "transfer() royalty did not land");

        assertTrue(mkt.pushViaSend(payable(address(split)), 1 ether), "send() royalty was refused");
        assertEq(address(split).balance, 2 ether);
    }

    /// The headroom itself, so a future edit that eats it fails HERE with a
    /// number rather than in a marketplace. A zero-value call gets no stipend,
    /// so sweeping its gas measures the body directly.
    function test_receiveBodyStaysWellUnderTheStipend() public {
        uint256 need;
        for (uint256 g = 1000; g <= 2300; g += 1) {
            (bool ok,) = address(split).call{value: 0, gas: g}("");
            if (ok) { need = g; break; }
        }
        assertGt(need, 0, "receive() does not fit in 2300 gas at all");
        assertLe(need, 1800, "receive() has grown into its stipend headroom - a royalty push is close to bricking");
    }

}

/// Stands in for a marketplace paying an ERC-2981 royalty the old way: both of
/// Solidity's stipend-capped sends, which forward exactly 2300 gas.
contract RoyaltyPusher {
    function pushViaTransfer(address payable to, uint256 amt) external {
        to.transfer(amt);
    }

    function pushViaSend(address payable to, uint256 amt) external returns (bool) {
        return to.send(amt);
    }

    receive() external payable {}
}


contract ScryHarvestTest is Test {
    MockSCRY scry;
    ScryHarvest harvest;
    address w1 = address(0x1111);
    address w2 = address(0x2222);

    function setUp() public {
        scry = new MockSCRY();
        harvest = new ScryHarvest(IERC20(address(scry)));
        scry.mint(address(harvest), 10_000e18);
    }

    function _pair(bytes32 a, bytes32 b) internal pure returns (bytes32) {
        return a < b ? keccak256(abi.encodePacked(a, b)) : keccak256(abi.encodePacked(b, a));
    }

    /// two-leaf tree: root = sortedPair(leaf1, leaf2)
    function _postTwoLeafRoot(uint256 cum1, uint256 cum2) internal returns (bytes32 leaf1, bytes32 leaf2) {
        leaf1 = keccak256(abi.encodePacked(w1, cum1));
        leaf2 = keccak256(abi.encodePacked(w2, cum2));
        harvest.postRoot(_pair(leaf1, leaf2), cum1 + cum2, "augury/ledger@test");
    }

    function test_claimAndReclaimCumulative() public {
        (, bytes32 leaf2) = _postTwoLeafRoot(40e18, 60e18);
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf2;
        uint256 got = harvest.claim(w1, 40e18, proof);
        assertEq(got, 40e18);
        assertEq(scry.balanceOf(w1), 40e18);

        // second epoch: cumulative grows to 70; claim pays the 30 delta
        (, bytes32 l2b) = _postTwoLeafRoot(70e18, 60e18);
        proof[0] = l2b;
        got = harvest.claim(w1, 70e18, proof);
        assertEq(got, 30e18);
        assertEq(scry.balanceOf(w1), 70e18);
    }

    function test_doubleClaimReverts() public {
        (, bytes32 leaf2) = _postTwoLeafRoot(40e18, 60e18);
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf2;
        harvest.claim(w1, 40e18, proof);
        vm.expectRevert(bytes("nothing to claim"));
        harvest.claim(w1, 40e18, proof);
    }

    function test_badProofReverts() public {
        _postTwoLeafRoot(40e18, 60e18);
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = bytes32(uint256(0xdead));
        vm.expectRevert(bytes("bad proof"));
        harvest.claim(w1, 40e18, proof);
    }

    function test_claimForSomeoneElsePaysThem() public {
        (, bytes32 leaf2) = _postTwoLeafRoot(40e18, 60e18);
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf2;
        vm.prank(w2); // stranger claims FOR w1
        harvest.claim(w1, 40e18, proof);
        assertEq(scry.balanceOf(w1), 40e18);
        assertEq(scry.balanceOf(w2), 0);
    }

    function test_sweepNeverBreaksPostedRoot() public {
        _postTwoLeafRoot(40e18, 60e18); // committed = 100
        // surplus = 10_000 - 100 => sweeping more than surplus reverts
        vm.expectRevert(bytes("would break the posted root"));
        harvest.sweep(address(this), 9_901e18);
        harvest.sweep(address(this), 9_900e18); // exactly surplus is fine
        assertEq(scry.balanceOf(address(this)), 9_900e18);
    }

    function test_onlyOperatorPostsRoots() public {
        vm.prank(w1);
        vm.expectRevert(bytes("not operator"));
        harvest.postRoot(bytes32(0), 0, "x");
    }

    // ── under-funded root: postRoot does NOT check funding, so the header's
    //    "a posted root is a kept promise" invariant is OPERATIONAL — the
    //    operator must fund `totalCommitted` BEFORE posting (POOLS.md §3 /
    //    RUNBOOK 7c order). A valid-proof claim beyond the balance dies
    //    inside the token (the mock's "bal"), not in a harvest guard. ──────
    function test_underFundedRootClaimHitsTokenRevert() public {
        // committed 20_060e18 against the 10_000e18 the setUp funded
        (, bytes32 leaf2) = _postTwoLeafRoot(20_000e18, 60e18);
        bytes32[] memory proof = new bytes32[](1);
        proof[0] = leaf2;
        vm.expectRevert(bytes("bal"));
        harvest.claim(w1, 20_000e18, proof);
    }

    // ── governance surface: sweep (surplus-withdraw that must never break
    //    the posted root) + operator hand-off. ─────────────────────────────

    /// sweep() lets the operator pull surplus but must never dip below what
    /// the posted root owes. With no root posted, owed==0 so the full balance
    /// is surplus; one wei over the balance is refused.
    function test_harvest_sweep_gates_and_effect() public {
        address parked = address(0x59EE7);
        vm.prank(w1); // non-operator refused
        vm.expectRevert(bytes("not operator"));
        harvest.sweep(parked, 1);
        vm.expectRevert(bytes("would break the posted root"));
        harvest.sweep(parked, 10_001e18); // over the 10_000e18 held
        harvest.sweep(parked, 4_000e18); // within surplus
        assertEq(scry.balanceOf(parked), 4_000e18);
        assertEq(scry.balanceOf(address(harvest)), 6_000e18);
    }

    /// transferOperator — hand-off + guards; old operator loses sweep/postRoot.
    function test_harvest_transferOperator() public {
        vm.prank(w1);
        vm.expectRevert(bytes("not operator"));
        harvest.transferOperator(w1);
        vm.expectRevert(bytes("zero"));
        harvest.transferOperator(address(0));
        harvest.transferOperator(w1);
        assertEq(harvest.operator(), w1);
        vm.expectRevert(bytes("not operator")); // old operator refused
        harvest.sweep(w1, 1);
        vm.prank(w1);
        harvest.sweep(w1, 1e18); // new operator works
        assertEq(scry.balanceOf(w1), 1e18);
    }

    /// PARITY: a merkle root + proof produced OFF-CHAIN by the launch airdrop
    /// planner (meter/claim_plan.py -> to_merkle) must verify under THIS
    /// contract's on-chain _verify. Fixture regenerated by claim_plan (leaf =
    /// keccak(wallet, amount_wei), sorted-pair). If the Python encoding ever
    /// drifts from the Solidity one, this claim reverts "bad proof" — so the
    /// claim page can never post a root the contract won't honor.
    function test_python_claim_plan_merkle_verifies_onchain() public {
        bytes32 root = 0x6273fcb1cef6c65ff6dd6e393efb99fc8e51aa5ebf598003250dc13063cad62b;
        address wallet = 0x00000000000000000000000000000000000000C3;
        uint256 amount = 1200000000000000000000; // 1200e18
        bytes32[] memory proof = new bytes32[](3);
        proof[0] = 0xac2c22fe88cd142ef16848a5f1377c01fa631bb64eb95638461b1bb4a814a099;
        proof[1] = 0x06c5b4309d7611cfa67ee53219f2a523fb136c47361887255b92b0c004c5e2ee;
        proof[2] = 0x1731cc8196dc7f3b3d2ac3219f987c14f9ca12756e1a0dcb78751ae7d00157cb;

        harvest.postRoot(root, amount, "claim_plan.py parity fixture");
        uint256 paid = harvest.claim(wallet, amount, proof);
        assertEq(paid, amount); // the off-chain proof verified on-chain
        assertEq(scry.balanceOf(wallet), amount);
        // a tampered amount fails the same _verify the planner's tamper-test predicts
        vm.expectRevert(bytes("bad proof"));
        harvest.claim(wallet, amount + 1, proof);
    }
}

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloBank.sol";
import "../src/EloFeeSplitter.sol";

/// Deploy the fun-layer economy spine to Robinhood Chain mainnet.
///
///   export PRIVATE_KEY=0x…          # operator deployer
///   export SCRY_TOKEN=0xDa2a4b23459e9ca88183e990802be644AcA7C4B0
///   export SCRY_OPS=0x…             # ops fee sink (defaults to deployer)
///   forge script script/DeployEloEconomy.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// This deploys the Bank + the SCRY fee splitter. The augury harvest claim
/// (EloHarvest) is the GAME coin — deploy it against the OBOL SpoilsToken in
/// the spoils arming, NEVER funded by this SCRY splitter.
///
/// ⚠ The old note here said "the GAMES pay game tokens, never SCRY"
/// (2026-07-20). "reserve can enter" REVERSED that on 2026-07-25 — SCRY is taken
/// on the game surfaces as fees, entries and bonds, so a SCRY game rake does
/// reach this splitter (`SENTENCES.md`, 2026-07-25).
///
/// THE BURN. The split now carries RESERVE_BURN (0xdEaD) as a
/// recipient, which is the entire SCRY half of the furnace — no new contract,
/// no new key (`TOKENOMICS.md`). A SCRY-denominated game rake that burns is a
/// closed loop rather than a pure recycle, and `burnBps()` makes the claim
/// checkable straight off the ABI.
///
///   SCRY_BPS_BURN=5000              # 0xdEaD — the furnace
///   SCRY_BPS_BANK=4000              # EloBank, the xSCRY flywheel
///   SCRY_BPS_PRIZES=0               # no line at open — see below
///   SCRY_BPS_OPS=1000               # SCRY_OPS
///
/// ⚠ THOSE FOUR NUMBERS WERE THE POSTED OPENING SPLIT — spoken 2026-07-27
/// (SENTENCES.md) and superseded twice since; see the five-leg note below
/// before deploying anything against them. They are still the DEFAULTS
/// here on purpose: this file is the single place the numbers live, and
/// `deploy_town.sh` reads them back OUT of these lines rather than keeping a
/// second copy that could drift. Override any of them by exporting it; the
/// phase will print that you have and name both values.
///
/// ⚠ THESE FOUR NUMBERS ARE THE SCRY-ERA SPLIT AND THE POSTED SPLIT IS NOW
/// FIVE LEGS. `SENTENCES.md` 2026-08-19 posts **10 burn / 25 bank / 25 cistern
/// / 20 ecosystem / 20 dev** for the ELO splitter, superseding 25/25/25/15/10
/// of 08-17 and the 50% below. The burn has now halved twice: it is the one
/// leg that buys no behaviour, since burn, bank and cistern all buy the reserve
/// on our own pool first and only the destination differs after the buy.
/// `EloFeeSplitter`'s constructor takes arbitrary `(address[], uint16[])`, so
/// the contract holds five legs happily; **it is THIS SCRIPT that cannot
/// express them**, because it hardcodes four env vars and `deploy_town.sh`
/// greps for exactly those four literal forms. Anyone deploying the ELO
/// splitter has to add the fifth here, or post the split with `setSplit`
/// afterwards — the cistern's address does not exist until `EloSeat` does, so
/// the second is often the honest order anyway.
///
/// Why this shape:
///   - BURN 50%. ⚠ **The reason recorded here was struck 2026-08-19.** It read:
///     "at population zero the burn is the one outlet an outsider verifies in
///     one click, and this audience screens for 'can the team take my money',
///     not for a rate" — sizing a money leg as a visible signal, which is the
///     rug-screen frame the operator dropped. The number is superseded twice
///     over (10% above); this note stays so nobody re-derives 50% from the
///     argument that produced it.
///     ⚠ And state the mechanics plainly wherever the burn is advertised: the
///     leg is a TRANSFER to `0xdEaD`, so `totalSupply()` does not fall, and
///     with fees arriving in ETH the leg is a BUYBACK — ETH buys the reserve on
///     our own pool before any of it can be burned.
///   - BANK 40%, doubled from the old 20% placeholder. The warning was that a bank
///     line this small makes xSCRY a rounding error "attached to a staking UI"
///     — true, and the reason that warning does not bite here is that no
///     surface quotes a rate at all: /bank and bank.html publish the pot and
///     the redemption rate and refuse to annualise anything.
///   - PRIZES 0, and this is a real change rather than a rounding. A prize cut
///     needs a prize ESCROW — a distinct wallet that does not exist yet — and
///     paying it to the ops wallet would post four outlets while paying three
///     (the constructor refuses that; see the require below). Prizes are
///     already funded directly from the purse (TREASURY.md P1-P9), so the
///     splitter line would be a second, currently-zero route asking for a new
///     key. Add it with setSplit when there is an escrow and a programme.
///   - OPS 10%, unchanged.
///
/// They must sum to 10000 or the constructor reverts. setSplit can repost them
/// later, and held fees always pay out under the split they were collected
/// under — so this number governs day one and everything until it is changed,
/// which is a narrower claim than "welded forever".
///
/// Set SCRY_BPS_BURN=0 and the burn line is simply absent — an honest zero, not
/// an error. Do not advertise a burn in that configuration.
///
/// SCRY_PRIZE_ESCROW must be a DIFFERENT wallet from SCRY_OPS whenever both
/// carry bps. Two cuts to one address are legal and merge silently, so the
/// posted table would advertise a prize programme that is really just ops
/// (`EloEconomy.t.sol::test_duplicateRecipientMergesAndOverstatesTheOutletCount`).
/// This script refuses that combination rather than posting it.
///
/// ⚠ THE BANK OPENS EMPTY, AND IT MUST. Do NOT transfer a seed tranche into
/// EloBank here or at any point before it has stakers. `enter()` reads the
/// pool BEFORE the pull and the first-deposit branch mints against the
/// depositor's own amount, so SCRY sitting in an empty bank belongs entirely to
/// whoever deposits first — one SCRY takes the whole tranche, and MINIMUM_SHARES
/// is a rounding artefact, not a haircut. `TREASURY.md` §P9's 9,000,000 line is a
/// trickle budget that cannot fire until `totalSupply() > 0`; the arithmetic is
/// pinned by `test_seedIntoAnEmptyBankIsCapturedWholeByTheFirstDepositor`.
///
/// ⚠ Broadcast is a deliberate human step — costs real gas, creates real
/// operator obligations (posted split, claimable roots). `forge test -vv`
/// first, always.
contract DeployEloEconomy is Script {
    function run() external {
        uint256 pk = vm.envUint("PRIVATE_KEY");
        IERC20 reserve = IERC20(vm.envAddress("SCRY_TOKEN"));
        address ops = vm.envOr("SCRY_OPS", vm.addr(pk));

        // The SCRY prize escrow (a SCRY sink for service-funded prizes) —
        // NOT the OBOL harvest claim. Defaults to ops if unset.
        address prizeEscrow = vm.envOr("SCRY_PRIZE_ESCROW", ops);

        // ⚠ deploy_town.sh GREPS these four lines for the posted numbers, so
        // the literal must stay in the form `SCRY_BPS_X", uint256(N)`. One copy
        // of a number that two things read; the shell keeps none of its own.
        uint16 bpsBurn = uint16(vm.envOr("SCRY_BPS_BURN", uint256(5000)));
        uint16 bpsBank = uint16(vm.envOr("SCRY_BPS_BANK", uint256(4000)));
        uint16 bpsPrizes = uint16(vm.envOr("SCRY_BPS_PRIZES", uint256(0)));
        uint16 bpsOps = uint16(vm.envOr("SCRY_BPS_OPS", uint256(1000)));

        // A prize line paid to the ops wallet is not a prize line. The cuts
        // would merge and the posted table would claim an outlet that does not
        // exist — welded, and unreadable as a mistake from outside.
        require(!(bpsPrizes > 0 && bpsOps > 0 && prizeEscrow == ops),
            "SCRY_PRIZE_ESCROW must differ from SCRY_OPS (or set SCRY_BPS_PRIZES=0)");

        vm.startBroadcast(pk);
        EloBank bank = new EloBank(reserve);

        // A zero-bps cut is not the same as no cut: it would post a recipient
        // that receives nothing, which reads as an outlet that exists. Omit it.
        uint256 n = (bpsBurn > 0 ? 1 : 0) + (bpsBank > 0 ? 1 : 0)
            + (bpsPrizes > 0 ? 1 : 0) + (bpsOps > 0 ? 1 : 0);
        address[] memory to = new address[](n);
        uint16[] memory bps = new uint16[](n);
        uint256 i;
        if (bpsBurn > 0) { to[i] = RESERVE_BURN; bps[i++] = bpsBurn; }
        if (bpsBank > 0) { to[i] = address(bank); bps[i++] = bpsBank; }
        if (bpsPrizes > 0) { to[i] = prizeEscrow; bps[i++] = bpsPrizes; }
        if (bpsOps > 0) { to[i] = ops; bps[i++] = bpsOps; }

        EloFeeSplitter splitter = new EloFeeSplitter(reserve, to, bps);
        vm.stopBroadcast();

        console2.log("EloBank      ", address(bank));
        console2.log("EloFeeSplitter", address(splitter));
        // Read the burn back off the contract, never echo the input: this line
        // is what the advertised rate gets quoted from.
        console2.log("  burn bps (0xdEaD)", splitter.burnBps());
        // The whole table, read back the same way — an echo of the inputs
        // proves the script's arithmetic, not the chain's state, and the
        // posted split is the thing anyone is going to audit.
        console2.log("  posted split, read back off the splitter:");
        for (uint256 j = 0; j < splitter.splitLength(); j++) {
            (address cutTo, uint16 cutBps) = splitter.cuts(j);
            console2.log("   ", cutTo, cutBps);
        }
        if (bpsBurn == 0) {
            console2.log("  NO BURN LINE IN THIS SPLIT - do not advertise a burn");
        }
        console2.log("");
        console2.log("THE BANK IS EMPTY AND STAYS EMPTY until someone stakes. Do NOT");
        console2.log("transfer a seed tranche in: with totalSupply()==0 the whole balance");
        console2.log("belongs to the first depositor, whatever they deposit. TREASURY.md P9.");
        console2.log("");
        console2.log("wire the surfaces (meter/ecosystem.config.js + watchtower/bank.config.json):");
        console2.log("  SCRY_BANK        ", address(bank));
        console2.log("  SCRY_FEE_SPLITTER", address(splitter));
        console2.log("(EloHarvest is the OBOL game-coin claim - deploy it against the OBOL");
        console2.log(" SpoilsToken in the spoils arming, never funded by this SCRY splitter)");
    }
}

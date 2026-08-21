// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/EloEidolon.sol";
import "../src/EloTicket.sol";

/// Deploy the Thousand (eidolon vessels) + their mint tickets to Robinhood
/// Chain, and weld them together in one broadcast.
///
/// BEFORE deploy — the salt ritual, which is what makes the roll checkable:
///   1. generate a long random salt; store it ONLY in the deploy box env
///      (SCRY_EIDOLON_SALT) — it must stay secret until mint-out
///   2. EIDOLON_SALT_COMMITMENT below = sha256(utf8(salt)), hex with 0x
///      (python3 -c 'import hashlib,os;print("0x"+hashlib.sha256(os.environ["SCRY_EIDOLON_SALT"].encode()).hexdigest())')
///
///   export PRIVATE_KEY=0x…
///   export SCRY_TOKEN=0xDa2a4b23459e9ca88183e990802be644AcA7C4B0   # SCRY
///   export SCRY_FEE_SPLITTER=0x…            # deployed EloFeeSplitter
///   export EIDOLON_PRICE_WEI=0              # the TICKET carries the price (ETH); 0 = no double charge
///   export EIDOLON_SALT_COMMITMENT=0x…      # sha256(salt) — see the ritual above
///   export EIDOLON_ROYALTY_BPS=500          # ERC-2981 to the splitter, ≤1000
///   export TICKET_TRIALS_CAP=250            # earned free in the Delver's Trials
///   export TICKET_SNAPSHOT_CAP=250          # holder snapshot, priced window
///   export TICKET_GACHA_CAP=250             # house tranche for gacha floor positions
///   export TICKET_SNAPSHOT_WINDOW=7200      # seconds the snapshot sale stays open
///   export TICKET_PROCEEDS=0x…              # immutable: the only place ticket ETH sweeps
///   export TICKET_ROYALTY_BPS=250           # ERC-2981 on secondary ticket trades, ≤1000
///   export SCRY_API_BASE=https://elopros.com/api
///   forge test -vv    # ALWAYS before broadcast (repo rule)
///   forge script script/DeployEloEidolon.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// AFTER deploy (each its own operator act, none in this script):
///   - setTrialsRoot(<meter's recomputable trials root>)   — opens door 1
///   - armSnapshot(<snapshot root>, <wei ≈ $25>, <opens>)  — door 2, 2h window
///   - mintGacha(<house wallet>, n) then escrow each ticket into EloGacha
///     with ETH backing — door 3; the escrowed backing IS the posted floor
///   - setPublicSale(<wei ≈ $100>, <whatever is left>)     — door 4, last
///   - set SCRY_EIDOLON_NFT + SCRY_TICKET_NFT on the meter (+ the same
///     SCRY_EIDOLON_SALT), restart, check GET /eidolon shows the commitment
///     matching saltCommitment() on the explorer.
contract DeployEloEidolon is Script {
    function run() external {
        vm.startBroadcast(vm.envUint("PRIVATE_KEY"));
        EloTicket tick = new EloTicket(
            vm.envUint("TICKET_TRIALS_CAP"),
            vm.envUint("TICKET_SNAPSHOT_CAP"),
            vm.envUint("TICKET_GACHA_CAP"),
            vm.envUint("TICKET_SNAPSHOT_WINDOW"),
            vm.envAddress("TICKET_PROCEEDS"),
            uint96(vm.envUint("TICKET_ROYALTY_BPS")),
            vm.envString("SCRY_API_BASE")
        );
        EloEidolon eid = new EloEidolon(
            IERC20(vm.envAddress("SCRY_TOKEN")),
            IEloTicket(address(tick)),
            vm.envAddress("SCRY_FEE_SPLITTER"),
            vm.envUint("EIDOLON_PRICE_WEI"),
            vm.envBytes32("EIDOLON_SALT_COMMITMENT"),
            uint96(vm.envUint("EIDOLON_ROYALTY_BPS")),
            vm.envString("SCRY_API_BASE")
        );
        tick.setEidolon(address(eid)); // one-shot; welded from here on
        vm.stopBroadcast();
        console2.log("EloTicket ", address(tick));
        console2.log("EloEidolon", address(eid));
    }
}

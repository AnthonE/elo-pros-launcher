// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../src/SpoilsToken.sol";

/// Deploy ONE game coin — a `SpoilsToken` — and nothing else.
///
///   export PRIVATE_KEY=0x…
///   COIN_NAME=Junk COIN_SYMBOL=JUNK COIN_CAP=0 \
///   forge script script/DeployGameCoin.s.sol \
///     --rpc-url https://rpc.mainnet.chain.robinhood.com --broadcast
///
/// ── WHY THIS EXISTS BESIDE DeploySpoils.s.sol ──────────────────────────
/// `DeploySpoils.s.sol` deploys the OBOL+MYRRH PAIR plus their Garden, in
/// one act, with the pair's names and caps baked into its env keys
/// (`OBOL_NAME`, `MYRRH_CAP`). That was right for a two-coin launch decided
/// at once. It is wrong for the current generation, where the coins arrive
/// one at a time: Junk seeds at the reserve's graduation and Orbs does not
/// (`SENTENCES.md` 2026-08-22), so a script that deploys both cannot run.
///
/// `SpoilsToken` itself was already generic — `(name, symbol, cap, minter)`,
/// nothing OBOL-specific in it — so this is a deploy lane, never a fork of
/// the token. Read `contracts/src/SpoilsToken.sol` before changing a cap:
/// the safety boundary is the MINT GATE, not the cap.
///
/// ⛔ WELDED BY THIS BROADCAST: name, symbol and cap. `cap` is
/// `immutable` and `0` means UNCAPPED forever — that is Junk's decision
/// (the play coin, in-game it is scrap), copied from OBOL. A cap set > 0 is
/// honored forever and burning never re-opens mint room, so a cap typed by
/// accident is a redeploy.
///
/// ✅ NOT welded: the minter. It starts as the deploy wallet and rotates
/// with `setMinter` — which is the whole distribution question and is
/// deliberately NOT answered here. A coin with the deploy wallet as minter
/// is a coin nothing can earn yet; that is the correct opening state, and
/// the pool can be seeded against it before the game can mint a single
/// unit.
contract DeployGameCoin is Script {
    struct CoinInputs {
        uint256 pk;
        string name;
        string symbol;
        uint256 cap;
        address minter;
    }

    function run() external {
        CoinInputs memory inp;
        inp.pk = vm.envUint("PRIVATE_KEY");
        // No defaults on name/symbol. A game coin that deploys as "Obol"
        // because nobody set an env var is a redeploy, and it would print a
        // clean summary on the way there (ONE-SHOT.md §1, the SEAT_SCRY
        // finding). `vm.envString` reverts on a missing key; that is the
        // behaviour we want, not a helpful fallback.
        inp.name = vm.envString("COIN_NAME");
        inp.symbol = vm.envString("COIN_SYMBOL");
        // 0 == uncapped is a REAL choice, so it cannot be distinguished from
        // an unset key by value. `COIN_CAP` is therefore required too.
        inp.cap = vm.envUint("COIN_CAP");
        inp.minter = vm.envOr("COIN_MINTER", vm.addr(inp.pk));
        runWith(inp);
    }

    function runWith(CoinInputs memory inp) public returns (address coin) {
        require(bytes(inp.name).length != 0, "COIN_NAME empty");
        require(bytes(inp.symbol).length != 0, "COIN_SYMBOL empty");
        require(inp.minter != address(0), "COIN_MINTER zero");

        vm.startBroadcast(inp.pk);
        SpoilsToken token = new SpoilsToken(inp.name, inp.symbol, inp.cap, inp.minter);
        vm.stopBroadcast();

        coin = address(token);
        console2.log("coin      ", coin);
        console2.log("name      ", inp.name);
        console2.log("symbol    ", inp.symbol);
        console2.log("cap (0=uncapped)", inp.cap);
        console2.log("minter    ", inp.minter);
        console2.log("");
        console2.log("record it in contracts/deployments.json, then seed with contracts/seed_junk.sh");
        console2.log("the minter is the DEPLOY WALLET until setMinter — nothing can earn this coin yet");
    }
}

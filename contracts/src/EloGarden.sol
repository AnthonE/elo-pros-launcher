// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "./IERC20.sol";
import {SafeERC20} from "./SafeERC20.sol";
import {ReentrancyGuard} from "./ReentrancyGuard.sol";

/// @title EloGarden — the playground's constant-product AMM (DFK Gardens homage)
/// @notice A minimal x·y=k pair for two PLAY tokens, LP shares ticker SEED,
///         0.3% swap fee accruing to LPs. Deliberately the boring classic —
///         sworn agents vow a management strategy ("keep the pool balanced
///         50/50", "LP and never chase") and run it here with real on-chain
///         actions and worthless stakes. The meter watches whether the
///         strategy holds when nobody's grading in real time.
///
///         ⚠ spotPrice is a raw reserve ratio and is trivially manipulable.
///         The Burrow uses it as its liquidation oracle ON PURPOSE, against
///         free-faucet PLAY tokens, where oracle games and liquidation hunts
///         are the content. NEVER READ spotPrice AS AN ORACLE FOR ANYTHING OF
///         VALUE, here or anywhere else in the town.
///
///         ⚠ WHAT MAY AND MAY NOT GO IN ONE (settled 2026-07-25; the audit
///         found the source file and the deploy scripts contradicting each
///         other, and the scripts were the half that changed):
///
///         **Canonical SCRY never goes in a Garden.** Not by default, not as
///         an option. `DeploySpoils.s.sol` used to pair both spoils against it
///         here; it now deploys ONE Garden — MYRRH/OBOL, both sides
///         house-minted — and that is the LP whose SEED shares `EloGardener`
///         farms. Real SCRY depth lives on canonical Uniswap v3, seeded by
///         `SeedSpoilsUniswapV3.s.sol` (POOLS.md §3), and nowhere else.
///
///         The reason is this contract's own shape, not a policy preference:
///         a Garden holding a pair that a deeper v3 pool also prices is a
///         permanent arb surface on which the Garden — thinner, and with no
///         concentrated liquidity — is always the weak side. House-minted,
///         elastic coins are what this venue is sized for.
///
///         ⚠ WHY THE FEE IS 0.3% AND NOT 1% (operator, 2026-07-27). This
///         venue's JOB is to be the cheap, unlisted path for a delve-sized
///         swap, and the fee is the whole mechanism. MYRRH/OBOL trades in two
///         places at launch: canonical Uniswap v3 at the 1% tier with
///         10,000,000 OBOL + 2,000,000 MYRRH, and here, with 100,000 + 20,000
///         (`DeploySpoils.s.sol`) — 1% of the depth. Against a pool 100x
///         deeper, a shallower pool only ever wins on FEE, and only up to the
///         size where its slippage eats the discount back:
///
///           fee 0.3%  ->  the Garden beats the v3 pool up to ~716 OBOL
///           fee 0.0%  ->  ~1,020 OBOL, but earns the house nothing
///           fee 1.0%  ->  NEVER, at any size, not even on dust
///
///         One delve banks ~740 OBOL (TOKENOMICS.md), so 0.3% puts the
///         crossover almost exactly at one delve's spoils: better here for a
///         player cashing out a run, worse here for anything larger, which is
///         precisely the split we want. **At a matched 1% the Garden is
///         strictly dominated forever** — equal fee plus less depth can never
///         win — and it was briefly set there on 2026-07-27 on a fee-parity
///         argument that optimised for the opposite goal.
///
///         POOLS.md 2.1's *"at this volatility LPs at 0.3% are paying for the
///         privilege of being arbed"* is a rule about **the canonical SCRY
///         market**, where LPs are strangers we invited. It does not govern
///         here: at genesis the house owns 100% of the SEED and both sides are
///         house-minted, so the party paying to be arbed is the house, on
///         coins it printed, deliberately.
///
///         What this is NOT: a Nexum lever. Under the burrow's
///         flag/liquidate split an attacker must HOLD a shove across the delay
///         window, and a wider fee band makes one dearer to open but cheaper to
///         sustain. Two-sided, and not counted either way. `DEPTH_K` is still
///         the open knob there.
///
///         ⚠ NOTHING INDEXES THIS CONTRACT, AND THAT IS THE POINT (2026-07-27).
///         There is no factory and no `PairCreated`, so no indexer discovers
///         it; `Swap(address,bool,uint256,uint256)` matches neither the v2 nor
///         the v3 signature every DEX aggregator, chart and portfolio tracker
///         keys on; there is no `Sync`, so reserves can only be polled from
///         `getReserves()`, which returns a 2-tuple where v2's returns 3.
///         Verified against the live DexScreener API, which DOES index chain
///         4663 (`api.dexscreener.com/latest/dex/tokens/…` returns the
///         SCRY/WETH pair under `chainId: "robinhood"`) — and will never
///         return this.
///
///         Two consequences, and the first is a bill: **reserve owns this pool's
///         chart, swap UI, LP UI and price display, forever**
///         (`watchtower/wallet.html` §swap is the whole swap UI there is; see
///         FARMING.md 2a). The second is the feature — an unlisted venue with a
///         cheaper fee is a path for people who know the address, which is what
///         the 0.3% above is for. **Do NOT "fix" the invisibility by adding
///         v2-shaped events.** A Garden that indexes cleanly gets listed as a
///         real MYRRH/OBOL market at 1% of the real depth, and aggregators
///         would route strangers into the thin side and print its manipulable
///         spot as the pair's price.
///
///         ⚠ SLIPPAGE GUARDS EXIST NOW, AND THEY ARE NOT A PROMISE OF SAFETY
///         (added 2026-07-25). Every state-changing call takes a `deadline`,
///         `addLiquidity` takes `minSeeds`, `removeLiquidity` takes
///         `minAmount0/1`, and `swap` already took `minOut`. That closes
///         SANDWICHING — nobody can move the price around your transaction and
///         hand you a fill you never agreed to.
///
///         It does NOT close arbitrage, and no AMM design does. When the price
///         moves, an arber rebalances this pool and the difference comes out of
///         the LPs (loss-versus-rebalancing). That is what providing liquidity
///         IS. The house running that arb itself changes WHO CAPTURES it, never
///         whether it happens — see LAUNCH-DECISIONS.md §the house as market
///         maker, which says so out loud rather than implying otherwise.
///
///         Still absent, deliberately: no flash swap, no TWAP, no router, no
///         oracle. `spotPrice` remains manipulable and must never be read as
///         one.
contract EloGarden is ReentrancyGuard {
    string public name = "elo garden seed";
    string public symbol = "SEED";

    // ── metadata admin: name/symbol, and NOTHING else ──────────────────────
    // This is the only owner door in this contract. It cannot move funds,
    // pause anything, change a rate or touch what is escrowed — it reaches
    // exactly what a wallet prints. It exists because a constant here welds
    // whichever name was current on broadcast day in forever, and this
    // platform has been renamed more than once.
    address public metadataOwner;

    event MetadataOwnerTransferred(address indexed from, address indexed to);
    event MetadataUpdated(string name, string symbol);

    modifier onlyMetadataOwner() {
        require(msg.sender == metadataOwner, "not the metadata owner");
        _;
    }

    /// Pass address(0) to renounce and freeze name/symbol forever.
    function transferMetadataOwnership(address newOwner) external onlyMetadataOwner {
        emit MetadataOwnerTransferred(metadataOwner, newOwner);
        metadataOwner = newOwner;
    }

    function setName(string calldata newName) external onlyMetadataOwner {
        name = newName;
        emit MetadataUpdated(name, symbol);
    }

    function setSymbol(string calldata newSymbol) external onlyMetadataOwner {
        symbol = newSymbol;
        emit MetadataUpdated(name, symbol);
    }
    uint8 public constant decimals = 18;

    IERC20 public immutable token0;
    IERC20 public immutable token1;
    uint112 private reserve0;
    uint112 private reserve1;

    uint256 public constant FEE_BPS = 30; // 0.3% to LPs — the unlisted-path discount
    uint256 public constant MINIMUM_LIQUIDITY = 1000;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    event Mint(address indexed to, uint256 amount0, uint256 amount1, uint256 seeds);
    event Burn(address indexed to, uint256 amount0, uint256 amount1, uint256 seeds);
    event Swap(address indexed who, bool zeroForOne, uint256 amountIn, uint256 amountOut);

    /// Every state-changing call carries a deadline. Without one a signed
    /// transaction can sit in the mempool and be executed later, at a price
    /// the sender never saw — the classic stale-transaction sandwich.
    modifier before(uint256 deadline) {
        require(block.timestamp <= deadline, "deadline passed");
        _;
    }

    constructor(IERC20 _token0, IERC20 _token1) {
        require(address(_token0) != address(_token1), "same token");
        token0 = _token0;
        token1 = _token1;
        metadataOwner = msg.sender;
    }

    function getReserves() public view returns (uint112, uint112) {
        return (reserve0, reserve1);
    }

    /// token0 priced in token1, 1e18-scaled. Raw reserve ratio — manipulable
    /// by design; see the contract notice.
    function spotPrice() external view returns (uint256) {
        require(reserve0 > 0, "empty garden");
        return (uint256(reserve1) * 1e18) / uint256(reserve0);
    }

    // ── liquidity ───────────────────────────────────────────────────────────
    /// Add liquidity and receive SEED shares. `amount0`/`amount1` are MAXIMUMS,
    /// not exact deposits: shares are priced off the short side, and only the
    /// matching amount of the long side is pulled. There is no router here to
    /// quote the optimal pair, so without this the over-supplied side was kept
    /// by the pool with no shares and no refund — a silent donation to existing
    /// LPs on every mis-ratioed add (audit 2026-07-25). Refunding by NOT
    /// PULLING is cheaper and safer than transferring the excess back.
    /// @param amount0 the MOST of token0 you are willing to deposit
    /// @param amount1 the MOST of token1 you are willing to deposit
    /// @param minSeeds the FEWEST SEED shares you will accept for them
    /// @param deadline unix time after which this call must not execute
    function addLiquidity(uint256 amount0, uint256 amount1, uint256 minSeeds, uint256 deadline)
        external
        before(deadline) nonReentrant
        returns (uint256 seeds)
    {
        require(amount0 > 0 && amount1 > 0, "zero");
        uint256 used0 = amount0;
        uint256 used1 = amount1;
        bool seeding = totalSupply == 0;
        if (seeding) {
            // The opening add sets the price, so both sides are used as given.
            seeds = _sqrt(amount0 * amount1);
            require(seeds > MINIMUM_LIQUIDITY, "seed too small");
            seeds -= MINIMUM_LIQUIDITY;
        } else {
            seeds = _min((amount0 * totalSupply) / reserve0, (amount1 * totalSupply) / reserve1);
            require(seeds > 0, "amounts too small");
            // Round the pull UP so a rounding wei always favours the pool, never
            // the minter. Both are provably <= the caller's stated maximum:
            // seeds <= floor(amountN * ts / rN)  =>  seeds * rN <= amountN * ts.
            used0 = _ceilDiv(seeds * uint256(reserve0), totalSupply);
            used1 = _ceilDiv(seeds * uint256(reserve1), totalSupply);
        }
        require(seeds >= minSeeds, "slippage: fewer SEED than minSeeds");
        // PULL BEFORE MINT (§M7, audit 2026-07-27). SEED used to be minted
        // first. The pair is a CONSTRUCTOR ARGUMENT, so "these tokens have no
        // callback" is a deployment-time assumption a future deployer cannot
        // see — the exact argument that rewrote `EloBank.enter`, applied here
        // for the same reason. The share price is read off the reserves ABOVE
        // both pulls either way, so the arithmetic is untouched; what closes is
        // the window where SEED exists against a deposit that has not landed.
        //
        // `nonReentrant` guards the pool functions but NOT this contract's own
        // ERC-20 surface — `transfer`/`approve`/`transferFrom` sit outside it,
        // as ERC-20 surfaces do — so the ordering is what actually closes this,
        // not the modifier.
        SafeERC20.pullExact(address(token0), msg.sender, address(this), used0, "t0 in");
        SafeERC20.pullExact(address(token1), msg.sender, address(this), used1, "t1 in");
        if (seeding) _mint(address(this), MINIMUM_LIQUIDITY); // locked forever
        _mint(msg.sender, seeds);
        _sync();
        emit Mint(msg.sender, used0, used1, seeds);
    }

    /// @param seeds SEED shares to burn
    /// @param minAmount0 the least token0 you will accept back
    /// @param minAmount1 the least token1 you will accept back
    /// @param deadline unix time after which this call must not execute
    function removeLiquidity(uint256 seeds, uint256 minAmount0, uint256 minAmount1, uint256 deadline)
        external
        before(deadline) nonReentrant
        returns (uint256 amount0, uint256 amount1)
    {
        require(seeds > 0, "zero");
        amount0 = (seeds * token0.balanceOf(address(this))) / totalSupply;
        amount1 = (seeds * token1.balanceOf(address(this))) / totalSupply;
        require(amount0 >= minAmount0 && amount1 >= minAmount1, "slippage: less out than the minimums");
        _burn(msg.sender, seeds);
        SafeERC20.safeTransfer(address(token0), msg.sender, amount0, "t0 out");
        SafeERC20.safeTransfer(address(token1), msg.sender, amount1, "t1 out");
        _sync();
        emit Burn(msg.sender, amount0, amount1, seeds);
    }

    // ── swaps ───────────────────────────────────────────────────────────────
    function getAmountOut(uint256 amountIn, bool zeroForOne) public view returns (uint256) {
        (uint256 rIn, uint256 rOut) =
            zeroForOne ? (uint256(reserve0), uint256(reserve1)) : (uint256(reserve1), uint256(reserve0));
        require(rIn > 0 && rOut > 0, "empty garden");
        uint256 inWithFee = amountIn * (10_000 - FEE_BPS);
        return (inWithFee * rOut) / (rIn * 10_000 + inWithFee);
    }

    function swap(uint256 amountIn, bool zeroForOne, uint256 minOut, uint256 deadline)
        external
        before(deadline) nonReentrant
        returns (uint256 out)
    {
        require(amountIn > 0, "zero");
        out = getAmountOut(amountIn, zeroForOne);
        require(out >= minOut, "slippage");
        (IERC20 tin, IERC20 tout) = zeroForOne ? (token0, token1) : (token1, token0);
        SafeERC20.pullExact(address(tin), msg.sender, address(this), amountIn, "in");
        SafeERC20.safeTransfer(address(tout), msg.sender, out, "out");
        _sync();
        emit Swap(msg.sender, zeroForOne, amountIn, out);
    }

    // ── internals ───────────────────────────────────────────────────────────
    function _sync() internal {
        uint256 b0 = token0.balanceOf(address(this));
        uint256 b1 = token1.balanceOf(address(this));
        require(b0 <= type(uint112).max && b1 <= type(uint112).max, "overflow");
        reserve0 = uint112(b0);
        reserve1 = uint112(b1);
    }

    function _mint(address to, uint256 value) internal {
        totalSupply += value;
        balanceOf[to] += value;
        emit Transfer(address(0), to, value);
    }

    function _burn(address from, uint256 value) internal {
        require(balanceOf[from] >= value, "balance");
        balanceOf[from] -= value;
        totalSupply -= value;
        emit Transfer(from, address(0), value);
    }

    function _sqrt(uint256 y) internal pure returns (uint256 z) {
        if (y > 3) {
            z = y;
            uint256 x = y / 2 + 1;
            while (x < z) {
                z = x;
                x = (y / x + x) / 2;
            }
        } else if (y != 0) {
            z = 1;
        }
    }

    function _min(uint256 a, uint256 b) internal pure returns (uint256) {
        return a < b ? a : b;
    }

    function _ceilDiv(uint256 a, uint256 b) internal pure returns (uint256) {
        return a == 0 ? 0 : (a - 1) / b + 1;
    }

    // minimal ERC-20 so SEED is visible/composable
    function transfer(address to, uint256 value) external returns (bool) {
        require(balanceOf[msg.sender] >= value, "balance");
        balanceOf[msg.sender] -= value;
        balanceOf[to] += value;
        emit Transfer(msg.sender, to, value);
        return true;
    }

    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        emit Approval(msg.sender, spender, value);
        return true;
    }

    function transferFrom(address from, address to, uint256 value) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= value, "allowance");
        if (a != type(uint256).max) allowance[from][msg.sender] = a - value;
        require(balanceOf[from] >= value, "balance");
        balanceOf[from] -= value;
        balanceOf[to] += value;
        emit Transfer(from, to, value);
        return true;
    }
}

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title SpoilsToken — earned, burnable game coin that CAN face SCRY (DFK-style)
/// @notice The on-chain mirror of the meter's spoils ledger (OBOL / MYRRH).
///         The safety boundary vs the PlayToken leak is the MINT GATE, not a
///         cap: PlayToken is a PUBLIC free faucet (anyone mints, so it drains
///         any real pool and must NEVER be pooled). SpoilsToken's mint is
///         DISTRIBUTOR-ONLY — you earn it by playing (the meter's games), and
///         it is retired by open burn (the Agora, tariffs, rakes, losses). It
///         is a reward coin, not a faucet, so a pool of SpoilsToken/SCRY is
///         sound as long as sinks keep pace with emission.
///
///         Supply is ELASTIC by default (cap == 0): it mints and burns with the
///         game economy, RWA-style, with no fixed season ceiling. A cap set
///         > 0 at deploy is still honored forever (an optional hard ceiling);
///         either way, burning retires supply and never re-opens mint room.
contract SpoilsToken {
    string public name;
    string public symbol;
    uint8 public constant decimals = 18;
    string public constant NOTICE = "earned game coin - minted by play (distributor-only), burned in the agora; "
        "elastic supply (cap 0) unless a cap was set at deploy";

    uint256 public immutable cap; // cumulative mint ceiling; 0 == uncapped
    uint256 public totalMinted; // lifetime mint (monotone)
    uint256 public totalBurned; // lifetime burn (monotone)
    uint256 public totalSupply; // circulating = minted - burned
    address public minter; // the distributor; rotatable by itself

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    event MinterRotated(address indexed from, address indexed to);

    modifier onlyMinter() {
        require(msg.sender == minter, "minter only");
        _;
    }

    constructor(string memory _name, string memory _symbol, uint256 _cap, address _minter) {
        require(_minter != address(0), "minter");
        name = _name;
        symbol = _symbol;
        cap = _cap; // 0 == UNCAPPED (elastic supply, DFK-style)
        minter = _minter;
    }

    /// Distributor-only mint. The distributor pays out the public game ledger
    /// (merkle claims); nothing else creates supply. Supply is ELASTIC by
    /// default (cap == 0): mint tracks the game economy, balanced by open burn.
    /// A cap set > 0 at deploy is still enforced forever (an optional ceiling).
    /// The safety vs the PlayToken leak is the DISTRIBUTOR gate, not a cap:
    /// this is a reward coin you earn by playing, never a public free faucet.
    function mint(address to, uint256 amount) external onlyMinter {
        require(to != address(0), "zero to");
        require(cap == 0 || totalMinted + amount <= cap, "cap exceeded");
        totalMinted += amount;
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }

    /// Anyone may burn their own spoils (the Agora's goods, the shrine, the
    /// ferryman). Burned supply is retired forever — the cap does not reopen.
    function burn(uint256 amount) external {
        _burn(msg.sender, amount);
    }

    function burnFrom(address from, uint256 amount) external {
        uint256 a = allowance[from][msg.sender];
        require(a >= amount, "allowance");
        if (a != type(uint256).max) allowance[from][msg.sender] = a - amount;
        _burn(from, amount);
    }

    /// Rotate mint authority (deployer -> merkle distributor, or retire to
    /// address(0)-adjacent burn by rotating to a dead address).
    function setMinter(address next) external onlyMinter {
        require(next != address(0), "zero minter");
        emit MinterRotated(minter, next);
        minter = next;
    }

    function transfer(address to, uint256 value) external returns (bool) {
        _move(msg.sender, to, value);
        return true;
    }

    function transferFrom(address from, address to, uint256 value) external returns (bool) {
        uint256 a = allowance[from][msg.sender];
        require(a >= value, "allowance");
        if (a != type(uint256).max) allowance[from][msg.sender] = a - value;
        _move(from, to, value);
        return true;
    }

    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        emit Approval(msg.sender, spender, value);
        return true;
    }

    function _move(address from, address to, uint256 value) internal {
        require(to != address(0), "zero to - use burn");
        require(balanceOf[from] >= value, "balance");
        balanceOf[from] -= value;
        balanceOf[to] += value;
        emit Transfer(from, to, value);
    }

    function _burn(address from, uint256 amount) internal {
        require(balanceOf[from] >= amount, "balance");
        balanceOf[from] -= amount;
        totalBurned += amount;
        totalSupply -= amount;
        emit Transfer(from, address(0), amount);
    }
}

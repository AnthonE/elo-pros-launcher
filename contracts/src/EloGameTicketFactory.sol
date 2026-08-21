// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title EloGameTicketFactory — one place that mints a title's ticket, so its code is known
/// @notice PERMISSIONLESS ON PURPOSE, and that is not a loose end. Anyone can
///         deploy a `EloGameTicket` with `forge create` today; a factory that
///         gated deployment would be gating nothing while looking like it
///         gated something. What this buys is the thing a gate cannot:
///         `deployedByFactory[addr]` is a **known-code signal** — that address
///         was built from creation code whose hash this factory checked, so a
///         stranger reading chain alone knows it runs the pinned ticket, with
///         no source-verification step and no trust in reserve.
///
///         That shape is already the house's settled answer for item contracts
///         (`ITEM-CONTRACT.md` §1b, closed 2026-08-04: *"The house publishes an
///         optional factory so `deployedByFactory[addr]` is a known-code signal,
///         and renders mint authority on the rug screen instead of holding
///         it"*). This is the same doctrine applied to the contract that takes
///         the money.
///
///         DEPLOYING IS NOT LISTING, and the two must not be confused. This
///         contract confers no place on the store, no curation, no endorsement.
///         Curation is a hand act with no queue (`GATES.md` §3) and it lives in
///         `EloGameDirectory`, which is keeper-gated. A game deployed here and
///         never listed is exactly what it looks like: a contract somebody
///         deployed.
///
///         ⚠ WHY THE CALLER HANDS IN THE CREATION CODE. The obvious shape —
///         `new EloGameTicket(...)` — embeds the ticket's whole creation code
///         in this contract, and the ticket is ~21KB on its own: the combined
///         thing measured **26,753 bytes against EIP-170's 24,576** and could
///         not be deployed at all. So the code arrives as calldata and this
///         contract checks its hash against `ticketCodeHash`, pinned immutably
///         at deploy. The signal gets STRONGER rather than weaker — the factory
///         attests one exact build, not "some ticket" — and a new ticket
///         version is a new factory with a new pinned hash, recorded in
///         `deployments.json` like everything else. It costs no more calldata
///         than deploying the ticket by hand does, because that transaction
///         carries the same bytes.
///
///         CREATE2, so an address is knowable before it exists. `predict()`
///         answers what `deploy()` would produce, which is what lets a store
///         page, a depot gate or a directory row be prepared before the
///         broadcast. Same pattern `EloPeculiumFactory` already uses here.
///
///         Holds no money, takes no fee, has no owner. There is no fee to be
///         considered and no fee to deploy (`GATES.md` §3). Nothing here is
///         payable, so nothing can accumulate, and there is no admin to rotate
///         because there is no admin.
contract EloGameTicketFactory {
    string public constant NOTICE = "deploys a game's ticket contract from one pinned build - permissionless, free, "
        "and NOT a listing: deployedByFactory is a known-code signal only. Curation lives in EloGameDirectory";

    /// keccak256 of the exact `EloGameTicket` creation code this factory will
    /// deploy, without constructor arguments. Immutable: a factory is bound to
    /// one build for its whole life, which is the entire point of the signal.
    bytes32 public immutable ticketCodeHash;

    /// Every ticket this factory has deployed. The known-code signal.
    mapping(address => bool) public deployedByFactory;

    /// Enumerable, because RPC log history is not: `eth_getLogs` is refused
    /// beyond ~100 blocks on this chain — about ten seconds at 101ms — and no
    /// RPC method enumerates. An event-only record is unreadable by anyone who
    /// was not watching live, so a mirror must be able to walk this from state
    /// (`CLAUDE.md` §traps).
    address[] private _deployments;

    /// The ticket's constructor arguments, as one struct. Solidity's stack does
    /// not fit ten of these as separate parameters, and a named struct is what
    /// a caller reads back on the explorer.
    struct TicketParams {
        string name;
        string symbol;
        string game; // the slug the depot serves
        address reserve;
        address usdg;
        address reserveSink; // the deployed EloFeeSplitter — where the SCRY leg burns
        address proceeds; // IMMUTABLE in the ticket: ETH sweeps and USDG land here
        uint256 supplyCap; // 0 = uncapped
        string baseURI; // the SITE base, used only for external_url
        address owner; // who holds the knobs from block one. 0 => msg.sender
    }

    event TicketDeployed(
        address indexed ticket, address indexed deployer, address indexed owner, string game, uint256 index
    );

    constructor(bytes32 _ticketCodeHash) {
        require(_ticketCodeHash != bytes32(0), "zero code hash");
        ticketCodeHash = _ticketCodeHash;
    }

    /// The exact bytes CREATE2 hashes: the pinned creation code with the
    /// constructor arguments appended. Public so a caller can check what they
    /// are about to deploy, and so `predict` and `deploy` cannot drift.
    function initCode(bytes calldata creationCode, TicketParams calldata p) public view returns (bytes memory) {
        require(keccak256(creationCode) == ticketCodeHash, "unexpected ticket code");
        return abi.encodePacked(
            creationCode,
            abi.encode(
                p.name,
                p.symbol,
                p.game,
                p.reserve,
                p.usdg,
                p.reserveSink,
                p.proceeds,
                p.supplyCap,
                p.baseURI,
                p.owner == address(0) ? msg.sender : p.owner
            )
        );
    }

    /// What `deploy` would produce for these arguments and this salt.
    function predict(bytes calldata creationCode, TicketParams calldata p, bytes32 salt)
        external
        view
        returns (address)
    {
        bytes32 h = keccak256(initCode(creationCode, p));
        return address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), address(this), salt, h)))));
    }

    /// Deploy a title's ticket.
    ///
    /// `p.owner` is set at BIRTH rather than handed over afterwards, and that
    /// is a change the ticket carries for this contract's sake: its handover is
    /// deliberately two-step (offer, then accept), so a contract deployer that
    /// could not name the owner up front would own every ticket it made until a
    /// human accepted — a live sale contract owned by a factory, and a listing
    /// that takes two transactions instead of one.
    function deploy(bytes calldata creationCode, TicketParams calldata p, bytes32 salt)
        external
        returns (address ticket)
    {
        address owner_ = p.owner == address(0) ? msg.sender : p.owner;
        bytes memory code = initCode(creationCode, p);
        assembly {
            ticket := create2(0, add(code, 32), mload(code), salt)
        }
        // Zero means the salt is already used at these exact arguments, or the
        // constructor reverted. Both are the caller's to fix and neither may
        // pass silently — a factory that returned address(0) would hand a
        // directory a row pointing at nothing.
        require(ticket != address(0), "create2 failed");
        deployedByFactory[ticket] = true;
        _deployments.push(ticket);
        emit TicketDeployed(ticket, msg.sender, owner_, p.game, _deployments.length - 1);
    }

    function deploymentCount() external view returns (uint256) {
        return _deployments.length;
    }

    function deploymentAt(uint256 i) external view returns (address) {
        return _deployments[i];
    }

    /// Walk the whole set from state, in pages. `start` past the end returns an
    /// empty array rather than reverting — a mirror paging to exhaustion should
    /// stop, not fail. `limit` 0 means "to the end".
    function deployments(uint256 start, uint256 limit) external view returns (address[] memory page) {
        uint256 n = _deployments.length;
        if (start >= n) return new address[](0);
        uint256 end = limit == 0 ? n : start + limit;
        if (end > n) end = n;
        page = new address[](end - start);
        for (uint256 i; i < page.length; ++i) {
            page[i] = _deployments[start + i];
        }
    }
}

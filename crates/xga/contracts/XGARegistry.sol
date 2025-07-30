// SPDX-License-Identifier: Apache-2.0 OR MIT
pragma solidity ^0.8.19;

/// @notice Development Only Usage
/// @custom:version API Interface unstable, not versioned

interface IEigenLayer {
    function isOperator(address operator) external view returns (bool);
}

contract XGARegistry {
    struct XGAOperator {
        bytes32 commitmentHash;
        bytes signature;
        uint256 registrationBlock;
        uint256 lastRewardBlock;
        bool isActive;
        uint256 accumulatedRewards;
    }
    
    // State variables
    mapping(address => XGAOperator) public operators;
    mapping(address => uint256) public penaltyRates; // 0-100 percentage
    
    IEigenLayer public immutable eigenLayer;
    address public governance;
    uint256 public constant REWARD_PER_BLOCK = 0.00001 ether;
    
    // Events
    event CommitmentRegistered(address indexed operator, bytes32 commitmentHash);
    event CommitmentUpdated(address indexed operator, bytes32 newCommitmentHash);
    // Shadow mode only - no reward claiming events
    event PenaltyApplied(address indexed operator, uint256 penaltyRate);
    
    // Modifiers
    modifier onlyGovernance() {
        require(msg.sender == governance, "Only governance");
        _;
    }
    
    modifier onlyRegistered() {
        require(operators[msg.sender].isActive, "Not registered");
        _;
    }
    
    constructor(address _eigenLayer, address _governance) {
        eigenLayer = IEigenLayer(_eigenLayer);
        governance = _governance;
    }
    
    // Register initial XGA commitment
    function registerXGACommitment(
        bytes32 commitmentHash,
        bytes calldata signature
    ) external {
        require(eigenLayer.isOperator(msg.sender), "Not EigenLayer operator");
        require(!operators[msg.sender].isActive, "Already registered");
        require(commitmentHash != bytes32(0), "Invalid commitment hash");
        require(signature.length > 0, "Invalid signature");
        
        operators[msg.sender] = XGAOperator({
            commitmentHash: commitmentHash,
            signature: signature,
            registrationBlock: block.number,
            lastRewardBlock: block.number,
            isActive: true,
            accumulatedRewards: 0
        });
        
        emit CommitmentRegistered(msg.sender, commitmentHash);
    }
    
    // Update commitment (for consolidation, increase, etc.)
    function updateXGACommitment(
        bytes32 newCommitmentHash,
        bytes calldata newSignature
    ) external onlyRegistered {
        require(newCommitmentHash != bytes32(0), "Invalid commitment hash");
        require(newSignature.length > 0, "Invalid signature");
        require(newCommitmentHash != operators[msg.sender].commitmentHash, "Same commitment");
        
        // Update rewards before changing commitment
        _updateRewards(msg.sender);
        
        // Update commitment
        operators[msg.sender].commitmentHash = newCommitmentHash;
        operators[msg.sender].signature = newSignature;
        
        emit CommitmentUpdated(msg.sender, newCommitmentHash);
    }
    
    // Shadow mode only - operators can track but not claim rewards
    // This allows monitoring potential rewards without any claiming risk
    
    // View function to get pending rewards
    function getPendingRewards(address operator) external view returns (uint256) {
        XGAOperator memory op = operators[operator];
        if (!op.isActive) return 0;
        
        uint256 blocksSinceLastUpdate = block.number - op.lastRewardBlock;
        uint256 newRewards = blocksSinceLastUpdate * REWARD_PER_BLOCK;
        
        // Apply penalties if any
        uint256 penaltyRate = penaltyRates[operator];
        if (penaltyRate > 0) {
            uint256 penalty = (newRewards * penaltyRate) / 100;
            newRewards = newRewards - penalty;
        }
        
        return op.accumulatedRewards + newRewards;
    }
    
    // Governance functions
    function applyPenalty(address operator, uint256 penaltyRate) external onlyGovernance {
        require(penaltyRate <= 100, "Invalid penalty rate");
        penaltyRates[operator] = penaltyRate;
        emit PenaltyApplied(operator, penaltyRate);
    }
    
    function removePenalty(address operator) external onlyGovernance {
        penaltyRates[operator] = 0;
        emit PenaltyApplied(operator, 0);
    }
    
    function updateGovernance(address newGovernance) external onlyGovernance {
        require(newGovernance != address(0), "Invalid address");
        governance = newGovernance;
    }
    
    // Internal function to update rewards
    function _updateRewards(address operator) internal {
        XGAOperator storage op = operators[operator];
        
        if (!op.isActive) return;
        
        uint256 blocksSinceLastUpdate = block.number - op.lastRewardBlock;
        if (blocksSinceLastUpdate == 0) return;
        
        uint256 newRewards = blocksSinceLastUpdate * REWARD_PER_BLOCK;
        
        // Apply penalties if any
        uint256 penaltyRate = penaltyRates[operator];
        if (penaltyRate > 0) {
            uint256 penalty = (newRewards * penaltyRate) / 100;
            newRewards = newRewards - penalty;
        }
        
        op.accumulatedRewards += newRewards;
        op.lastRewardBlock = block.number;
    }
    
    // No receive function needed in shadow mode - contract doesn't handle ETH
}
/// Defines all Governance accounts types
#[derive(Clone, Debug, PartialEq)]
pub enum GovernanceAccountType {
    /// 0 - Default uninitialized account state
    Uninitialized,

    /// 1 - Governance account
    Governance,

    /// 2 - Proposal account for Governance account. A single Governance account can have multiple Proposal accounts
    Proposal,

    /// 3 - Proposal voting state account. Every Proposal account has exactly one ProposalState account
    ProposalState,

    /// 4 - Vote record account for a given Proposal.  Proposal can have 0..n voting records
    VoteRecord,

    /// 5 Custom Single Signer Transaction account which holds instructions to execute for Proposal
    CustomSingleSignerTransaction,
}

impl Default for GovernanceAccountType {
    fn default() -> Self {
        GovernanceAccountType::Uninitialized
    }
}

/// What state a Proposal is in
#[derive(Clone, Debug, PartialEq)]
pub enum ProposalStateStatus {
    /// Draft
    Draft,

    /// Taking votes
    Voting,

    /// Votes complete, in execution phase
    Executing,

    /// Completed, can be rebooted
    Completed,

    /// Deleted
    Deleted,

    /// Defeated
    Defeated,
}

impl Default for ProposalStateStatus {
    fn default() -> Self {
        ProposalStateStatus::Draft
    }
}
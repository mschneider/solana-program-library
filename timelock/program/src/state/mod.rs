use crate::{
    error::LendingError,
    math::{Decimal, Rate, SCALE},
};
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use solana_program::{
    clock::{Slot, DEFAULT_TICKS_PER_SECOND, DEFAULT_TICKS_PER_SLOT, SECONDS_PER_DAY},
    entrypoint::ProgramResult,
    program_error::ProgramError,
    program_option::COption,
    program_pack::{IsInitialized, Pack, Sealed},
    pubkey::Pubkey,
    sysvar::clock::Clock,
};

const TRANSACTION_SLOTS: u8 = 10;
const TIMELOCK_VERSION: u8 = 1;

/// Global app state
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelockProgram {
    /// Version of app
    pub version: u8,
    /// program id
    pub program_id: Pubkey,
}

pub struct TimelockSet {
    /// Version of the struct
    pub version: u8,

    /// Mint that creates signatory tokens of this instruction
    /// If there are outstanding signatory tokens, then cannot leave draft state. Signatories must burn tokens (ie agree
    /// to move instruction to voting state) and bring mint to net 0 tokens outstanding. Each signatory gets 1 (serves as flag)
    pub signatory_mint: Pubkey,

    /// Admin ownership mint. One token is minted, can be used to grant admin status to a new person.
    pub admin_mint: Pubkey,

    /// Mint that creates voting tokens of this instruction
    pub voting_mint: Pubkey,

    /// Program id of the app
    pub timelock_program_id: Pubkey,

    /// Reserve state
    pub state: TimelockState,

    /// configuration values
    pub config: TimelockConfig,
}

pub enum TimelockStateStatus {
    Draft,
    Voting,
    AwaitingExecution,
    Defeated,
    Executed,
}

pub struct TimelockState {
    /// Current state of the invoked instruction account
    pub status: TimelockStateStatus,

    /// Total voting tokens minted, for use comparing to supply remaining during consensus
    pub total_voting_tokens_minted: u64,

    /// Array of pubkeys pointing at TimelockTransactions, up to 10
    pub timelock_transactions: [Pubkey; TRANSACTION_SLOTS],

    /// cross program id to invoke
    pub cross_program_id: Pubkey,
}

pub struct TimelockConfig {}

pub struct TimelockTransaction {
    /// Slot at which this will execute
    slot: u64,

    /// Executable account
    executable: Pubkey,
}
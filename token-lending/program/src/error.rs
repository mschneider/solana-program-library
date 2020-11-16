//! Error types

use num_derive::FromPrimitive;
use solana_program::{decode_error::DecodeError, program_error::ProgramError};
use thiserror::Error;

/// Errors that may be returned by the TokenLending program.
#[derive(Clone, Debug, Eq, Error, FromPrimitive, PartialEq)]
pub enum LendingError {
    /// The account cannot be initialized because it is already being used.
    #[error("Lending account already in use")]
    AlreadyInUse,
    /// Lamport balance below rent-exempt threshold.
    #[error("Lamport balance below rent-exempt threshold")]
    NotRentExempt,
    /// Invalid input passed in to instruction.
    #[error("InvalidInput")]
    InvalidInput,
    /// Invalid instruction number passed in.
    #[error("Invalid instruction")]
    InvalidInstruction,
    /// The program address provided doesn't match the value generated by the program.
    #[error("Invalid program address generated from bump seed and key")]
    InvalidProgramAddress,
    /// The owner of the input isn't set to the program address generated by the program.
    #[error("Input account owner is not the program address")]
    InvalidOwner,
    /// The owner of the account input isn't set to the correct token program id.
    #[error("Input account owner is not the correct token program id")]
    InvalidTokenProgram,
    /// The mint of the collateral token account doesn't match the liquidity mint.
    #[error("Collateral token account is not minted by liquidity mint")]
    InvalidCollateral,
    /// The provided token account has a delegate.
    #[error("Token account has a delegate")]
    InvalidDelegate,
    /// The provided token account has a close authority.
    #[error("Token account has a close authority")]
    InvalidCloseAuthority,
    /// The provided mint account has a freeze authority.
    #[error("Mint account has a freeze authority")]
    InvalidFreezeAuthority,
    /// Expected an SPL Token account
    #[error("Input token account is not valid")]
    ExpectedTokenAccount,
    /// Expected an SPL Token mint
    #[error("Input mint account is not valid")]
    ExpectedTokenMint,
    /// Expected a Serum DEX market
    #[error("Input dex market account is not valid")]
    ExpectedDexMarket,
    /// The reserve cannot be added a full pool
    #[error("Cannot add reserve to full pool")]
    PoolFull,
    /// The reserve pools must be the same
    #[error("Reserve pools do not match")]
    PoolMismatch,
    /// Reserve price is not set
    #[error("Reserve price is not set")]
    ReservePriceUnset,
    /// Reserve price is expired
    #[error("Reserve price is expired")]
    ReservePriceExpired,
    /// Token initialize mint failed
    #[error("Token initialize mint failed")]
    TokenInitializeMintFailed,
    /// Token initialize account failed
    #[error("Token initialize account failed")]
    TokenInitializeAccountFailed,
    /// Token transfer failed
    #[error("Token transfer failed")]
    TokenTransferFailed,
    /// Token mint to failed
    #[error("Token mint to failed")]
    TokenMintToFailed,
    /// Token burn failed
    #[error("Token burn failed")]
    TokenBurnFailed,
}

impl From<LendingError> for ProgramError {
    fn from(e: LendingError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

impl<T> DecodeError<T> for LendingError {
    fn type_of() -> &'static str {
        "Lending Error"
    }
}

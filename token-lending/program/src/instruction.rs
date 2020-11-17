//! Instruction types

use crate::error::LendingError;
use solana_program::{
    instruction::{AccountMeta, Instruction},
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar,
};
use std::{convert::TryInto, mem::size_of};

/// Instructions supported by the lending program.
#[repr(C)]
#[derive(Clone, Debug, PartialEq)]
pub enum LendingInstruction {
    /// Initializes a new lending market.
    ///
    ///   0. `[writable]` Lending market account.
    ///   1. `[]` Quote currency SPL Token mint. Must be initialized.
    ///   2. `[]` Rent sysvar
    ///   3. '[]` Token program id
    InitLendingMarket, // TODO: liquidation margin threshold

    /// Initializes a new lending market reserve.
    ///
    ///   0. `[writable]` Reserve account.
    ///   1. `[signer]` Lending market account.
    ///   2. `[]` Derived lending market authority ($authority).
    ///   3. `[]` Liquidity supply SPL Token account. Must NOT be empty, owned by $authority
    ///   4. `[]` Collateral token mint - uninitialized
    ///   5. `[]` Collateral token supply - uninitialized
    ///   6. `[]` Collateral token output - uninitialized
    ///   7. `[]` Clock sysvar
    ///   8. `[]` Rent sysvar
    ///   9. '[]` Token program id
    ///   10 `[optional]` Serum DEX market account. Not required for quote currency reserves. Must be initialized and match quote and base currency.
    InitReserve, // TODO: maintenance margin percent, interest rate strategy

    /// Deposit liquidity into a reserve. The output is a collateral token representing ownership
    /// of the reserve liquidity pool.
    ///
    ///   0. `[writable]` Reserve account.
    ///   1. `[]` Derived lending market authority ($authority).
    ///   2. `[writable]` Liquidity input SPL Token account. $authority can transfer $amount
    ///   3. `[writable]` Liquidity supply SPL Token account.
    ///   4. `[writable]` Collateral output SPL Token account,
    ///   5. `[writable]` Collateral SPL Token mint.
    ///   6. `[]` Clock sysvar
    ///   7. '[]` Token program id
    DepositReserveLiquidity {
        /// Amount to deposit into the reserve
        liquidity_amount: u64,
    },

    /// Withdraw tokens from a reserve. The input is a collateral token representing ownership
    /// of the reserve liquidity pool.
    ///
    ///   0. `[writable]` Reserve account.
    ///   1. `[]` Derived lending market authority ($authority).
    ///   3. `[writable]` Liquidity supply SPL Token account,
    ///   3. `[writable]` Liquidity output SPL Token account.
    ///   2. `[writable]` Collateral input SPL Token account. $authority can transfer $amount
    ///   5. `[writable]` Collateral SPL Token mint.
    ///   6. '[]` Token program id
    WithdrawReserveLiquidity {
        /// Amount of collateral to deposit in exchange for liquidity
        collateral_amount: u64,
    },

    /// Borrow tokens from a reserve by depositing collateral tokens. The number of borrowed tokens
    /// is calculated by market price. The debt obligation is tokenized.
    ///
    ///   0. `[]` Deposit reserve account.
    ///   1. `[writable]` Borrow reserve account.
    ///   2. `[]` Derived lending market authority ($authority).
    ///   3. `[writable]` Liquidity supply SPL Token account
    ///   4. `[writable]` Liquidity output SPL Token account
    ///   5. `[writable]` Collateral input SPL Token account, $authority can transfer $collateral_amount
    ///   6. `[writable]` Collateral supply SPL Token account
    ///   7. `[writable]` Obligation - uninitialized
    ///   8. `[writable]` Obligation token mint - uninitialized
    ///   9. `[writable]` Obligation token output - uninitialized
    ///   10 `[]` Obligation token owner
    ///   11 `[]` Clock sysvar
    ///   12 `[]` Rent sysvar
    ///   13 '[]` Token program id
    BorrowReserveLiquidity {
        // TODO: slippage constraint
        /// Amount of collateral to deposit
        collateral_amount: u64,
    },

    /// Repay loaned tokens to a reserve and receive collateral tokens. The obligation balance
    /// will be recalculated for interest.
    ///
    ///   0. `[writable]` Repay reserve account.
    ///   1. `[]` Withdraw reserve account.
    ///   2. `[]` Derived lending market authority ($authority).
    ///   3. `[writable]` Liquidity input SPL Token account, $authority can transfer $liquidity_amount
    ///   4. `[writable]` Liquidity supply SPL Token account
    ///   5. `[writable]` Collateral supply SPL Token account
    ///   6. `[writable]` Collateral output SPL Token account
    ///   7. `[writable]` Obligation - initialized
    ///   8. `[writable]` Obligation token mint, $authority can transfer calculated amount
    ///   9. `[writable]` Obligation token input
    ///   10 `[]` Clock sysvar
    ///   11 `[]` Token program id
    RepayReserveLiquidity {
        /// Amount of loan to repay
        liquidity_amount: u64,
    },
}

impl LendingInstruction {
    /// Unpacks a byte buffer into a [LendingInstruction](enum.LendingInstruction.html).
    pub fn unpack(input: &[u8]) -> Result<Self, ProgramError> {
        let (&tag, rest) = input
            .split_first()
            .ok_or(LendingError::InvalidInstruction)?;
        Ok(match tag {
            0 => Self::InitLendingMarket,
            1 => Self::InitReserve,
            2 => {
                let (liquidity_amount, _rest) = Self::unpack_u64(rest)?;
                Self::DepositReserveLiquidity { liquidity_amount }
            }
            3 => {
                let (collateral_amount, _rest) = Self::unpack_u64(rest)?;
                Self::WithdrawReserveLiquidity { collateral_amount }
            }
            4 => {
                let (collateral_amount, _rest) = Self::unpack_u64(rest)?;
                Self::BorrowReserveLiquidity { collateral_amount }
            }
            5 => {
                let (liquidity_amount, _rest) = Self::unpack_u64(rest)?;
                Self::RepayReserveLiquidity { liquidity_amount }
            }
            _ => return Err(LendingError::InvalidInstruction.into()),
        })
    }

    fn unpack_u64(input: &[u8]) -> Result<(u64, &[u8]), ProgramError> {
        if input.len() >= 8 {
            let (amount, rest) = input.split_at(8);
            let amount = amount
                .get(..8)
                .and_then(|slice| slice.try_into().ok())
                .map(u64::from_le_bytes)
                .ok_or(LendingError::InvalidInstruction)?;
            Ok((amount, rest))
        } else {
            Err(LendingError::InvalidInstruction.into())
        }
    }

    /// Packs a [LendingInstruction](enum.LendingInstruction.html) into a byte buffer.
    pub fn pack(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(size_of::<Self>());
        match *self {
            Self::InitLendingMarket => {
                buf.push(0);
            }
            Self::InitReserve => {
                buf.push(1);
            }
            Self::DepositReserveLiquidity { liquidity_amount } => {
                buf.push(2);
                buf.extend_from_slice(&liquidity_amount.to_le_bytes());
            }
            Self::WithdrawReserveLiquidity { collateral_amount } => {
                buf.push(3);
                buf.extend_from_slice(&collateral_amount.to_le_bytes());
            }
            Self::BorrowReserveLiquidity { collateral_amount } => {
                buf.push(4);
                buf.extend_from_slice(&collateral_amount.to_le_bytes());
            }
            Self::RepayReserveLiquidity { liquidity_amount } => {
                buf.push(5);
                buf.extend_from_slice(&liquidity_amount.to_le_bytes());
            }
        }
        buf
    }
}

/// Creates an 'InitLendingMarket' instruction.
pub fn init_lending_market(
    program_id: Pubkey,
    lending_market_pubkey: Pubkey,
    quote_token_mint: Pubkey,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(lending_market_pubkey, false),
            AccountMeta::new_readonly(quote_token_mint, false),
            AccountMeta::new_readonly(sysvar::rent::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: LendingInstruction::InitLendingMarket.pack(),
    }
}

/// Creates an 'InitReserve' instruction.
#[allow(clippy::too_many_arguments)]
pub fn init_reserve(
    program_id: Pubkey,
    reserve_pubkey: Pubkey,
    lending_market_pubkey: Pubkey,
    liquidity_supply_pubkey: Pubkey,
    collateral_mint_pubkey: Pubkey,
    collateral_supply_pubkey: Pubkey,
    collateral_output_pubkey: Pubkey,
    dex_market_pubkey: Option<Pubkey>,
) -> Instruction {
    let (lending_market_authority_pubkey, _bump_seed) =
        Pubkey::find_program_address(&[&lending_market_pubkey.to_bytes()[..32]], &program_id);
    let mut accounts = vec![
        AccountMeta::new(reserve_pubkey, false),
        AccountMeta::new_readonly(lending_market_pubkey, true),
        AccountMeta::new_readonly(lending_market_authority_pubkey, false),
        AccountMeta::new_readonly(liquidity_supply_pubkey, false),
        AccountMeta::new(collateral_mint_pubkey, false),
        AccountMeta::new(collateral_supply_pubkey, false),
        AccountMeta::new(collateral_output_pubkey, false),
        AccountMeta::new_readonly(sysvar::clock::id(), false),
        AccountMeta::new_readonly(sysvar::rent::id(), false),
        AccountMeta::new_readonly(spl_token::id(), false),
    ];

    if let Some(dex_market_pubkey) = dex_market_pubkey {
        accounts.push(AccountMeta::new_readonly(dex_market_pubkey, false));
    }

    Instruction {
        program_id,
        accounts,
        data: LendingInstruction::InitReserve.pack(),
    }
}

/// Creates a 'DepositReserveLiquidity' instruction.
#[allow(clippy::too_many_arguments)]
pub fn deposit_reserve_liquidity(
    program_id: Pubkey,
    reserve_pubkey: Pubkey,
    lending_market_authority_pubkey: Pubkey,
    liquidity_amount: u64,
    liquidity_input_pubkey: Pubkey,
    liquidity_supply_pubkey: Pubkey,
    collateral_output_pubkey: Pubkey,
    collateral_mint_pubkey: Pubkey,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(reserve_pubkey, false),
            AccountMeta::new_readonly(lending_market_authority_pubkey, false),
            AccountMeta::new(liquidity_input_pubkey, false),
            AccountMeta::new(liquidity_supply_pubkey, false),
            AccountMeta::new(collateral_output_pubkey, false),
            AccountMeta::new(collateral_mint_pubkey, false),
            AccountMeta::new_readonly(sysvar::clock::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: LendingInstruction::DepositReserveLiquidity { liquidity_amount }.pack(),
    }
}

/// Creates a 'WithdrawReserveLiquidity' instruction.
#[allow(clippy::too_many_arguments)]
pub fn withdraw_reserve_liquidity(
    program_id: Pubkey,
    reserve_pubkey: Pubkey,
    lending_market_authority_pubkey: Pubkey,
    liquidity_supply_pubkey: Pubkey,
    liquidity_output_pubkey: Pubkey,
    collateral_mint_pubkey: Pubkey,
    collateral_input_pubkey: Pubkey,
    collateral_amount: u64,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(reserve_pubkey, false),
            AccountMeta::new_readonly(lending_market_authority_pubkey, false),
            AccountMeta::new(liquidity_supply_pubkey, false),
            AccountMeta::new(liquidity_output_pubkey, false),
            AccountMeta::new(collateral_mint_pubkey, false),
            AccountMeta::new(collateral_input_pubkey, false),
            AccountMeta::new_readonly(sysvar::clock::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: LendingInstruction::WithdrawReserveLiquidity { collateral_amount }.pack(),
    }
}

/// Creates a 'BorrowReserveLiquidity' instruction.
#[allow(clippy::too_many_arguments)]
pub fn borrow_reserve_liquidity(
    program_id: Pubkey,
    collateral_amount: u64,
    collateral_input_pubkey: Pubkey,
    liquidity_output_pubkey: Pubkey,
    deposit_reserve_pubkey: Pubkey,
    deposit_reserve_collateral_mint_pubkey: Pubkey,
    deposit_reserve_liquidity_supply_pubkey: Pubkey,
    deposit_reserve_collateral_supply_pubkey: Pubkey,
    borrow_reserve_pubkey: Pubkey,
    borrow_reserve_liquidity_supply_pubkey: Pubkey,
    lending_market_authority_pubkey: Pubkey,
    obligation_pubkey: Pubkey,
    obligation_token_mint_pubkey: Pubkey,
    obligation_token_output_pubkey: Pubkey,
    obligation_token_owner_pubkey: Pubkey,
    dex_market_pubkey: Pubkey,
    dex_market_orders_pubkey: Pubkey,
    memory_pubkey: Pubkey,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(collateral_input_pubkey, false),
            AccountMeta::new(liquidity_output_pubkey, false),
            AccountMeta::new_readonly(deposit_reserve_pubkey, false),
            AccountMeta::new_readonly(deposit_reserve_collateral_mint_pubkey, false),
            AccountMeta::new(deposit_reserve_collateral_supply_pubkey, false),
            AccountMeta::new_readonly(deposit_reserve_liquidity_supply_pubkey, false),
            AccountMeta::new(borrow_reserve_pubkey, false),
            AccountMeta::new(borrow_reserve_liquidity_supply_pubkey, false),
            AccountMeta::new(obligation_pubkey, false),
            AccountMeta::new(obligation_token_mint_pubkey, false),
            AccountMeta::new(obligation_token_output_pubkey, false),
            AccountMeta::new_readonly(obligation_token_owner_pubkey, false),
            AccountMeta::new_readonly(lending_market_authority_pubkey, false),
            AccountMeta::new_readonly(dex_market_pubkey, false),
            AccountMeta::new_readonly(dex_market_orders_pubkey, false),
            AccountMeta::new_readonly(memory_pubkey, false),
            AccountMeta::new_readonly(sysvar::clock::id(), false),
            AccountMeta::new_readonly(sysvar::rent::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: LendingInstruction::BorrowReserveLiquidity { collateral_amount }.pack(),
    }
}

/// Creates a `RepayReserveLiquidity` instruction
#[allow(clippy::too_many_arguments)]
pub fn repay_reserve_liquidity(
    program_id: Pubkey,
    repay_reserve_pubkey: Pubkey,
    withdraw_reserve_pubkey: Pubkey,
    lending_market_authority_pubkey: Pubkey,
    liquidity_input_pubkey: Pubkey,
    liquidity_supply_pubkey: Pubkey,
    liquidity_amount: u64,
    collateral_supply_pubkey: Pubkey,
    collateral_output_pubkey: Pubkey,
    obligation_pubkey: Pubkey,
    obligation_mint_pubkey: Pubkey,
    obligation_output_pubkey: Pubkey,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(repay_reserve_pubkey, false),
            AccountMeta::new_readonly(withdraw_reserve_pubkey, false),
            AccountMeta::new_readonly(lending_market_authority_pubkey, false),
            AccountMeta::new(liquidity_input_pubkey, false),
            AccountMeta::new(liquidity_supply_pubkey, false),
            AccountMeta::new(collateral_supply_pubkey, false),
            AccountMeta::new(collateral_output_pubkey, false),
            AccountMeta::new(obligation_pubkey, false),
            AccountMeta::new(obligation_mint_pubkey, false),
            AccountMeta::new(obligation_output_pubkey, false),
            AccountMeta::new_readonly(sysvar::clock::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: LendingInstruction::RepayReserveLiquidity { liquidity_amount }.pack(),
    }
}

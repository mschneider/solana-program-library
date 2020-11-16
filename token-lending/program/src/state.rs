//! State types

use crate::{error::LendingError, math::Decimal};
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use solana_program::{
    clock::{DEFAULT_TICKS_PER_SECOND, DEFAULT_TICKS_PER_SLOT, SECONDS_PER_DAY},
    info,
    program_error::ProgramError,
    program_option::COption,
    program_pack::{IsInitialized, Pack, Sealed},
    pubkey::Pubkey,
    sysvar::clock::Clock,
};
use spl_token::state::{Account as TokenAccount, Mint};

/// Prices are only valid for a few slots before needing to be updated again
const PRICE_EXPIRATION_SLOTS: u64 = 5;

/// Number of slots per year
pub const SLOTS_PER_YEAR: u64 =
    DEFAULT_TICKS_PER_SECOND / DEFAULT_TICKS_PER_SLOT * SECONDS_PER_DAY * 365;

/// Lending pool state
#[repr(C)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PoolInfo {
    /// Initialized state.
    pub is_initialized: bool,
    /// Quote currency token mint.
    pub quote_token_mint: Pubkey,
}

/// Pool reserve state
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReserveInfo {
    /// Initialized state.
    pub is_initialized: bool,
    /// Pool address
    pub pool: Pubkey,
    /// Reserve liquidity
    pub liquidity_reserve: Pubkey,
    /// Reserve collateral
    /// Collateral is stored rather than burned to keep an accurate total collateral supply
    pub collateral_reserve: Pubkey,
    /// Collateral tokens are minted when liquidity is deposited in the reserve.
    /// Collateral tokens can be withdrawn back to the underlying liquidity token.
    pub collateral_mint: Pubkey,
    /// DEX market state account
    pub dex_market: COption<Pubkey>,

    /// Latest market price
    pub market_price: u64,
    /// DEX market state account
    pub market_price_updated_slot: u64,

    /// Cumulative borrow rate
    cumulative_borrow_rate: Decimal,
    /// Total borrows, plus interest
    total_borrows: Decimal,
    /// Last slot when borrow state was updated
    pub last_update_slot: u64,
}

impl ReserveInfo {
    /// Fetch the current market price
    pub fn current_market_price(&self, clock: &Clock) -> Result<u64, ProgramError> {
        if self.dex_market.is_none() {
            Ok(1) // TODO: need decimals?
        } else if self.market_price_updated_slot == 0 {
            Err(LendingError::ReservePriceUnset.into())
        } else if self.market_price_updated_slot + PRICE_EXPIRATION_SLOTS <= clock.slot {
            Err(LendingError::ReservePriceExpired.into())
        } else {
            Ok(self.market_price)
        }
    }

    /// Add new borrow amount to total borrows
    pub fn add_borrow(&mut self, borrow_amount: Decimal) {
        self.total_borrows += borrow_amount;
    }

    /// Subtract repay amount to total borrows
    pub fn subtract_repay(&mut self, repay_amount: Decimal) {
        self.total_borrows -= repay_amount;
    }

    /// Update the cumulative borrow rate for the reserve
    pub fn update_cumulative_rate(
        &mut self,
        clock: &Clock,
        reserve_token: &TokenAccount,
    ) -> Decimal {
        if self.last_update_slot == 0 {
            self.last_update_slot = clock.slot;
            self.cumulative_borrow_rate = Decimal::from(1u64);
        } else if self.total_borrows == Decimal::from(0u64) {
            self.last_update_slot = clock.slot;
        } else {
            // Optimize for this utilization rate for stable coins
            //  increase borrow rate multiplier when utilization is higher
            let optimal_utilization_rate = Decimal::new(80, 2);
            let optimal_borrow_rate = Decimal::new(4, 2);
            let base_borrow_rate = Decimal::new(0, 2);
            let max_borrow_rate = Decimal::new(30, 2);

            let total_liquidity = Decimal::from(reserve_token.amount);
            let utilization_rate: Decimal =
                self.total_borrows / (self.total_borrows + total_liquidity);
            let borrow_rate: Decimal = if utilization_rate < optimal_utilization_rate {
                // 50% should be normalized to 5/8 of the way to the optimal borrow rate
                let normalized_rate = utilization_rate / optimal_utilization_rate;
                // Borrow rate will then be 5/8 * optimal borrow rate
                normalized_rate * (optimal_borrow_rate - base_borrow_rate) + base_borrow_rate
            } else {
                let normalized_rate = (utilization_rate - optimal_utilization_rate)
                    / (Decimal::from(1) - optimal_utilization_rate);
                normalized_rate * (max_borrow_rate - optimal_borrow_rate) + optimal_borrow_rate
            };

            let slots_elapsed = Decimal::from(clock.slot - self.last_update_slot);
            let interest_rate: Decimal =
                slots_elapsed * borrow_rate / Decimal::from(SLOTS_PER_YEAR);
            let accrued_interest: Decimal = self.total_borrows * interest_rate;

            self.total_borrows += accrued_interest;
            self.cumulative_borrow_rate *= Decimal::from(1) + interest_rate;
            self.last_update_slot = clock.slot;
        }

        self.cumulative_borrow_rate
    }

    /// Return the current collateral exchange rate.
    pub fn collateral_exchange_rate(
        &self,
        clock: &Clock,
        reserve_token: &TokenAccount,
        collateral_mint: &Mint,
    ) -> Result<Decimal, ProgramError> {
        // TODO: is exchange rate fixed within a slot?
        if self.last_update_slot != clock.slot {
            info!("exchange rate needs to be updated");
            Err(LendingError::InvalidInput.into())
        } else {
            Ok((self.total_borrows + Decimal::from(reserve_token.amount))
                / Decimal::from(collateral_mint.supply))
        }
    }
}

/// Borrow obligation state
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ObligationInfo {
    /// Slot when obligation was updated. Used for calculating interest.
    pub last_update_slot: u64,
    /// Address that has the authority to repay this obligation
    pub authority: Pubkey,
    /// Amount of collateral tokens deposited for this obligation
    pub collateral_amount: u64,
    /// Reserve which collateral tokens were deposited into
    pub collateral_reserve: Pubkey,
    /// Borrow rate used for calculating interest.
    pub cumulative_borrow_rate: Decimal,
    /// Amount of tokens borrowed for this obligation plus interest
    pub borrow_amount: Decimal,
    /// Reserve which tokens were borrowed from
    pub borrow_reserve: Pubkey,
}

impl ObligationInfo {
    /// Accrue interest
    pub fn accrue_interest(
        &mut self,
        clock: &Clock,
        reserve: &ReserveInfo,
    ) -> Result<(), ProgramError> {
        if clock.slot != reserve.last_update_slot {
            info!("reserve rates need to be updated");
            return Err(LendingError::InvalidInput.into());
        }

        let slots_elapsed = Decimal::from(clock.slot - self.last_update_slot);
        let borrow_rate: Decimal =
            reserve.cumulative_borrow_rate / self.cumulative_borrow_rate - Decimal::from(1);
        let yearly_interest: Decimal = self.borrow_amount * borrow_rate;
        let accrued_interest: Decimal =
            slots_elapsed * yearly_interest / Decimal::from(SLOTS_PER_YEAR);

        self.borrow_amount += accrued_interest;
        self.cumulative_borrow_rate = reserve.cumulative_borrow_rate;
        self.last_update_slot = clock.slot;

        Ok(())
    }
}

impl Sealed for ReserveInfo {}
impl IsInitialized for ReserveInfo {
    fn is_initialized(&self) -> bool {
        self.is_initialized
    }
}

const RESERVE_LEN: usize = 221;
impl Pack for ReserveInfo {
    const LEN: usize = 221;

    /// Unpacks a byte buffer into a [ReserveInfo](struct.ReserveInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, RESERVE_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            is_initialized,
            pool,
            liquidity,
            collateral,
            collateral_mint,
            dex_market,
            market_price,
            market_price_updated_slot,
            cumulative_borrow_rate,
            total_borrows,
            last_update_slot,
        ) = array_refs![input, 1, 32, 32, 32, 32, 36, 8, 8, 16, 16, 8];
        Ok(Self {
            is_initialized: match is_initialized {
                [0] => false,
                [1] => true,
                _ => return Err(ProgramError::InvalidAccountData),
            },
            pool: Pubkey::new_from_array(*pool),
            liquidity_reserve: Pubkey::new_from_array(*liquidity),
            collateral_reserve: Pubkey::new_from_array(*collateral),
            collateral_mint: Pubkey::new_from_array(*collateral_mint),
            dex_market: unpack_coption_key(dex_market)?,
            market_price: u64::from_le_bytes(*market_price),
            market_price_updated_slot: u64::from_le_bytes(*market_price_updated_slot),
            cumulative_borrow_rate: unpack_decimal(cumulative_borrow_rate),
            total_borrows: unpack_decimal(total_borrows),
            last_update_slot: u64::from_le_bytes(*last_update_slot),
        })
    }

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, RESERVE_LEN];
        let (
            is_initialized,
            pool,
            liquidity,
            collateral,
            collateral_mint,
            dex_market,
            market_price,
            market_price_updated_slot,
            cumulative_borrow_rate,
            total_borrows,
            last_update_slot,
        ) = mut_array_refs![output, 1, 32, 32, 32, 32, 36, 8, 8, 16, 16, 8];
        is_initialized[0] = self.is_initialized as u8;
        pool.copy_from_slice(self.pool.as_ref());
        liquidity.copy_from_slice(self.liquidity_reserve.as_ref());
        collateral.copy_from_slice(self.collateral_reserve.as_ref());
        collateral_mint.copy_from_slice(self.collateral_mint.as_ref());
        pack_coption_key(&self.dex_market, dex_market);
        *market_price = self.market_price.to_le_bytes();
        *market_price_updated_slot = self.market_price_updated_slot.to_le_bytes();
        pack_decimal(self.cumulative_borrow_rate, cumulative_borrow_rate);
        pack_decimal(self.total_borrows, total_borrows);
        *last_update_slot = self.last_update_slot.to_le_bytes();
    }
}

impl Sealed for PoolInfo {}
impl IsInitialized for PoolInfo {
    fn is_initialized(&self) -> bool {
        self.is_initialized
    }
}

const POOL_LEN: usize = 33;
impl Pack for PoolInfo {
    const LEN: usize = 33;

    /// Unpacks a byte buffer into a [PoolInfo](struct.PoolInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, POOL_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (is_initialized, quote_token_mint) = array_refs![input, 1, 32];
        Ok(Self {
            is_initialized: match is_initialized {
                [0] => false,
                [1] => true,
                _ => return Err(ProgramError::InvalidAccountData),
            },
            quote_token_mint: Pubkey::new_from_array(*quote_token_mint),
        })
    }

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, POOL_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (is_initialized, quote_token_mint) = mut_array_refs![output, 1, 32];
        *is_initialized = [self.is_initialized as u8];
        quote_token_mint.copy_from_slice(self.quote_token_mint.as_ref());
    }
}

impl Sealed for ObligationInfo {}
impl IsInitialized for ObligationInfo {
    fn is_initialized(&self) -> bool {
        self.last_update_slot > 0
    }
}

const OBLIGATION_LEN: usize = 144;
impl Pack for ObligationInfo {
    const LEN: usize = 144;

    /// Unpacks a byte buffer into a [ObligationInfo](struct.ObligationInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, OBLIGATION_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            last_update_slot,
            authority,
            collateral_amount,
            collateral_reserve,
            cumulative_borrow_rate,
            borrow_amount,
            borrow_reserve,
        ) = array_refs![input, 8, 32, 8, 32, 16, 16, 32];
        Ok(Self {
            last_update_slot: u64::from_le_bytes(*last_update_slot),
            authority: Pubkey::new_from_array(*authority),
            collateral_amount: u64::from_le_bytes(*collateral_amount),
            collateral_reserve: Pubkey::new_from_array(*collateral_reserve),
            cumulative_borrow_rate: unpack_decimal(cumulative_borrow_rate),
            borrow_amount: unpack_decimal(borrow_amount),
            borrow_reserve: Pubkey::new_from_array(*borrow_reserve),
        })
    }

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, OBLIGATION_LEN];
        let (
            last_update_slot,
            authority,
            collateral_amount,
            collateral_reserve,
            cumulative_borrow_rate,
            borrow_amount,
            borrow_reserve,
        ) = mut_array_refs![output, 8, 32, 8, 32, 16, 16, 32];

        *last_update_slot = self.last_update_slot.to_le_bytes();
        authority.copy_from_slice(self.authority.as_ref());
        *collateral_amount = self.collateral_amount.to_le_bytes();
        collateral_reserve.copy_from_slice(self.collateral_reserve.as_ref());
        pack_decimal(self.cumulative_borrow_rate, cumulative_borrow_rate);
        pack_decimal(self.borrow_amount, borrow_amount);
        borrow_reserve.copy_from_slice(self.borrow_reserve.as_ref());
    }
}

// Helpers
fn pack_coption_key(src: &COption<Pubkey>, dst: &mut [u8; 36]) {
    let (tag, body) = mut_array_refs![dst, 4, 32];
    match src {
        COption::Some(key) => {
            *tag = [1, 0, 0, 0];
            body.copy_from_slice(key.as_ref());
        }
        COption::None => {
            *tag = [0; 4];
        }
    }
}

fn unpack_coption_key(src: &[u8; 36]) -> Result<COption<Pubkey>, ProgramError> {
    let (tag, body) = array_refs![src, 4, 32];
    match *tag {
        [0, 0, 0, 0] => Ok(COption::None),
        [1, 0, 0, 0] => Ok(COption::Some(Pubkey::new_from_array(*body))),
        _ => Err(ProgramError::InvalidAccountData),
    }
}

fn pack_decimal(decimal: Decimal, dst: &mut [u8; 16]) {
    *dst = decimal.to_scaled_val().to_le_bytes();
}

fn unpack_decimal(src: &[u8; 16]) -> Decimal {
    Decimal::from_scaled_val(u128::from_le_bytes(*src))
}

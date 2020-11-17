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

/// Collateral tokens are initially valued at a ratio of 5:1 (collateral:liquidity)
pub const INITIAL_COLLATERAL_RATE: u64 = 5;

/// Number of slots per year
pub const SLOTS_PER_YEAR: u64 =
    DEFAULT_TICKS_PER_SECOND / DEFAULT_TICKS_PER_SLOT * SECONDS_PER_DAY * 365;

/// Lending market state
#[repr(C)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LendingMarket {
    /// Initialized state.
    pub is_initialized: bool,
    /// Quote currency token mint.
    pub quote_token_mint: Pubkey,
}

/// Lending market reserve state
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reserve {
    /// Initialized state.
    pub is_initialized: bool,
    /// Lending market address
    pub lending_market: Pubkey,
    /// Reserve liquidity supply
    pub liquidity_supply: Pubkey,
    /// Reserve liquidity mint
    pub liquidity_mint: Pubkey,
    /// Reserve collateral supply
    /// Collateral is stored rather than burned to keep an accurate total collateral supply
    pub collateral_supply: Pubkey,
    /// Collateral tokens are minted when liquidity is deposited in the reserve.
    /// Collateral tokens can be withdrawn back to the underlying liquidity token.
    pub collateral_mint: Pubkey,

    /// Dex market state account
    pub dex_market: COption<Pubkey>,
    /// Dex market price
    pub dex_market_price: u64,
    /// Dex market price last updated
    pub dex_market_price_updated_slot: u64,

    /// Cumulative borrow rate
    pub cumulative_borrow_rate: Decimal,
    /// Total borrows, plus interest
    pub total_borrows: Decimal,
    /// Last slot when borrow state was updated
    pub borrow_state_update_slot: u64,
}

impl Reserve {
    /// Fetch the current market price
    pub fn current_dex_market_price(&self, clock: &Clock) -> Result<u64, ProgramError> {
        if self.dex_market.is_none() {
            Ok(1) // TODO: need decimals?
        } else if self.dex_market_price_updated_slot == 0 {
            Err(LendingError::ReservePriceUnset.into())
        } else if self.dex_market_price_updated_slot + PRICE_EXPIRATION_SLOTS <= clock.slot {
            Err(LendingError::ReservePriceExpired.into())
        } else {
            Ok(self.dex_market_price)
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

    /// Calculate the current borrow rate
    pub fn current_borrow_rate(&self, liquidity_supply: &TokenAccount) -> Decimal {
        let total_liquidity = Decimal::from(liquidity_supply.amount);
        let total_supply = self.total_borrows + total_liquidity;

        // let zero = Decimal::from(0);
        // if total_supply == zero {
        //     return zero;
        // }

        // Optimize for this utilization rate for stable coins
        //  increase borrow rate multiplier when utilization is higher
        let optimal_utilization_rate = Decimal::new(80, 2);
        let optimal_borrow_rate = Decimal::new(4, 2);
        let base_borrow_rate = Decimal::new(0, 2);
        let max_borrow_rate = Decimal::new(30, 2);

        let utilization_rate: Decimal = self.total_borrows / total_supply;
        if utilization_rate < optimal_utilization_rate {
            // 50% should be normalized to 5/8 of the way to the optimal borrow rate
            let normalized_rate = utilization_rate / optimal_utilization_rate;
            // Borrow rate will then be 5/8 * optimal borrow rate
            normalized_rate * (optimal_borrow_rate - base_borrow_rate) + base_borrow_rate
        } else {
            let normalized_rate = (utilization_rate - optimal_utilization_rate)
                / (Decimal::from(1) - optimal_utilization_rate);
            normalized_rate * (max_borrow_rate - optimal_borrow_rate) + optimal_borrow_rate
        }
    }

    /// Update the cumulative borrow rate for the reserve
    pub fn update_cumulative_rate(
        &mut self,
        clock: &Clock,
        liquidity_supply: &TokenAccount,
    ) -> Decimal {
        if self.borrow_state_update_slot == 0 {
            self.borrow_state_update_slot = clock.slot;
            self.cumulative_borrow_rate = Decimal::from(1u64);
        } else {
            let borrow_rate = self.current_borrow_rate(liquidity_supply);
            let slots_elapsed = Decimal::from(clock.slot - self.borrow_state_update_slot);
            let interest_rate: Decimal =
                slots_elapsed * borrow_rate / Decimal::from(SLOTS_PER_YEAR);
            let accrued_interest: Decimal = self.total_borrows * interest_rate;

            self.total_borrows += accrued_interest;
            self.cumulative_borrow_rate *= Decimal::from(1) + interest_rate;
            self.borrow_state_update_slot = clock.slot;
        }

        self.cumulative_borrow_rate
    }

    /// Convert reserve collateral to liquidity
    pub fn collateral_to_liquidity(
        &self,
        clock: &Clock,
        liquidity_supply: &TokenAccount,
        collateral_mint: &Mint,
        collateral_amount: u64,
    ) -> Result<Decimal, ProgramError> {
        let exchange_rate =
            self.collateral_exchange_rate(clock, liquidity_supply, collateral_mint)?;
        Ok(Decimal::from(collateral_amount) / exchange_rate)
    }

    /// Convert reserve liquidity to collateral
    pub fn liquidity_to_collateral(
        &self,
        clock: &Clock,
        liquidity_supply: &TokenAccount,
        collateral_mint: &Mint,
        liquidity_amount: u64,
    ) -> Result<Decimal, ProgramError> {
        let exchange_rate =
            self.collateral_exchange_rate(clock, liquidity_supply, collateral_mint)?;
        Ok(Decimal::from(liquidity_amount) * exchange_rate)
    }

    /// Return the current collateral exchange rate.
    fn collateral_exchange_rate(
        &self,
        clock: &Clock,
        liquidity_supply: &TokenAccount,
        collateral_mint: &Mint,
    ) -> Result<Decimal, ProgramError> {
        // TODO: is exchange rate fixed within a slot?
        if self.borrow_state_update_slot != clock.slot {
            info!("collateral exchange rate needs to be updated");
            Err(LendingError::InvalidInput.into())
        } else if collateral_mint.supply == 0 {
            Ok(Decimal::from(INITIAL_COLLATERAL_RATE))
        } else {
            let collateral_supply = Decimal::from(collateral_mint.supply);
            let liquidity_supply = Decimal::from(liquidity_supply.amount);
            let collateral_over_liquidity_rate: Decimal =
                collateral_supply / (self.total_borrows + liquidity_supply);
            Ok(collateral_over_liquidity_rate)
        }
    }
}

/// Borrow obligation state
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Obligation {
    /// Slot when obligation was updated. Used for calculating interest.
    pub last_update_slot: u64,
    /// Amount of collateral tokens deposited for this obligation
    pub collateral_amount: u64,
    /// Reserve which collateral tokens were deposited into
    pub collateral_supply: Pubkey,
    /// Borrow rate used for calculating interest.
    pub cumulative_borrow_rate: Decimal,
    /// Amount of tokens borrowed for this obligation plus interest
    pub borrow_amount: Decimal,
    /// Reserve which tokens were borrowed from
    pub borrow_reserve: Pubkey,
    /// Mint address of the tokens for this obligation
    pub token_mint: Pubkey,
}

impl Obligation {
    /// Accrue interest
    pub fn accrue_interest(
        &mut self,
        clock: &Clock,
        reserve: &Reserve,
    ) -> Result<(), ProgramError> {
        if clock.slot != reserve.borrow_state_update_slot {
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

impl Sealed for Reserve {}
impl IsInitialized for Reserve {
    fn is_initialized(&self) -> bool {
        self.is_initialized
    }
}

const RESERVE_LEN: usize = 253;
impl Pack for Reserve {
    const LEN: usize = 253;

    /// Unpacks a byte buffer into a [ReserveInfo](struct.ReserveInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, RESERVE_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            is_initialized,
            lending_market,
            liquidity_supply,
            liquidity_mint,
            collateral_supply,
            collateral_mint,
            dex_market,
            dex_market_price,
            dex_market_price_updated_slot,
            cumulative_borrow_rate,
            total_borrows,
            borrow_state_update_slot,
        ) = array_refs![input, 1, 32, 32, 32, 32, 32, 36, 8, 8, 16, 16, 8];
        Ok(Self {
            is_initialized: match is_initialized {
                [0] => false,
                [1] => true,
                _ => return Err(ProgramError::InvalidAccountData),
            },
            lending_market: Pubkey::new_from_array(*lending_market),
            liquidity_supply: Pubkey::new_from_array(*liquidity_supply),
            liquidity_mint: Pubkey::new_from_array(*liquidity_mint),
            collateral_supply: Pubkey::new_from_array(*collateral_supply),
            collateral_mint: Pubkey::new_from_array(*collateral_mint),
            dex_market: unpack_coption_key(dex_market)?,
            dex_market_price: u64::from_le_bytes(*dex_market_price),
            dex_market_price_updated_slot: u64::from_le_bytes(*dex_market_price_updated_slot),
            cumulative_borrow_rate: unpack_decimal(cumulative_borrow_rate),
            total_borrows: unpack_decimal(total_borrows),
            borrow_state_update_slot: u64::from_le_bytes(*borrow_state_update_slot),
        })
    }

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, RESERVE_LEN];
        let (
            is_initialized,
            lending_market,
            liquidity_supply,
            liquidity_mint,
            collateral_supply,
            collateral_mint,
            dex_market,
            dex_market_price,
            dex_market_price_updated_slot,
            cumulative_borrow_rate,
            total_borrows,
            borrow_state_update_slot,
        ) = mut_array_refs![output, 1, 32, 32, 32, 32, 32, 36, 8, 8, 16, 16, 8];
        is_initialized[0] = self.is_initialized as u8;
        lending_market.copy_from_slice(self.lending_market.as_ref());
        liquidity_supply.copy_from_slice(self.liquidity_supply.as_ref());
        liquidity_mint.copy_from_slice(self.liquidity_mint.as_ref());
        collateral_supply.copy_from_slice(self.collateral_supply.as_ref());
        collateral_mint.copy_from_slice(self.collateral_mint.as_ref());
        pack_coption_key(&self.dex_market, dex_market);
        *dex_market_price = self.dex_market_price.to_le_bytes();
        *dex_market_price_updated_slot = self.dex_market_price_updated_slot.to_le_bytes();
        pack_decimal(self.cumulative_borrow_rate, cumulative_borrow_rate);
        pack_decimal(self.total_borrows, total_borrows);
        *borrow_state_update_slot = self.borrow_state_update_slot.to_le_bytes();
    }
}

impl Sealed for LendingMarket {}
impl IsInitialized for LendingMarket {
    fn is_initialized(&self) -> bool {
        self.is_initialized
    }
}

const LENDING_MARKET_LEN: usize = 33;
impl Pack for LendingMarket {
    const LEN: usize = 33;

    /// Unpacks a byte buffer into a [LendingMarketInfo](struct.LendingMarketInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, LENDING_MARKET_LEN];
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
        let output = array_mut_ref![output, 0, LENDING_MARKET_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (is_initialized, quote_token_mint) = mut_array_refs![output, 1, 32];
        *is_initialized = [self.is_initialized as u8];
        quote_token_mint.copy_from_slice(self.quote_token_mint.as_ref());
    }
}

impl Sealed for Obligation {}
impl IsInitialized for Obligation {
    fn is_initialized(&self) -> bool {
        self.last_update_slot > 0
    }
}

const OBLIGATION_LEN: usize = 144;
impl Pack for Obligation {
    const LEN: usize = 144;

    /// Unpacks a byte buffer into a [ObligationInfo](struct.ObligationInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, OBLIGATION_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            last_update_slot,
            collateral_amount,
            collateral_supply,
            cumulative_borrow_rate,
            borrow_amount,
            borrow_reserve,
            token_mint,
        ) = array_refs![input, 8, 8, 32, 16, 16, 32, 32];
        Ok(Self {
            last_update_slot: u64::from_le_bytes(*last_update_slot),
            collateral_amount: u64::from_le_bytes(*collateral_amount),
            collateral_supply: Pubkey::new_from_array(*collateral_supply),
            cumulative_borrow_rate: unpack_decimal(cumulative_borrow_rate),
            borrow_amount: unpack_decimal(borrow_amount),
            borrow_reserve: Pubkey::new_from_array(*borrow_reserve),
            token_mint: Pubkey::new_from_array(*token_mint),
        })
    }

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, OBLIGATION_LEN];
        let (
            last_update_slot,
            collateral_amount,
            collateral_supply,
            cumulative_borrow_rate,
            borrow_amount,
            borrow_reserve,
            token_mint,
        ) = mut_array_refs![output, 8, 8, 32, 16, 16, 32, 32];

        *last_update_slot = self.last_update_slot.to_le_bytes();
        *collateral_amount = self.collateral_amount.to_le_bytes();
        collateral_supply.copy_from_slice(self.collateral_supply.as_ref());
        pack_decimal(self.cumulative_borrow_rate, cumulative_borrow_rate);
        pack_decimal(self.borrow_amount, borrow_amount);
        borrow_reserve.copy_from_slice(self.borrow_reserve.as_ref());
        token_mint.copy_from_slice(self.token_mint.as_ref());
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

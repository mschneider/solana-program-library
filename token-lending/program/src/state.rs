//! State types

use crate::error::LendingError;
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use solana_program::{
    program_error::ProgramError,
    program_option::COption,
    program_pack::{IsInitialized, Pack, Sealed},
    pubkey::Pubkey,
    sysvar::clock::Clock,
};

/// Prices are only valid for a few slots before needing to be updated again
const PRICE_EXPIRATION_SLOTS: u64 = 5;

/// Maximum number of pool reserves
pub const MAX_RESERVES: u8 = 10;
const MAX_RESERVES_USIZE: usize = MAX_RESERVES as usize;

/// Lending pool state
#[repr(C)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PoolInfo {
    /// Initialized state.
    pub is_initialized: bool,
    /// Quote currency token mint.
    pub quote_token_mint: Pubkey,
    /// Number of active reserves.
    pub num_reserves: u8,
    /// List of reserves that belong to this pool.
    pub reserves: Box<[Pubkey; MAX_RESERVES_USIZE]>,
}

/// Pool reserve state
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReserveInfo {
    /// Initialized state.
    pub is_initialized: bool,
    /// Pool address
    pub pool: Pubkey,
    /// Reserve token pool
    pub reserve: Pubkey,
    /// Collateral token pool (liquidity tokens tracked for interest calculation)
    pub collateral: Pubkey,
    /// Liquidity tokens are minted when reserve tokens are deposited.
    /// Liquidity tokens can be withdrawn back to the original reserve token.
    pub liquidity_token_mint: Pubkey,
    /// DEX market state account
    pub dex_market: COption<Pubkey>,
    /// Latest market price
    pub market_price: u64,
    /// DEX market state account
    pub market_price_updated_slot: u64,
    /// Borrow rate (over fixed denominator)
    pub borrow_rate: u32,
}

impl ReserveInfo {
    /// Fetch the current market price
    pub fn current_market_price(&self, clock: &Clock) -> Result<u64, ProgramError> {
        if self.dex_market.is_none() {
            Ok(1) // TODO: need decimals
        } else if self.market_price_updated_slot == 0 {
            Err(LendingError::ReservePriceUnset.into())
        } else if self.market_price_updated_slot + PRICE_EXPIRATION_SLOTS <= clock.slot {
            Err(LendingError::ReservePriceExpired.into())
        } else {
            Ok(self.market_price)
        }
    }
}

/// Borrow obligation state
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ObligationInfo {
    /// Slot when obligation was updated. Used for calculating interest.
    pub updated_at_slot: u64,
    /// Address that has the authority to repay this obligation
    pub authority: Pubkey,
    /// Amount of collateral tokens deposited for this obligation
    pub collateral_amount: u64,
    /// Reserve which collateral tokens were deposited into
    pub collateral_reserve: Pubkey,
    /// Amount of tokens borrowed for this obligation
    pub borrow_amount: u64,
    /// Reserve which tokens were borrowed from
    pub borrow_reserve: Pubkey,
}

impl Sealed for ReserveInfo {}
impl IsInitialized for ReserveInfo {
    fn is_initialized(&self) -> bool {
        self.is_initialized
    }
}

const RESERVE_LEN: usize = 185;
impl Pack for ReserveInfo {
    const LEN: usize = 185;

    /// Unpacks a byte buffer into a [ReserveInfo](struct.ReserveInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, RESERVE_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            is_initialized,
            pool,
            reserve,
            collateral,
            liquidity_token_mint,
            dex_market,
            market_price,
            market_price_updated_slot,
            borrow_rate,
        ) = array_refs![input, 1, 32, 32, 32, 32, 36, 8, 8, 4];
        Ok(Self {
            is_initialized: match is_initialized {
                [0] => false,
                [1] => true,
                _ => return Err(ProgramError::InvalidAccountData),
            },
            pool: Pubkey::new_from_array(*pool),
            reserve: Pubkey::new_from_array(*reserve),
            collateral: Pubkey::new_from_array(*collateral),
            liquidity_token_mint: Pubkey::new_from_array(*liquidity_token_mint),
            dex_market: unpack_coption_key(dex_market)?,
            market_price: u64::from_le_bytes(*market_price),
            market_price_updated_slot: u64::from_le_bytes(*market_price_updated_slot),
            borrow_rate: u32::from_le_bytes(*borrow_rate),
        })
    }

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, RESERVE_LEN];
        let (
            is_initialized,
            pool,
            reserve,
            collateral,
            pool_mint,
            dex_market,
            market_price,
            market_price_updated_slot,
            borrow_rate,
        ) = mut_array_refs![output, 1, 32, 32, 32, 32, 36, 8, 8, 4];
        is_initialized[0] = self.is_initialized as u8;
        pool.copy_from_slice(self.pool.as_ref());
        reserve.copy_from_slice(self.reserve.as_ref());
        collateral.copy_from_slice(self.collateral.as_ref());
        pool_mint.copy_from_slice(self.liquidity_token_mint.as_ref());
        pack_coption_key(&self.dex_market, dex_market);
        *market_price = self.market_price.to_le_bytes();
        *market_price_updated_slot = self.market_price_updated_slot.to_le_bytes();
        *borrow_rate = self.borrow_rate.to_le_bytes();
    }
}

impl Sealed for PoolInfo {}
impl IsInitialized for PoolInfo {
    fn is_initialized(&self) -> bool {
        self.is_initialized
    }
}

const POOL_LEN: usize = 354;
impl Pack for PoolInfo {
    const LEN: usize = 354;

    /// Unpacks a byte buffer into a [PoolInfo](struct.PoolInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, POOL_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (is_initialized, quote_token_mint, num_reserves, reserves_flat) =
            array_refs![input, 1, 32, 1, 32 * MAX_RESERVES_USIZE];
        let mut pool = Self {
            is_initialized: match is_initialized {
                [0] => false,
                [1] => true,
                _ => return Err(ProgramError::InvalidAccountData),
            },
            quote_token_mint: Pubkey::new_from_array(*quote_token_mint),
            num_reserves: num_reserves[0],
            reserves: Box::new([Pubkey::new_from_array([0u8; 32]); MAX_RESERVES_USIZE]),
        };
        for (src, dst) in reserves_flat
            .chunks(32)
            .zip(pool.reserves.iter_mut())
            .take(pool.num_reserves as usize)
        {
            *dst = Pubkey::new(src);
        }
        Ok(pool)
    }

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, POOL_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (is_initialized, quote_token_mint, num_reserves, reserves_flat) =
            mut_array_refs![output, 1, 32, 1, 32 * MAX_RESERVES_USIZE];
        *is_initialized = [self.is_initialized as u8];
        quote_token_mint.copy_from_slice(self.quote_token_mint.as_ref());
        *num_reserves = [self.num_reserves];
        for (i, src) in self.reserves.iter().enumerate() {
            let dst_array = array_mut_ref![reserves_flat, 32 * i, 32];
            dst_array.copy_from_slice(src.as_ref());
        }
    }
}

impl Sealed for ObligationInfo {}
impl IsInitialized for ObligationInfo {
    fn is_initialized(&self) -> bool {
        self.updated_at_slot > 0
    }
}

const OBLIGATION_LEN: usize = 120;
impl Pack for ObligationInfo {
    const LEN: usize = 120;

    /// Unpacks a byte buffer into a [ObligationInfo](struct.ObligationInfo.html).
    fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
        let input = array_ref![input, 0, OBLIGATION_LEN];
        #[allow(clippy::ptr_offset_with_cast)]
        let (
            updated_at_slot,
            authority,
            collateral_amount,
            collateral_reserve,
            borrow_amount,
            borrow_reserve,
        ) = array_refs![input, 8, 32, 8, 32, 8, 32];
        Ok(Self {
            updated_at_slot: u64::from_le_bytes(*updated_at_slot),
            authority: Pubkey::new_from_array(*authority),
            collateral_amount: u64::from_le_bytes(*collateral_amount),
            collateral_reserve: Pubkey::new_from_array(*collateral_reserve),
            borrow_amount: u64::from_le_bytes(*borrow_amount),
            borrow_reserve: Pubkey::new_from_array(*borrow_reserve),
        })
    }

    fn pack_into_slice(&self, output: &mut [u8]) {
        let output = array_mut_ref![output, 0, OBLIGATION_LEN];
        let (
            updated_at_slot,
            authority,
            collateral_amount,
            collateral_reserve,
            borrow_amount,
            borrow_reserve,
        ) = mut_array_refs![output, 8, 32, 8, 32, 8, 32];

        *updated_at_slot = self.updated_at_slot.to_le_bytes();
        authority.copy_from_slice(self.authority.as_ref());
        *collateral_amount = self.collateral_amount.to_le_bytes();
        collateral_reserve.copy_from_slice(self.collateral_reserve.as_ref());
        *borrow_amount = self.borrow_amount.to_le_bytes();
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

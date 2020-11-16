//! Program state processor

use crate::{
    error::LendingError,
    instruction::LendingInstruction,
    math::Decimal,
    state::{ObligationInfo, PoolInfo, ReserveInfo},
};
use arrayref::{array_refs, mut_array_refs};
use num_traits::FromPrimitive;
use serum_dex::critbit::{Slab, SlabView};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    decode_error::DecodeError,
    entrypoint::ProgramResult,
    info,
    program::{invoke, invoke_signed},
    program_error::{PrintProgramError, ProgramError},
    program_option::COption,
    program_pack::{IsInitialized, Pack},
    pubkey::Pubkey,
    sysvar::{clock::Clock, rent::Rent, Sysvar},
};
use spl_token::state::{Account as Token, Mint};
use std::cell::RefMut;

/// Processes an instruction
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    input: &[u8],
) -> ProgramResult {
    let instruction = LendingInstruction::unpack(input)?;
    match instruction {
        LendingInstruction::InitPool => {
            info!("Instruction: Init Pool");
            process_init_pool(program_id, accounts)
        }
        LendingInstruction::InitReserve => {
            info!("Instruction: Init Reserve");
            process_init_reserve(program_id, accounts)
        }
        LendingInstruction::Deposit { amount } => {
            info!("Instruction: Deposit");
            process_deposit(program_id, amount, accounts)
        }
        LendingInstruction::Withdraw { amount } => {
            info!("Instruction: Withdraw");
            process_withdraw(program_id, amount, accounts)
        }
        LendingInstruction::Borrow { collateral_amount } => {
            info!("Instruction: Borrow");
            process_borrow(program_id, collateral_amount, accounts)
        }
        LendingInstruction::Repay { liquidity_amount } => {
            info!("Instruction: Repay");
            process_repay(program_id, liquidity_amount, accounts)
        }
        LendingInstruction::SetPrice => {
            info!("Instruction: Set price");
            process_set_price(accounts)
        }
    }
}

fn process_init_pool(_program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let pool_info = next_account_info(account_info_iter)?;
    let quote_token_mint_info = next_account_info(account_info_iter)?;
    let rent = &Rent::from_account_info(next_account_info(account_info_iter)?)?;
    let token_program_id = next_account_info(account_info_iter)?;

    let mut pool = PoolInfo::unpack_unchecked(&pool_info.data.borrow())?;
    if pool.is_initialized() {
        return Err(LendingError::AlreadyInUse.into());
    }
    if !rent.is_exempt(pool_info.lamports(), pool_info.data_len()) {
        info!(&rent.minimum_balance(pool_info.data_len()).to_string());
        return Err(LendingError::NotRentExempt.into());
    }

    if quote_token_mint_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }
    unpack_mint(&quote_token_mint_info.data.borrow())?;

    pool.is_initialized = true;
    pool.quote_token_mint = *quote_token_mint_info.key;
    PoolInfo::pack(pool, &mut pool_info.data.borrow_mut())?;

    Ok(())
}

fn process_init_reserve(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let reserve_info = next_account_info(account_info_iter)?;
    let pool_info = next_account_info(account_info_iter)?;
    let pool_authority_info = next_account_info(account_info_iter)?;
    let liquidity_reserve_info = next_account_info(account_info_iter)?;
    let collateral_mint_info = next_account_info(account_info_iter)?;
    let collateral_reserve_info = next_account_info(account_info_iter)?;
    let collateral_output_info = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let rent_info = next_account_info(account_info_iter)?;
    let rent = &Rent::from_account_info(rent_info)?;
    let token_program_id = next_account_info(account_info_iter)?;

    let mut reserve = ReserveInfo::unpack_unchecked(&reserve_info.data.borrow())?;
    if reserve.is_initialized() {
        return Err(LendingError::AlreadyInUse.into());
    }
    if !rent.is_exempt(reserve_info.lamports(), reserve_info.data_len()) {
        info!(&rent.minimum_balance(reserve_info.data_len()).to_string());
        return Err(LendingError::NotRentExempt.into());
    }

    let pool = PoolInfo::unpack(&pool_info.data.borrow())?;
    let bump_seed = find_authority_bump_seed(program_id, &pool_info.key);
    if pool_authority_info.key != &authority_id(program_id, pool_info.key, bump_seed)? {
        return Err(LendingError::InvalidInput.into());
    }
    if !pool_info.is_signer {
        return Err(LendingError::InvalidInput.into());
    }

    if liquidity_reserve_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }
    if collateral_reserve_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }
    if collateral_mint_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }

    let liquidity_reserve = unpack_token_account(&liquidity_reserve_info.data.borrow())?;
    if pool_authority_info.key != &liquidity_reserve.owner {
        return Err(LendingError::InvalidOwner.into());
    }

    let collateral_output = Token::unpack_unchecked(&collateral_output_info.data.borrow())?;
    let collateral_reserve = Token::unpack_unchecked(&collateral_reserve_info.data.borrow())?;
    let collateral_mint = Mint::unpack_unchecked(&collateral_mint_info.data.borrow())?;
    if collateral_mint.is_initialized()
        || collateral_reserve.is_initialized()
        || collateral_output.is_initialized()
    {
        return Err(LendingError::AlreadyInUse.into());
    }

    spl_token_init_mint(TokenInitializeMintParams {
        mint: collateral_mint_info.clone(),
        authority: pool_authority_info.key,
        rent: rent_info.clone(),
        token_program: token_program_id.clone(),
    })?;

    spl_token_init_account(TokenInitializeAccountParams {
        account: collateral_reserve_info.clone(),
        mint: collateral_mint_info.clone(),
        owner: pool_authority_info.clone(),
        rent: rent_info.clone(),
        token_program: token_program_id.clone(),
    })?;

    spl_token_init_account(TokenInitializeAccountParams {
        account: collateral_output_info.clone(),
        mint: collateral_mint_info.clone(),
        owner: pool_authority_info.clone(),
        rent: rent_info.clone(),
        token_program: token_program_id.clone(),
    })?;

    spl_token_mint_to(TokenMintToParams {
        mint: collateral_mint_info.clone(),
        destination: collateral_output_info.clone(),
        amount: liquidity_reserve.amount, // TODO exchange rate
        authority: pool_authority_info.clone(),
        authorized: pool_info.key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    if liquidity_reserve.close_authority.is_some() {
        return Err(LendingError::InvalidCloseAuthority.into());
    }
    if liquidity_reserve.delegate.is_some() {
        return Err(LendingError::InvalidDelegate.into());
    }
    if liquidity_reserve.close_authority.is_some() {
        return Err(LendingError::InvalidCloseAuthority.into());
    }
    if liquidity_reserve.delegate.is_some() {
        return Err(LendingError::InvalidDelegate.into());
    }
    if liquidity_reserve.amount == 0 {
        return Err(LendingError::InvalidInput.into());
    }

    let dex_market = if liquidity_reserve.mint != pool.quote_token_mint {
        let dex_market_info = next_account_info(account_info_iter)?;
        if !rent.is_exempt(dex_market_info.lamports(), dex_market_info.data_len()) {
            return Err(LendingError::NotRentExempt.into());
        }

        // TODO: check that market state is owned by real serum dex program
        fn base_mint_pubkey(data: &[u8]) -> Pubkey {
            let count_start = 5 + 6 * 8;
            let count_end = count_start + 32;
            Pubkey::new(&data[count_start..count_end])
        }

        fn quote_mint_pubkey(data: &[u8]) -> Pubkey {
            let count_start = 5 + 10 * 8;
            let count_end = count_start + 32;
            Pubkey::new(&data[count_start..count_end])
        }

        let market_base_mint = base_mint_pubkey(&dex_market_info.data.borrow());
        let market_quote_mint = quote_mint_pubkey(&dex_market_info.data.borrow());
        if pool.quote_token_mint != market_quote_mint {
            info!(&market_quote_mint.to_string().as_str());
            return Err(LendingError::InvalidInput.into());
        }
        if liquidity_reserve.mint != market_base_mint {
            info!(&market_base_mint.to_string().as_str());
            return Err(LendingError::InvalidInput.into());
        }

        COption::Some(*dex_market_info.key)
    } else {
        COption::None
    };

    reserve.is_initialized = true;
    reserve.pool = *pool_info.key;
    reserve.liquidity_reserve = *liquidity_reserve_info.key;
    reserve.collateral_reserve = *collateral_reserve_info.key;
    reserve.collateral_mint = *collateral_mint_info.key;
    reserve.dex_market = dex_market;
    reserve.update_cumulative_rate(clock, &liquidity_reserve);
    ReserveInfo::pack(reserve, &mut reserve_info.data.borrow_mut())?;

    Ok(())
}

fn process_deposit(program_id: &Pubkey, amount: u64, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let reserve_info = next_account_info(account_info_iter)?;
    let pool_authority_info = next_account_info(account_info_iter)?;
    let liquidity_input_info = next_account_info(account_info_iter)?;
    let liquidity_reserve_info = next_account_info(account_info_iter)?;
    let collateral_output_info = next_account_info(account_info_iter)?;
    let collateral_mint_info = next_account_info(account_info_iter)?;
    let token_program_id = next_account_info(account_info_iter)?;

    let reserve = ReserveInfo::unpack(&reserve_info.data.borrow())?;
    let bump_seed = find_authority_bump_seed(program_id, &reserve.pool);
    if pool_authority_info.key != &authority_id(program_id, &reserve.pool, bump_seed)? {
        return Err(LendingError::InvalidProgramAddress.into());
    }

    if liquidity_reserve_info.key != &reserve.liquidity_reserve
        || collateral_mint_info.key != &reserve.collateral_mint
    {
        return Err(LendingError::InvalidInput.into());
    }
    if liquidity_reserve_info.key == liquidity_input_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if collateral_output_info.key == &reserve.collateral_reserve {
        return Err(LendingError::InvalidInput.into());
    }

    spl_token_transfer(TokenTransferParams {
        source: liquidity_input_info.clone(),
        destination: liquidity_reserve_info.clone(),
        amount,
        authority: pool_authority_info.clone(),
        authorized: &reserve.pool,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    // TODO: update reserve
    // TODO: calculate exchange rate

    spl_token_mint_to(TokenMintToParams {
        mint: collateral_mint_info.clone(),
        destination: collateral_output_info.clone(),
        amount,
        authority: pool_authority_info.clone(),
        authorized: &reserve.pool,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    Ok(())
}

fn process_withdraw(
    program_id: &Pubkey,
    collateral_amount: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let reserve_info = next_account_info(account_info_iter)?;
    let pool_authority_info = next_account_info(account_info_iter)?;
    let liquidity_reserve_info = next_account_info(account_info_iter)?;
    let liquidity_output_info = next_account_info(account_info_iter)?;
    let collateral_input_info = next_account_info(account_info_iter)?;
    let collateral_mint_info = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let token_program_id = next_account_info(account_info_iter)?;

    let mut reserve = ReserveInfo::unpack(&reserve_info.data.borrow())?;
    let bump_seed = find_authority_bump_seed(program_id, &reserve.pool);
    if pool_authority_info.key != &authority_id(program_id, &reserve.pool, bump_seed)? {
        return Err(LendingError::InvalidProgramAddress.into());
    }

    if liquidity_reserve_info.key != &reserve.liquidity_reserve
        || collateral_mint_info.key != &reserve.collateral_mint
    {
        return Err(LendingError::InvalidInput.into());
    }
    if liquidity_reserve_info.key == liquidity_output_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if collateral_input_info.key == &reserve.collateral_reserve {
        return Err(LendingError::InvalidInput.into());
    }

    let reserve_token = &unpack_token_account(&liquidity_reserve_info.data.borrow())?;
    let liquidity_mint = &unpack_mint(&collateral_mint_info.data.borrow())?;

    reserve.update_cumulative_rate(clock, reserve_token);
    let collateral_exchange_rate =
        reserve.collateral_exchange_rate(clock, reserve_token, liquidity_mint)?;
    let withdraw_amount: Decimal = collateral_exchange_rate * Decimal::from(collateral_amount);

    spl_token_transfer(TokenTransferParams {
        source: liquidity_reserve_info.clone(),
        destination: liquidity_output_info.clone(),
        amount: withdraw_amount.round_u64(),
        authority: pool_authority_info.clone(),
        authorized: &reserve.pool,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    spl_token_burn(TokenBurnParams {
        mint: collateral_mint_info.clone(),
        source: collateral_input_info.clone(),
        amount: collateral_amount,
        authority: pool_authority_info.clone(),
        authorized: &reserve.pool,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    ReserveInfo::pack(reserve, &mut reserve_info.data.borrow_mut())?;

    Ok(())
}

#[inline(never)] // avoid stack frame limit
fn process_borrow(
    program_id: &Pubkey,
    collateral_amount: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let deposit_reserve_info = next_account_info(account_info_iter)?;
    let borrow_reserve_info = next_account_info(account_info_iter)?;
    let pool_authority_info = next_account_info(account_info_iter)?;
    let liquidity_reserve_info = next_account_info(account_info_iter)?;
    let liquidity_output_info = next_account_info(account_info_iter)?;
    let collateral_input_info = next_account_info(account_info_iter)?;
    let collateral_reserve_info = next_account_info(account_info_iter)?;
    let obligation_info = next_account_info(account_info_iter)?;
    let obligation_token_mint_info = next_account_info(account_info_iter)?;
    let obligation_token_output_info = next_account_info(account_info_iter)?;
    let obligation_token_owner_info = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let rent_info = next_account_info(account_info_iter)?;
    let rent = &Rent::from_account_info(rent_info)?;
    let token_program_id = next_account_info(account_info_iter)?;

    let obligation = ObligationInfo::unpack_unchecked(&obligation_info.data.borrow())?;
    if obligation.is_initialized() {
        return Err(LendingError::AlreadyInUse.into());
    }
    if !rent.is_exempt(obligation_info.lamports(), obligation_info.data_len()) {
        info!(&rent.minimum_balance(obligation_info.data_len()).to_string());
        return Err(LendingError::NotRentExempt.into());
    }

    let deposit_reserve = ReserveInfo::unpack(&deposit_reserve_info.data.borrow())?;
    let mut borrow_reserve = ReserveInfo::unpack(&borrow_reserve_info.data.borrow())?;
    let collateral_reserve = unpack_token_account(&collateral_reserve_info.data.borrow())?;

    if deposit_reserve.pool != borrow_reserve.pool {
        return Err(LendingError::PoolMismatch.into());
    }
    if &borrow_reserve.liquidity_reserve != liquidity_reserve_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if deposit_reserve.collateral_mint != collateral_reserve.mint {
        return Err(LendingError::InvalidInput.into());
    }

    let pool_key = deposit_reserve.pool;
    let bump_seed = find_authority_bump_seed(program_id, &pool_key);

    spl_token_transfer(TokenTransferParams {
        source: collateral_input_info.clone(),
        destination: collateral_reserve_info.clone(),
        amount: collateral_amount,
        authority: pool_authority_info.clone(),
        authorized: &pool_key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    let deposit_token_price = Decimal::from(deposit_reserve.current_market_price(clock)?);
    let borrow_token_price = Decimal::from(borrow_reserve.current_market_price(clock)?);
    let collateral_value: Decimal = Decimal::from(collateral_amount) * deposit_token_price;
    let borrow_amount: Decimal = collateral_value / borrow_token_price;

    spl_token_transfer(TokenTransferParams {
        source: liquidity_reserve_info.clone(),
        destination: liquidity_output_info.clone(),
        amount: borrow_amount.round_u64(),
        authority: pool_authority_info.clone(),
        authorized: &pool_key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    let borrow_reserve_token = &unpack_token_account(&liquidity_reserve_info.data.borrow())?;
    let cumulative_borrow_rate = borrow_reserve.update_cumulative_rate(clock, borrow_reserve_token);
    borrow_reserve.add_borrow(borrow_amount);
    ReserveInfo::pack(borrow_reserve, &mut borrow_reserve_info.data.borrow_mut())?;

    if obligation_token_mint_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }
    if obligation_token_output_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }

    let debt_mint = Mint::unpack_unchecked(&obligation_token_mint_info.data.borrow())?;
    let debt_output = Token::unpack_unchecked(&obligation_token_output_info.data.borrow())?;
    if debt_mint.is_initialized() || debt_output.is_initialized() {
        return Err(LendingError::AlreadyInUse.into());
    }

    let pool_authority = &authority_id(program_id, &deposit_reserve.pool, bump_seed)?;
    spl_token_init_mint(TokenInitializeMintParams {
        mint: obligation_token_mint_info.clone(),
        authority: pool_authority,
        rent: rent_info.clone(),
        token_program: token_program_id.clone(),
    })?;

    spl_token_init_account(TokenInitializeAccountParams {
        account: obligation_token_output_info.clone(),
        mint: obligation_token_mint_info.clone(),
        owner: obligation_token_owner_info.clone(),
        rent: rent_info.clone(),
        token_program: token_program_id.clone(),
    })?;

    spl_token_mint_to(TokenMintToParams {
        mint: obligation_token_mint_info.clone(),
        destination: obligation_token_output_info.clone(),
        amount: collateral_amount,
        authority: pool_authority_info.clone(),
        authorized: &deposit_reserve.pool,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    ObligationInfo::pack(
        ObligationInfo {
            last_update_slot: clock.slot,
            collateral_amount,
            collateral_reserve: *deposit_reserve_info.key,
            cumulative_borrow_rate,
            borrow_amount,
            borrow_reserve: *borrow_reserve_info.key,
            token_mint: *obligation_token_mint_info.key,
        },
        &mut obligation_info.data.borrow_mut(),
    )?;

    Ok(())
}

#[inline(never)] // avoid stack frame limit
fn process_repay(
    program_id: &Pubkey,
    liquidity_amount: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let repay_reserve_info = next_account_info(account_info_iter)?;
    let withdraw_reserve_info = next_account_info(account_info_iter)?;
    let pool_authority_info = next_account_info(account_info_iter)?;
    let liquidity_input_info = next_account_info(account_info_iter)?;
    let liquidity_reserve_info = next_account_info(account_info_iter)?;
    let collateral_reserve_info = next_account_info(account_info_iter)?;
    let collateral_output_info = next_account_info(account_info_iter)?;
    let obligation_info = next_account_info(account_info_iter)?;
    let obligation_mint_info = next_account_info(account_info_iter)?;
    let obligation_input_info = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let token_program_id = next_account_info(account_info_iter)?;

    let mut obligation = ObligationInfo::unpack(&obligation_info.data.borrow())?;
    if &obligation.token_mint != obligation_mint_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if &obligation.borrow_reserve != repay_reserve_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if &obligation.collateral_reserve != withdraw_reserve_info.key {
        return Err(LendingError::InvalidInput.into());
    }

    let mut repay_reserve = ReserveInfo::unpack(&repay_reserve_info.data.borrow())?;
    let withdraw_reserve = ReserveInfo::unpack(&withdraw_reserve_info.data.borrow())?;
    if repay_reserve.pool != withdraw_reserve.pool {
        return Err(LendingError::PoolMismatch.into());
    }
    if &repay_reserve.liquidity_reserve != liquidity_reserve_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if &withdraw_reserve.collateral_reserve != collateral_reserve_info.key {
        return Err(LendingError::InvalidInput.into());
    }

    let pool_key = repay_reserve.pool;
    let bump_seed = find_authority_bump_seed(program_id, &pool_key);

    let liquidity_reserve = &unpack_token_account(&liquidity_reserve_info.data.borrow())?;
    repay_reserve.update_cumulative_rate(clock, liquidity_reserve);
    obligation.accrue_interest(clock, &repay_reserve)?;

    let borrowed_amount = obligation.borrow_amount.round_u64();
    let repay_amount = liquidity_amount.min(borrowed_amount);
    let repay_pct: Decimal = Decimal::from(repay_amount) / obligation.borrow_amount;

    let collateral_withdraw_amount = {
        let withdraw_amount: Decimal = repay_pct * Decimal::from(obligation.collateral_amount);
        withdraw_amount.round_u64()
    };

    let obligation_token_amount = {
        let obligation_mint = &unpack_mint(&obligation_mint_info.data.borrow())?;
        let token_amount: Decimal = repay_pct * Decimal::from(obligation_mint.supply);
        token_amount.round_u64()
    };

    spl_token_transfer(TokenTransferParams {
        source: liquidity_input_info.clone(),
        destination: liquidity_reserve_info.clone(),
        amount: repay_amount,
        authority: pool_authority_info.clone(),
        authorized: &pool_key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    spl_token_transfer(TokenTransferParams {
        source: collateral_reserve_info.clone(),
        destination: collateral_output_info.clone(),
        amount: collateral_withdraw_amount,
        authority: pool_authority_info.clone(),
        authorized: &pool_key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    spl_token_burn(TokenBurnParams {
        mint: obligation_mint_info.clone(),
        source: obligation_input_info.clone(),
        authority: pool_authority_info.clone(),
        authorized: &pool_key,
        amount: obligation_token_amount,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    obligation.last_update_slot = clock.slot;
    obligation.borrow_amount -= Decimal::from(repay_amount);
    obligation.collateral_amount -= collateral_withdraw_amount;
    ObligationInfo::pack(obligation, &mut obligation_info.data.borrow_mut())?;

    repay_reserve.subtract_repay(Decimal::from(repay_amount));
    ReserveInfo::pack(repay_reserve, &mut repay_reserve_info.data.borrow_mut())?;

    Ok(())
}

fn process_set_price(accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let reserve_info = next_account_info(account_info_iter)?;
    let dex_market_info = next_account_info(account_info_iter)?;
    let dex_market_bids_info = next_account_info(account_info_iter)?;
    let dex_market_asks_info = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let memory = next_account_info(account_info_iter)?;

    let mut reserve = ReserveInfo::unpack(&reserve_info.data.borrow())?;
    if reserve.dex_market != COption::Some(*dex_market_info.key) {
        return Err(LendingError::InvalidInput.into());
    }

    fn load_bids_pubkey(data: &[u8]) -> Pubkey {
        let count_start = 5 + 35 * 8;
        let count_end = count_start + 32;
        Pubkey::new(&data[count_start..count_end])
    }

    fn load_asks_pubkey(data: &[u8]) -> Pubkey {
        let count_start = 5 + 39 * 8;
        let count_end = count_start + 32;
        Pubkey::new(&data[count_start..count_end])
    }

    let bids_pubkey = &load_bids_pubkey(&dex_market_info.data.borrow());
    let asks_pubkey = &load_asks_pubkey(&dex_market_info.data.borrow());

    if dex_market_bids_info.key != bids_pubkey {
        return Err(LendingError::InvalidInput.into());
    }
    if dex_market_asks_info.key != asks_pubkey {
        return Err(LendingError::InvalidInput.into());
    }

    enum Side {
        Bid,
        Ask,
    }

    fn find_best_order(
        orders: &AccountInfo,
        memory: &AccountInfo,
        side: Side,
    ) -> Result<u64, ProgramError> {
        let mut memory = memory.data.borrow_mut();
        {
            let bytes = orders.data.borrow();
            let start = 5 + 8;
            let end = bytes.len() - 7;
            fast_copy(&bytes[start..end], &mut memory);
        }

        let bytes = std::cell::RefCell::new(memory);
        let mut order_slab = RefMut::map(bytes.borrow_mut(), |bytes| Slab::new(bytes));

        let best_order = match side {
            Side::Bid => order_slab.find_max(),
            Side::Ask => order_slab.find_min(),
        }
        .ok_or_else(|| ProgramError::from(LendingError::InvalidInput))?;
        let best_order_ref = order_slab
            .get_mut(best_order)
            .unwrap()
            .as_leaf_mut()
            .unwrap();
        Ok(best_order_ref.price().into())
    }

    let best_bid = find_best_order(dex_market_bids_info, memory, Side::Bid)?;
    let best_ask = find_best_order(dex_market_asks_info, memory, Side::Ask)?;

    info!(&format!(
        "bid: {}, ask: {}, market: {}",
        best_bid,
        best_ask,
        (best_bid + best_ask) / 2
    ));
    fast_set(&mut memory.data.borrow_mut(), 0);

    reserve.market_price = (best_bid + best_ask) / 2;
    reserve.market_price_updated_slot = clock.slot;
    ReserveInfo::pack(reserve, &mut reserve_info.data.borrow_mut())?;

    Ok(())
}

/// Generates seed bump for lending pool authorities
fn find_authority_bump_seed(program_id: &Pubkey, my_info: &Pubkey) -> u8 {
    let (pubkey, bump_seed) =
        Pubkey::find_program_address(&[&my_info.to_bytes()[..32]], program_id);
    {
        let mut log_message: String = "Found authority ".to_string();
        log_message.push_str(&pubkey.to_string());
        log_message.push_str(" with bump seed ");
        log_message.push_str(&bump_seed.to_string());
        log_message.push_str(" for ");
        log_message.push_str(&my_info.to_string());
        info!(log_message.as_str());
    }
    bump_seed
}

/// Unpacks a spl_token `Account`.
fn unpack_token_account(data: &[u8]) -> Result<spl_token::state::Account, LendingError> {
    spl_token::state::Account::unpack(data).map_err(|_| LendingError::ExpectedTokenAccount)
}

/// Unpacks a spl_token `Mint`.
fn unpack_mint(data: &[u8]) -> Result<spl_token::state::Mint, LendingError> {
    spl_token::state::Mint::unpack(data).map_err(|_| LendingError::ExpectedTokenMint)
}

/// Calculates the authority id by generating a program address.
fn authority_id(
    program_id: &Pubkey,
    my_info: &Pubkey,
    bump_seed: u8,
) -> Result<Pubkey, LendingError> {
    Pubkey::create_program_address(&[&my_info.to_bytes()[..32], &[bump_seed]], program_id)
        .or(Err(LendingError::InvalidProgramAddress))
}

/// Issue a spl_token `InitializeMint` instruction.
fn spl_token_init_mint(params: TokenInitializeMintParams<'_, '_>) -> ProgramResult {
    let TokenInitializeMintParams {
        mint,
        rent,
        authority,
        token_program,
    } = params;
    let ix =
        spl_token::instruction::initialize_mint(token_program.key, mint.key, authority, None, 0)?;
    let result = invoke(&ix, &[mint, rent, token_program]);
    result.map_err(|err| {
        err.print::<spl_token::error::TokenError>();
        LendingError::TokenInitializeMintFailed.into()
    })
}

/// Issue a spl_token `InitializeAccount` instruction.
fn spl_token_init_account(params: TokenInitializeAccountParams<'_>) -> ProgramResult {
    let TokenInitializeAccountParams {
        account,
        mint,
        owner,
        rent,
        token_program,
    } = params;
    let ix = spl_token::instruction::initialize_account(
        token_program.key,
        account.key,
        mint.key,
        owner.key,
    )?;
    let result = invoke(&ix, &[account, mint, owner, rent, token_program]);
    result.map_err(|err| {
        err.print::<spl_token::error::TokenError>();
        LendingError::TokenInitializeAccountFailed.into()
    })
}

/// Issue a spl_token `Transfer` instruction.
fn spl_token_transfer(params: TokenTransferParams<'_, '_>) -> ProgramResult {
    let authorized_bytes = params.authorized.to_bytes();
    let authority_signer_seeds = [&authorized_bytes[..32], &[params.bump_seed]];
    let TokenTransferParams {
        source,
        destination,
        authority,
        token_program,
        amount,
        ..
    } = params;
    let ix = spl_token::instruction::transfer(
        token_program.key,
        source.key,
        destination.key,
        authority.key,
        &[],
        amount,
    )?;
    let result = invoke_signed(
        &ix,
        &[source, destination, authority, token_program],
        &[&authority_signer_seeds],
    );
    result.map_err(|err| {
        err.print::<spl_token::error::TokenError>();
        LendingError::TokenTransferFailed.into()
    })
}

/// Issue a spl_token `MintTo` instruction.
fn spl_token_mint_to(params: TokenMintToParams<'_, '_>) -> ProgramResult {
    let authorized_bytes = params.authorized.to_bytes();
    let authority_signer_seeds = [&authorized_bytes[..32], &[params.bump_seed]];
    let TokenMintToParams {
        mint,
        destination,
        authority,
        token_program,
        amount,
        ..
    } = params;
    let ix = spl_token::instruction::mint_to(
        token_program.key,
        mint.key,
        destination.key,
        authority.key,
        &[],
        amount,
    )?;
    let result = invoke_signed(
        &ix,
        &[mint, destination, authority, token_program],
        &[&authority_signer_seeds],
    );
    result.map_err(|err| {
        err.print::<spl_token::error::TokenError>();
        LendingError::TokenMintToFailed.into()
    })
}

/// Issue a spl_token `Burn` instruction.
fn spl_token_burn(params: TokenBurnParams<'_, '_>) -> ProgramResult {
    let authorized_bytes = params.authorized.to_bytes();
    let authority_signer_seeds = [&authorized_bytes[..32], &[params.bump_seed]];
    let TokenBurnParams {
        mint,
        source,
        authority,
        token_program,
        amount,
        ..
    } = params;
    let ix = spl_token::instruction::burn(
        token_program.key,
        source.key,
        mint.key,
        authority.key,
        &[],
        amount,
    )?;
    let result = invoke_signed(
        &ix,
        &[source, mint, authority, token_program],
        &[&authority_signer_seeds],
    );
    result.map_err(|err| {
        err.print::<spl_token::error::TokenError>();
        LendingError::TokenBurnFailed.into()
    })
}

struct TokenInitializeMintParams<'a: 'b, 'b> {
    mint: AccountInfo<'a>,
    rent: AccountInfo<'a>,
    authority: &'b Pubkey,
    token_program: AccountInfo<'a>,
}

struct TokenInitializeAccountParams<'a> {
    account: AccountInfo<'a>,
    mint: AccountInfo<'a>,
    owner: AccountInfo<'a>,
    rent: AccountInfo<'a>,
    token_program: AccountInfo<'a>,
}

struct TokenTransferParams<'a: 'b, 'b> {
    source: AccountInfo<'a>,
    destination: AccountInfo<'a>,
    amount: u64,
    authority: AccountInfo<'a>,
    authorized: &'b Pubkey,
    bump_seed: u8,
    token_program: AccountInfo<'a>,
}

struct TokenMintToParams<'a: 'b, 'b> {
    mint: AccountInfo<'a>,
    destination: AccountInfo<'a>,
    amount: u64,
    authority: AccountInfo<'a>,
    authorized: &'b Pubkey,
    bump_seed: u8,
    token_program: AccountInfo<'a>,
}

struct TokenBurnParams<'a: 'b, 'b> {
    mint: AccountInfo<'a>,
    source: AccountInfo<'a>,
    amount: u64,
    authority: AccountInfo<'a>,
    authorized: &'b Pubkey,
    bump_seed: u8,
    token_program: AccountInfo<'a>,
}

impl PrintProgramError for LendingError {
    fn print<E>(&self)
    where
        E: 'static + std::error::Error + DecodeError<E> + PrintProgramError + FromPrimitive,
    {
        info!(&format!(
            "{}: {}",
            <Self as DecodeError<E>>::type_of(),
            self.to_string()
        ));
    }
}

/// A more efficient `copy_from_slice` implementation.
fn fast_copy(mut src: &[u8], mut dst: &mut [u8]) {
    const COPY_SIZE: usize = 512;
    while src.len() >= COPY_SIZE {
        #[allow(clippy::ptr_offset_with_cast)]
        let (src_word, src_rem) = array_refs![src, COPY_SIZE; ..;];
        #[allow(clippy::ptr_offset_with_cast)]
        let (dst_word, dst_rem) = mut_array_refs![dst, COPY_SIZE; ..;];
        *dst_word = *src_word;
        src = src_rem;
        dst = dst_rem;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr(), src.len());
    }
}

/// A stack and instruction efficient memset
fn fast_set(mut dst: &mut [u8], val: u8) {
    const SET_SIZE: usize = 1024;
    while dst.len() >= SET_SIZE {
        #[allow(clippy::ptr_offset_with_cast)]
        let (dst_word, dst_rem) = mut_array_refs![dst, SET_SIZE; ..;];
        *dst_word = [val; SET_SIZE];
        dst = dst_rem;
    }
    for i in dst {
        *i = val
    }
}

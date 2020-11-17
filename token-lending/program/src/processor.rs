//! Program state processor

use crate::{
    error::LendingError,
    instruction::LendingInstruction,
    math::Decimal,
    state::{LendingMarket, Obligation, Reserve},
};
use arrayref::{array_refs, mut_array_refs};
use num_traits::FromPrimitive;
use serum_dex::critbit::{AnyNode, Slab, SlabView};
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
        LendingInstruction::InitLendingMarket => {
            info!("Instruction: Init Lending Market");
            process_init_lending_market(program_id, accounts)
        }
        LendingInstruction::InitReserve => {
            info!("Instruction: Init Reserve");
            process_init_reserve(program_id, accounts)
        }
        LendingInstruction::DepositReserveLiquidity { liquidity_amount } => {
            info!("Instruction: Deposit");
            process_deposit(program_id, liquidity_amount, accounts)
        }
        LendingInstruction::WithdrawReserveLiquidity { collateral_amount } => {
            info!("Instruction: Withdraw");
            process_withdraw(program_id, collateral_amount, accounts)
        }
        LendingInstruction::BorrowReserveLiquidity { collateral_amount } => {
            info!("Instruction: Borrow");
            process_borrow(program_id, collateral_amount, accounts)
        }
        LendingInstruction::RepayReserveLiquidity { liquidity_amount } => {
            info!("Instruction: Repay");
            process_repay(program_id, liquidity_amount, accounts)
        }
    }
}

fn process_init_lending_market(_program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let lending_market_info = next_account_info(account_info_iter)?;
    let quote_token_mint_info = next_account_info(account_info_iter)?;
    let rent = &Rent::from_account_info(next_account_info(account_info_iter)?)?;
    let token_program_id = next_account_info(account_info_iter)?;

    if !rent.is_exempt(
        lending_market_info.lamports(),
        lending_market_info.data_len(),
    ) {
        info!(&rent
            .minimum_balance(lending_market_info.data_len())
            .to_string());
        return Err(LendingError::NotRentExempt.into());
    }

    if quote_token_mint_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }

    unpack_mint(&quote_token_mint_info.data.borrow())?;

    let mut new_lending_market: LendingMarket = assert_uninitialized(lending_market_info)?;
    new_lending_market.is_initialized = true;
    new_lending_market.quote_token_mint = *quote_token_mint_info.key;
    LendingMarket::pack(
        new_lending_market,
        &mut lending_market_info.data.borrow_mut(),
    )?;

    Ok(())
}

fn process_init_reserve(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let reserve_info = next_account_info(account_info_iter)?;
    let lending_market_info = next_account_info(account_info_iter)?;
    let lending_market_authority_info = next_account_info(account_info_iter)?;
    let liquidity_supply_info = next_account_info(account_info_iter)?;
    let collateral_mint_info = next_account_info(account_info_iter)?;
    let collateral_supply_info = next_account_info(account_info_iter)?;
    let collateral_output_info = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let rent_info = next_account_info(account_info_iter)?;
    let rent = &Rent::from_account_info(rent_info)?;
    let token_program_id = next_account_info(account_info_iter)?;

    if !rent.is_exempt(reserve_info.lamports(), reserve_info.data_len()) {
        info!(&rent.minimum_balance(reserve_info.data_len()).to_string());
        return Err(LendingError::NotRentExempt.into());
    }

    let lending_market = LendingMarket::unpack(&lending_market_info.data.borrow())?;
    let bump_seed = find_authority_bump_seed(program_id, &lending_market_info.key);
    if lending_market_authority_info.key
        != &authority_id(program_id, lending_market_info.key, bump_seed)?
    {
        return Err(LendingError::InvalidInput.into());
    }
    if !lending_market_info.is_signer {
        return Err(LendingError::InvalidInput.into());
    }

    if liquidity_supply_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }
    if collateral_supply_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }
    if collateral_mint_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }

    let liquidity_supply = unpack_token_account(&liquidity_supply_info.data.borrow())?;
    if liquidity_supply.owner != *lending_market_authority_info.key {
        return Err(LendingError::InvalidOwner.into());
    }
    if liquidity_supply.close_authority.is_some() {
        return Err(LendingError::InvalidCloseAuthority.into());
    }
    if liquidity_supply.delegate.is_some() {
        return Err(LendingError::InvalidDelegate.into());
    }
    if liquidity_supply.close_authority.is_some() {
        return Err(LendingError::InvalidCloseAuthority.into());
    }
    if liquidity_supply.delegate.is_some() {
        return Err(LendingError::InvalidDelegate.into());
    }
    if liquidity_supply.amount == 0 {
        return Err(LendingError::InvalidInput.into());
    }

    assert_uninitialized::<Token>(collateral_output_info)?;
    assert_uninitialized::<Token>(collateral_supply_info)?;
    assert_uninitialized::<Mint>(collateral_mint_info)?;

    spl_token_init_mint(TokenInitializeMintParams {
        mint: collateral_mint_info.clone(),
        authority: lending_market_authority_info.key,
        rent: rent_info.clone(),
        token_program: token_program_id.clone(),
    })?;

    spl_token_init_account(TokenInitializeAccountParams {
        account: collateral_supply_info.clone(),
        mint: collateral_mint_info.clone(),
        owner: lending_market_authority_info.clone(),
        rent: rent_info.clone(),
        token_program: token_program_id.clone(),
    })?;

    spl_token_init_account(TokenInitializeAccountParams {
        account: collateral_output_info.clone(),
        mint: collateral_mint_info.clone(),
        owner: lending_market_authority_info.clone(),
        rent: rent_info.clone(),
        token_program: token_program_id.clone(),
    })?;

    let mut new_reserve: Reserve = assert_uninitialized(reserve_info)?;
    new_reserve.is_initialized = true;
    new_reserve.lending_market = *lending_market_info.key;
    new_reserve.liquidity_supply = *liquidity_supply_info.key;
    new_reserve.liquidity_mint = liquidity_supply.mint;
    new_reserve.collateral_supply = *collateral_supply_info.key;
    new_reserve.collateral_mint = *collateral_mint_info.key;

    let collateral_amount: Decimal = {
        let liquidity_supply = &unpack_token_account(&liquidity_supply_info.data.borrow())?;
        let collateral_mint = &unpack_mint(&collateral_mint_info.data.borrow())?;
        new_reserve.update_cumulative_rate(clock, &liquidity_supply);
        new_reserve.liquidity_to_collateral(
            clock,
            liquidity_supply,
            collateral_mint,
            liquidity_supply.amount,
        )?
    };

    spl_token_mint_to(TokenMintToParams {
        mint: collateral_mint_info.clone(),
        destination: collateral_output_info.clone(),
        amount: collateral_amount.round_u64(),
        authority: lending_market_authority_info.clone(),
        authorized: lending_market_info.key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    let dex_market = if liquidity_supply.mint != lending_market.quote_token_mint {
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
        if lending_market.quote_token_mint != market_quote_mint {
            info!(&market_quote_mint.to_string().as_str());
            return Err(LendingError::InvalidInput.into());
        }
        if liquidity_supply.mint != market_base_mint {
            info!(&market_base_mint.to_string().as_str());
            return Err(LendingError::InvalidInput.into());
        }

        COption::Some(*dex_market_info.key)
    } else {
        COption::None
    };

    new_reserve.dex_market = dex_market;
    Reserve::pack(new_reserve, &mut reserve_info.data.borrow_mut())?;

    Ok(())
}

fn process_deposit(
    program_id: &Pubkey,
    liquidity_amount: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let reserve_info = next_account_info(account_info_iter)?;
    let lending_market_authority_info = next_account_info(account_info_iter)?;
    let liquidity_input_info = next_account_info(account_info_iter)?;
    let liquidity_supply_info = next_account_info(account_info_iter)?;
    let collateral_output_info = next_account_info(account_info_iter)?;
    let collateral_mint_info = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let token_program_id = next_account_info(account_info_iter)?;

    let mut reserve = Reserve::unpack(&reserve_info.data.borrow())?;
    let bump_seed = find_authority_bump_seed(program_id, &reserve.lending_market);
    if lending_market_authority_info.key
        != &authority_id(program_id, &reserve.lending_market, bump_seed)?
    {
        return Err(LendingError::InvalidProgramAddress.into());
    }

    if liquidity_supply_info.key != &reserve.liquidity_supply
        || collateral_mint_info.key != &reserve.collateral_mint
    {
        return Err(LendingError::InvalidInput.into());
    }
    if liquidity_supply_info.key == liquidity_input_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if collateral_output_info.key == &reserve.collateral_supply {
        return Err(LendingError::InvalidInput.into());
    }

    let liquidity_supply = &unpack_token_account(&liquidity_supply_info.data.borrow())?;
    let collateral_mint = &unpack_mint(&collateral_mint_info.data.borrow())?;
    reserve.update_cumulative_rate(clock, liquidity_supply);
    let collateral_amount = reserve.liquidity_to_collateral(
        clock,
        liquidity_supply,
        collateral_mint,
        liquidity_amount,
    )?;

    spl_token_transfer(TokenTransferParams {
        source: liquidity_input_info.clone(),
        destination: liquidity_supply_info.clone(),
        amount: liquidity_amount,
        authority: lending_market_authority_info.clone(),
        authorized: &reserve.lending_market,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    spl_token_mint_to(TokenMintToParams {
        mint: collateral_mint_info.clone(),
        destination: collateral_output_info.clone(),
        amount: collateral_amount.round_u64(),
        authority: lending_market_authority_info.clone(),
        authorized: &reserve.lending_market,
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
    let lending_market_authority_info = next_account_info(account_info_iter)?;
    let liquidity_supply_info = next_account_info(account_info_iter)?;
    let liquidity_output_info = next_account_info(account_info_iter)?;
    let collateral_mint_info = next_account_info(account_info_iter)?;
    let collateral_input_info = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let token_program_id = next_account_info(account_info_iter)?;

    let mut reserve = Reserve::unpack(&reserve_info.data.borrow())?;
    let bump_seed = find_authority_bump_seed(program_id, &reserve.lending_market);
    if lending_market_authority_info.key
        != &authority_id(program_id, &reserve.lending_market, bump_seed)?
    {
        return Err(LendingError::InvalidProgramAddress.into());
    }

    if liquidity_supply_info.key != &reserve.liquidity_supply
        || collateral_mint_info.key != &reserve.collateral_mint
    {
        return Err(LendingError::InvalidInput.into());
    }
    if liquidity_supply_info.key == liquidity_output_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if collateral_input_info.key == &reserve.collateral_supply {
        return Err(LendingError::InvalidInput.into());
    }

    let liquidity_supply = &unpack_token_account(&liquidity_supply_info.data.borrow())?;
    let collateral_mint = &unpack_mint(&collateral_mint_info.data.borrow())?;

    reserve.update_cumulative_rate(clock, liquidity_supply);
    let liquidity_withdraw_amount = reserve.collateral_to_liquidity(
        clock,
        liquidity_supply,
        collateral_mint,
        collateral_amount,
    )?;

    spl_token_transfer(TokenTransferParams {
        source: liquidity_supply_info.clone(),
        destination: liquidity_output_info.clone(),
        amount: liquidity_withdraw_amount.round_u64(),
        authority: lending_market_authority_info.clone(),
        authorized: &reserve.lending_market,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    spl_token_burn(TokenBurnParams {
        mint: collateral_mint_info.clone(),
        source: collateral_input_info.clone(),
        amount: collateral_amount,
        authority: lending_market_authority_info.clone(),
        authorized: &reserve.lending_market,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    Reserve::pack(reserve, &mut reserve_info.data.borrow_mut())?;

    Ok(())
}

enum Side {
    Bid,
    Ask,
}

#[derive(PartialEq)]
enum Fill {
    Base,
    Quote,
}

#[inline(never)] // avoid stack frame limit
fn process_borrow(
    program_id: &Pubkey,
    collateral_amount: u64,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let collateral_input_info = next_account_info(account_info_iter)?;
    let liquidity_output_info = next_account_info(account_info_iter)?;
    let deposit_reserve_info = next_account_info(account_info_iter)?;
    let deposit_reserve_collateral_mint_info = next_account_info(account_info_iter)?;
    let deposit_reserve_collateral_supply_info = next_account_info(account_info_iter)?;
    let deposit_reserve_liquidity_supply_info = next_account_info(account_info_iter)?;
    let borrow_reserve_info = next_account_info(account_info_iter)?;
    let borrow_reserve_liquidity_supply_info = next_account_info(account_info_iter)?;
    let obligation_info = next_account_info(account_info_iter)?;
    let obligation_token_mint_info = next_account_info(account_info_iter)?;
    let obligation_token_output_info = next_account_info(account_info_iter)?;
    let obligation_token_owner_info = next_account_info(account_info_iter)?;
    let lending_market_authority_info = next_account_info(account_info_iter)?;
    let dex_market_info = next_account_info(account_info_iter)?;
    let dex_market_orders_info = next_account_info(account_info_iter)?;
    let memory = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let rent_info = next_account_info(account_info_iter)?;
    let rent = &Rent::from_account_info(rent_info)?;
    let token_program_id = next_account_info(account_info_iter)?;

    if !rent.is_exempt(obligation_info.lamports(), obligation_info.data_len()) {
        info!(&rent.minimum_balance(obligation_info.data_len()).to_string());
        return Err(LendingError::NotRentExempt.into());
    }

    let deposit_reserve = Reserve::unpack(&deposit_reserve_info.data.borrow())?;
    let mut borrow_reserve = Reserve::unpack(&borrow_reserve_info.data.borrow())?;
    let deposit_reserve_collateral_supply =
        unpack_token_account(&deposit_reserve_collateral_supply_info.data.borrow())?;

    if deposit_reserve.lending_market != borrow_reserve.lending_market {
        return Err(LendingError::LendingMarketMismatch.into());
    }
    if &borrow_reserve.liquidity_supply != borrow_reserve_liquidity_supply_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if deposit_reserve.collateral_mint != deposit_reserve_collateral_supply.mint {
        return Err(LendingError::InvalidInput.into());
    }

    let lending_market_key = deposit_reserve.lending_market;
    let bump_seed = find_authority_bump_seed(program_id, &lending_market_key);

    spl_token_transfer(TokenTransferParams {
        source: collateral_input_info.clone(),
        destination: deposit_reserve_collateral_supply_info.clone(),
        amount: collateral_amount,
        authority: lending_market_authority_info.clone(),
        authorized: &lending_market_key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    // TODO: handle case when neither reserve is the quote currency
    // TODO: verify the orders account

    let deposit_reserve_liquidity_supply =
        &unpack_token_account(&deposit_reserve_liquidity_supply_info.data.borrow())?;
    let deposit_reserve_collateral_mint =
        &unpack_mint(&deposit_reserve_collateral_mint_info.data.borrow())?;
    let deposit_amount = deposit_reserve
        .collateral_to_liquidity(
            clock,
            deposit_reserve_liquidity_supply,
            deposit_reserve_collateral_mint,
            collateral_amount,
        )?
        .round_u64();
    let borrow_amount = deposit_to_borrow(
        deposit_amount,
        memory,
        dex_market_orders_info,
        dex_market_info,
        &deposit_reserve,
    )?;

    let borrow_reserve_liquidity_supply =
        &unpack_token_account(&borrow_reserve_liquidity_supply_info.data.borrow())?;
    let cumulative_borrow_rate =
        borrow_reserve.update_cumulative_rate(clock, borrow_reserve_liquidity_supply);

    spl_token_transfer(TokenTransferParams {
        source: borrow_reserve_liquidity_supply_info.clone(),
        destination: liquidity_output_info.clone(),
        amount: borrow_amount,
        authority: lending_market_authority_info.clone(),
        authorized: &lending_market_key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    borrow_reserve.add_borrow(Decimal::from(borrow_amount));
    Reserve::pack(borrow_reserve, &mut borrow_reserve_info.data.borrow_mut())?;

    if obligation_token_mint_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }
    if obligation_token_output_info.owner != token_program_id.key {
        return Err(LendingError::InvalidTokenProgram.into());
    }

    assert_uninitialized::<Mint>(obligation_token_mint_info)?;
    assert_uninitialized::<Token>(obligation_token_output_info)?;

    spl_token_init_mint(TokenInitializeMintParams {
        mint: obligation_token_mint_info.clone(),
        authority: lending_market_authority_info.key,
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
        amount: deposit_amount,
        authority: lending_market_authority_info.clone(),
        authorized: &deposit_reserve.lending_market,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    let mut new_obligation: Obligation = assert_uninitialized(obligation_info)?;
    new_obligation.last_update_slot = clock.slot;
    new_obligation.collateral_amount = collateral_amount;
    new_obligation.collateral_supply = *deposit_reserve_info.key;
    new_obligation.cumulative_borrow_rate = cumulative_borrow_rate;
    new_obligation.borrow_amount = Decimal::from(borrow_amount);
    new_obligation.borrow_reserve = *borrow_reserve_info.key;
    new_obligation.token_mint = *obligation_token_mint_info.key;
    Obligation::pack(new_obligation, &mut obligation_info.data.borrow_mut())?;

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
    let lending_market_authority_info = next_account_info(account_info_iter)?;
    let liquidity_input_info = next_account_info(account_info_iter)?;
    let liquidity_supply_info = next_account_info(account_info_iter)?;
    let collateral_supply_info = next_account_info(account_info_iter)?;
    let collateral_output_info = next_account_info(account_info_iter)?;
    let obligation_info = next_account_info(account_info_iter)?;
    let obligation_mint_info = next_account_info(account_info_iter)?;
    let obligation_input_info = next_account_info(account_info_iter)?;
    let clock = &Clock::from_account_info(next_account_info(account_info_iter)?)?;
    let token_program_id = next_account_info(account_info_iter)?;

    let mut obligation = Obligation::unpack(&obligation_info.data.borrow())?;
    if &obligation.token_mint != obligation_mint_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if &obligation.borrow_reserve != repay_reserve_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if &obligation.collateral_supply != withdraw_reserve_info.key {
        return Err(LendingError::InvalidInput.into());
    }

    let mut repay_reserve = Reserve::unpack(&repay_reserve_info.data.borrow())?;
    let withdraw_reserve = Reserve::unpack(&withdraw_reserve_info.data.borrow())?;
    if repay_reserve.lending_market != withdraw_reserve.lending_market {
        return Err(LendingError::LendingMarketMismatch.into());
    }
    if &repay_reserve.liquidity_supply != liquidity_supply_info.key {
        return Err(LendingError::InvalidInput.into());
    }
    if &withdraw_reserve.collateral_supply != collateral_supply_info.key {
        return Err(LendingError::InvalidInput.into());
    }

    let lending_market_key = repay_reserve.lending_market;
    let bump_seed = find_authority_bump_seed(program_id, &lending_market_key);

    let liquidity_supply = &unpack_token_account(&liquidity_supply_info.data.borrow())?;
    repay_reserve.update_cumulative_rate(clock, liquidity_supply);
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
        destination: liquidity_supply_info.clone(),
        amount: repay_amount,
        authority: lending_market_authority_info.clone(),
        authorized: &lending_market_key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    spl_token_transfer(TokenTransferParams {
        source: collateral_supply_info.clone(),
        destination: collateral_output_info.clone(),
        amount: collateral_withdraw_amount,
        authority: lending_market_authority_info.clone(),
        authorized: &lending_market_key,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    spl_token_burn(TokenBurnParams {
        mint: obligation_mint_info.clone(),
        source: obligation_input_info.clone(),
        authority: lending_market_authority_info.clone(),
        authorized: &lending_market_key,
        amount: obligation_token_amount,
        bump_seed,
        token_program: token_program_id.clone(),
    })?;

    obligation.last_update_slot = clock.slot;
    obligation.borrow_amount -= Decimal::from(repay_amount);
    obligation.collateral_amount -= collateral_withdraw_amount;
    Obligation::pack(obligation, &mut obligation_info.data.borrow_mut())?;

    repay_reserve.subtract_repay(Decimal::from(repay_amount));
    Reserve::pack(repay_reserve, &mut repay_reserve_info.data.borrow_mut())?;

    Ok(())
}

fn assert_uninitialized<T: Pack + IsInitialized>(
    account_info: &AccountInfo,
) -> Result<T, ProgramError> {
    let account: T = T::unpack_unchecked(&account_info.data.borrow())?;
    if account.is_initialized() {
        Err(LendingError::AlreadyInUse.into())
    } else {
        Ok(account)
    }
}

/// Generates seed bump for lending lending market authorities
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

#[inline]
fn next_order<'a>(
    orders: &'a mut RefMut<Slab>,
    side: &Side,
) -> Result<&'a mut AnyNode, ProgramError> {
    let index = match side {
        Side::Bid => orders.find_max(),
        Side::Ask => orders.find_min(),
    }
    .ok_or_else(|| ProgramError::from(LendingError::InvalidInput))?;
    Ok(orders.get_mut(index).unwrap())
}

fn exchange_with_order_book(
    mut orders: RefMut<Slab>,
    side: Side,
    fill: Fill,
    mut input_quantity: u64,
) -> Result<u64, ProgramError> {
    let mut output_quantity = Decimal::from(0);
    while input_quantity > 0 {
        let next_order_node = next_order(&mut orders, &side)?;
        let next_order = next_order_node.as_leaf_mut().unwrap();

        let mut base_quantity = next_order.quantity();
        let mut quote_quantity = base_quantity * next_order.price().get();

        if fill == Fill::Base {
            std::mem::swap(&mut base_quantity, &mut quote_quantity);
        }

        let fill_quantity = input_quantity.min(quote_quantity);
        input_quantity -= fill_quantity;
        output_quantity += Decimal::from(fill_quantity) * Decimal::from(base_quantity)
            / Decimal::from(quote_quantity);
    }

    Ok(output_quantity.round_u64())
}

fn align_orders<'a>(orders: &AccountInfo, memory: &'a AccountInfo) -> RefMut<'a, Slab> {
    let mut memory = memory.data.borrow_mut();
    {
        let bytes = orders.data.borrow();
        let start = 5 + 8;
        let end = bytes.len() - 7;
        fast_copy(&bytes[start..end], &mut memory);
    }

    RefMut::map(memory, |bytes| Slab::new(bytes))
}

fn quote_mint_pubkey(data: &[u8]) -> Pubkey {
    let count_start = 5 + 10 * 8;
    let count_end = count_start + 32;
    Pubkey::new(&data[count_start..count_end])
}

fn deposit_to_borrow(
    deposit_amount: u64,
    memory: &AccountInfo,
    dex_market_orders_info: &AccountInfo,
    dex_market_info: &AccountInfo,
    deposit_reserve: &Reserve,
) -> Result<u64, ProgramError> {
    let orders = align_orders(dex_market_orders_info, memory);
    let market_quote_mint = quote_mint_pubkey(&dex_market_info.data.borrow());
    let (fill, side) = if deposit_reserve.liquidity_mint == market_quote_mint {
        (Fill::Quote, Side::Bid)
    } else {
        (Fill::Base, Side::Ask)
    };
    let borrow_amount = exchange_with_order_book(orders, side, fill, deposit_amount)?;
    fast_set(&mut memory.data.borrow_mut(), 0);
    Ok(borrow_amount)
}

mod helpers;

use assert_matches::*;
use helpers::*;
use solana_program::{program_option::COption, program_pack::Pack};
use solana_program_test::*;
use solana_sdk::{signature::Signer, transaction::Transaction};
use spl_token::state::Account as Token;
use spl_token_lending::{
    instruction::{repay_reserve_liquidity, withdraw_reserve_liquidity},
    math::Decimal,
    state::{INITIAL_COLLATERAL_RATE, SLOTS_PER_YEAR},
};

#[tokio::test]
async fn test_transaction() {
    let (test, dex_market) = setup_test();

    let (mut banks_client, payer, recent_blockhash, bank_forks) = test.start().await;

    // Initialize Lending Market
    let quote_token_mint = dex_market.usdc_mint_pubkey;
    let lending_market = TestLendingMarket::init(&mut banks_client, quote_token_mint, &payer).await;
    let lending_market_info = lending_market.get_info(&mut banks_client).await;
    assert_eq!(lending_market_info.is_initialized, true);
    assert_eq!(lending_market_info.quote_token_mint, quote_token_mint);

    let usdc_reserve = TestReserve::init(
        &mut banks_client,
        dex_market.usdc_mint_pubkey,
        &lending_market,
        &payer,
        None,
        1000 * dex_market.price,
        Some(&dex_market.usdc_mint_authority),
        &dex_market,
    )
    .await;

    let sol_reserve = TestReserve::init(
        &mut banks_client,
        spl_token::native_mint::id(),
        &lending_market,
        &payer,
        Some(1000),
        1000,
        None,
        &dex_market,
    )
    .await;

    // Verify reserve Accounts
    let usdc_reserve_info = usdc_reserve.get_info(&mut banks_client).await;
    assert_eq!(usdc_reserve_info.is_initialized, true);
    assert_eq!(
        usdc_reserve_info.lending_market,
        lending_market.keypair.pubkey()
    );
    assert_eq!(
        usdc_reserve_info.liquidity_supply,
        usdc_reserve.liquidity_supply_pubkey
    );
    assert_eq!(
        usdc_reserve_info.collateral_supply,
        usdc_reserve.collateral_supply_pubkey
    );
    assert_eq!(
        usdc_reserve_info.collateral_mint,
        usdc_reserve.collateral_mint_pubkey
    );
    assert_eq!(usdc_reserve_info.dex_market, COption::None);
    assert_eq!(usdc_reserve_info.dex_market_price, 0);
    assert_eq!(usdc_reserve_info.dex_market_price_updated_slot, 0);

    let usdc_liquidity_supply =
        get_token_balance(&mut banks_client, usdc_reserve.liquidity_supply_pubkey).await;
    assert_eq!(usdc_liquidity_supply, 1000 * dex_market.price);

    let sol_reserve_info = sol_reserve.get_info(&mut banks_client).await;
    assert_eq!(sol_reserve_info.is_initialized, true);
    assert_eq!(
        sol_reserve_info.lending_market,
        lending_market.keypair.pubkey()
    );
    assert_eq!(
        sol_reserve_info.liquidity_supply,
        sol_reserve.liquidity_supply_pubkey
    );
    assert_eq!(
        sol_reserve_info.collateral_supply,
        sol_reserve.collateral_supply_pubkey
    );
    assert_eq!(
        sol_reserve_info.collateral_mint,
        sol_reserve.collateral_mint_pubkey
    );
    assert_eq!(
        sol_reserve_info.dex_market,
        COption::Some(dex_market.pubkey)
    );
    assert_eq!(sol_reserve_info.dex_market_price, 0);
    assert_eq!(sol_reserve_info.dex_market_price_updated_slot, 0);

    let sol_liquidity_supply =
        get_token_balance(&mut banks_client, sol_reserve.liquidity_supply_pubkey).await;
    assert_eq!(sol_liquidity_supply, 1000);

    lending_market
        .deposit(&mut banks_client, &payer, &sol_reserve, 1000)
        .await;

    let user_sol_account = banks_client
        .get_account(sol_reserve.user_token_pubkey)
        .await
        .unwrap()
        .unwrap();
    let user_sol = Token::unpack(&user_sol_account.data[..]).unwrap();
    let user_sol_collateral_account = banks_client
        .get_account(sol_reserve.user_collateral_token_pubkey)
        .await
        .unwrap()
        .unwrap();
    let user_sol_collateral = Token::unpack(&user_sol_collateral_account.data[..]).unwrap();
    assert_eq!(user_sol.amount, 0);
    assert_eq!(user_sol_collateral.amount, INITIAL_COLLATERAL_RATE * 2000);

    let obligation = lending_market
        .borrow(
            &mut banks_client,
            &payer,
            &sol_reserve,
            &usdc_reserve,
            INITIAL_COLLATERAL_RATE * 1000,
            lending_market.authority_pubkey,
            &dex_market,
        )
        .await;

    let usdc_liquidity_supply = banks_client
        .get_account(usdc_reserve.liquidity_supply_pubkey)
        .await
        .unwrap()
        .unwrap();
    let usdc_liquidity_supply = Token::unpack(&usdc_liquidity_supply.data[..]).unwrap();
    assert_eq!(usdc_liquidity_supply.amount, 0);

    // Check reserve
    let usdc_reserve_info = usdc_reserve.get_info(&mut banks_client).await;
    assert_eq!(usdc_reserve_info.total_borrows, Decimal::from(2210 * 1000));
    let usdc_borrow_rate = Decimal::new(30, 2);
    assert_eq!(
        usdc_reserve_info.current_borrow_rate(&usdc_liquidity_supply),
        usdc_borrow_rate
    );

    // Verify obligation Account
    let obligation_info = obligation.get_info(&mut banks_client).await;
    assert_eq!(obligation_info.borrow_amount.round_u64(), 2210 * 1000);
    assert_eq!(
        obligation_info.collateral_amount,
        INITIAL_COLLATERAL_RATE * 1000
    );

    {
        // Advance the clock one full year so that interest is accumulated
        use solana_sdk::sysvar::clock;
        let bank = bank_forks.write().unwrap().working_bank();
        let account = bank.get_account(&clock::id()).unwrap();
        let mut clock = bank.clock();
        clock.slot += SLOTS_PER_YEAR;
        bank.store_account(
            &clock::id(),
            &solana_sdk::account::create_account(&clock, account.lamports),
        );
    }

    let mut transaction = Transaction::new_with_payer(
        &[repay_reserve_liquidity(
            spl_token_lending::id(),
            usdc_reserve.pubkey,
            sol_reserve.pubkey,
            lending_market.authority_pubkey,
            usdc_reserve.user_token_pubkey,
            usdc_reserve.liquidity_supply_pubkey,
            dex_market.price * 1000,
            sol_reserve.collateral_supply_pubkey,
            sol_reserve.user_collateral_token_pubkey,
            obligation.pubkey,
            obligation.token_mint,
            obligation.token_account,
        )],
        Some(&payer.pubkey()),
    );

    transaction.sign(&[&payer], recent_blockhash);
    assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

    // Verify obligation Account
    let obligation_info = obligation.get_info(&mut banks_client).await;
    let remaining_borrow_amount =
        (usdc_borrow_rate * Decimal::from(dex_market.price * 1000)).round_u64();
    assert_eq!(
        obligation_info.borrow_amount.round_u64(),
        remaining_borrow_amount
    );
    let remaining_proportion: Decimal = usdc_borrow_rate / (Decimal::from(1) + usdc_borrow_rate);
    let remaining_collateral =
        (remaining_proportion * Decimal::from(INITIAL_COLLATERAL_RATE * 1000)).round_u64();
    assert_eq!(obligation_info.collateral_amount, remaining_collateral);

    let mut transaction = Transaction::new_with_payer(
        &[withdraw_reserve_liquidity(
            spl_token_lending::id(),
            sol_reserve.pubkey,
            lending_market.authority_pubkey,
            sol_reserve.liquidity_supply_pubkey,
            sol_reserve.user_token_pubkey,
            sol_reserve.collateral_mint_pubkey,
            sol_reserve.user_collateral_token_pubkey,
            INITIAL_COLLATERAL_RATE * 1000 - remaining_collateral,
        )],
        Some(&payer.pubkey()),
    );

    transaction.sign(&[&payer], recent_blockhash);
    assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

    let user_sol_account = banks_client
        .get_account(sol_reserve.user_token_pubkey)
        .await
        .unwrap()
        .unwrap();
    let user_sol = Token::unpack(&user_sol_account.data[..]).unwrap();
    let remaining_sol = (remaining_proportion * Decimal::from(1000)).round_u64();
    assert_eq!(user_sol.amount, 1000 - remaining_sol);

    let user_sol_collateral_account = banks_client
        .get_account(sol_reserve.user_collateral_token_pubkey)
        .await
        .unwrap()
        .unwrap();
    let user_sol_collateral = Token::unpack(&user_sol_collateral_account.data[..]).unwrap();
    assert_eq!(user_sol_collateral.amount, INITIAL_COLLATERAL_RATE * 1000);
}

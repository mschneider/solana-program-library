mod helpers;

use assert_matches::*;
use helpers::*;
use solana_program::{program_option::COption, program_pack::Pack};
use solana_program_test::*;
use solana_sdk::{signature::Signer, transaction::Transaction};
use spl_token::state::Account as Token;
use spl_token_lending::{instruction::repay, state::SLOTS_PER_YEAR};

#[tokio::test]
async fn test_transaction() {
    let (test, market) = setup_test();

    let (mut banks_client, payer, recent_blockhash, bank_forks) = test.start().await;

    // Initialize Pool
    let quote_token_mint = market.usdc_mint_pubkey;
    let pool = TestPool::init(&mut banks_client, quote_token_mint, &payer).await;
    let pool_info = pool.get_info(&mut banks_client).await;
    assert_eq!(pool_info.is_initialized, true);
    assert_eq!(pool_info.quote_token_mint, quote_token_mint);

    let usdc_reserve = TestReserve::init(
        &mut banks_client,
        market.usdc_mint_pubkey,
        &pool,
        &payer,
        None,
        1000 * market.price,
        Some(&market.usdc_mint_authority),
        &market,
    )
    .await;

    let sol_reserve = TestReserve::init(
        &mut banks_client,
        spl_token::native_mint::id(),
        &pool,
        &payer,
        Some(1000),
        1000,
        None,
        &market,
    )
    .await;

    // Verify reserve Accounts
    let usdc_reserve_info = usdc_reserve.get_info(&mut banks_client).await;
    assert_eq!(usdc_reserve_info.is_initialized, true);
    assert_eq!(usdc_reserve_info.pool, pool.keypair.pubkey());
    assert_eq!(
        usdc_reserve_info.liquidity_reserve,
        usdc_reserve.liquidity_reserve_pubkey
    );
    assert_eq!(
        usdc_reserve_info.collateral_reserve,
        usdc_reserve.collateral_reserve_pubkey
    );
    assert_eq!(
        usdc_reserve_info.collateral_mint,
        usdc_reserve.collateral_mint_pubkey
    );
    assert_eq!(usdc_reserve_info.dex_market, COption::None);
    assert_eq!(usdc_reserve_info.market_price, 0);
    assert_eq!(usdc_reserve_info.market_price_updated_slot, 0);

    let usdc_liquidity_reserve =
        get_token_balance(&mut banks_client, usdc_reserve.liquidity_reserve_pubkey).await;
    assert_eq!(usdc_liquidity_reserve, 1000 * market.price);

    let sol_reserve_info = sol_reserve.get_info(&mut banks_client).await;
    assert_eq!(sol_reserve_info.is_initialized, true);
    assert_eq!(sol_reserve_info.pool, pool.keypair.pubkey());
    assert_eq!(
        sol_reserve_info.liquidity_reserve,
        sol_reserve.liquidity_reserve_pubkey
    );
    assert_eq!(
        sol_reserve_info.collateral_reserve,
        sol_reserve.collateral_reserve_pubkey
    );
    assert_eq!(
        sol_reserve_info.collateral_mint,
        sol_reserve.collateral_mint_pubkey
    );
    assert_eq!(sol_reserve_info.dex_market, COption::Some(market.pubkey));
    assert_eq!(sol_reserve_info.market_price, 0);
    assert_eq!(sol_reserve_info.market_price_updated_slot, 0);

    let sol_liquidity_reserve =
        get_token_balance(&mut banks_client, sol_reserve.liquidity_reserve_pubkey).await;
    assert_eq!(sol_liquidity_reserve, 1000);

    market
        .set_price(&mut banks_client, sol_reserve.pubkey, &payer)
        .await;

    pool.deposit(&mut banks_client, &payer, &sol_reserve, 1000)
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
    assert_eq!(user_sol_collateral.amount, 1000);

    let obligation = pool
        .borrow(
            &mut banks_client,
            &payer,
            &sol_reserve,
            &usdc_reserve,
            1000,
            pool.authority_pubkey,
        )
        .await;

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
        &[repay(
            spl_token_lending::id(),
            usdc_reserve.pubkey,
            sol_reserve.pubkey,
            pool.authority_pubkey,
            usdc_reserve.user_token_pubkey,
            usdc_reserve.liquidity_reserve_pubkey,
            2204000,
            sol_reserve.collateral_reserve_pubkey,
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
    assert_eq!(obligation_info.borrow_amount.round_u64(), 661200);
}

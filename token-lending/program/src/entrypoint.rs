//! Program entrypoint definitions

use crate::{error::LendingError, processor::Processor};
use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult,
    program_error::PrintProgramError, pubkey::Pubkey,
};

entrypoint!(process_instruction);
fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    if let Err(error) = Processor::process(program_id, accounts, instruction_data) {
        // catch the error so we can print it
        error.print::<LendingError>();
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::instruction::{borrow, deposit, init_pool, init_reserve, repay, set_price};
    use crate::state::{ObligationInfo, PoolInfo, ReserveInfo};
    use assert_matches::*;
    use solana_program::{program_option::COption, program_pack::Pack};
    use solana_program_test::*;
    use solana_sdk::account::Account;
    use solana_sdk::signature::Keypair;
    use solana_sdk::system_instruction::create_account;
    use solana_sdk::{signature::Signer, transaction::Transaction};
    use spl_token::state::{Account as Token, Mint};
    use std::str::FromStr;

    async fn create_mint_account(
        banks_client: &mut BanksClient,
        payer: &Keypair,
        authority: Option<Pubkey>,
    ) -> Pubkey {
        let mint_keypair = Keypair::new();
        let mint_pubkey = mint_keypair.pubkey();
        let authority_pubkey = authority.unwrap_or_else(|| payer.pubkey());

        let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &mint_pubkey,
                    1461600,
                    Mint::LEN as u64,
                    &spl_token::id(),
                ),
                spl_token::instruction::initialize_mint(
                    &spl_token::id(),
                    &mint_pubkey,
                    &authority_pubkey,
                    None,
                    0,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &mint_keypair], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        mint_pubkey
    }

    async fn create_token_account(
        banks_client: &mut BanksClient,
        mint_pubkey: Pubkey,
        payer: &Keypair,
        authority: Option<Pubkey>,
        native_amount: Option<u64>,
    ) -> Pubkey {
        let token_keypair = Keypair::new();
        let token_pubkey = token_keypair.pubkey();
        let authority_pubkey = authority.unwrap_or_else(|| payer.pubkey());

        let lamports = 2039280 + native_amount.unwrap_or_default();
        let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &token_pubkey,
                    lamports,
                    Token::LEN as u64,
                    &spl_token::id(),
                ),
                spl_token::instruction::initialize_account(
                    &spl_token::id(),
                    &token_pubkey,
                    &mint_pubkey,
                    &authority_pubkey,
                )
                .unwrap(),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &token_keypair], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        token_pubkey
    }

    async fn mint_to(
        banks_client: &mut BanksClient,
        mint_pubkey: Pubkey,
        payer: &Keypair,
        account_pubkey: Pubkey,
        authority: &Keypair,
        amount: u64,
    ) {
        let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
        let mut transaction = Transaction::new_with_payer(
            &[spl_token::instruction::mint_to(
                &spl_token::id(),
                &mint_pubkey,
                &account_pubkey,
                &authority.pubkey(),
                &[],
                amount,
            )
            .unwrap()],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[payer, authority], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));
    }

    #[tokio::test]
    async fn test_transaction() {
        let program_id = Pubkey::new_unique();

        let mut test = ProgramTest::new(
            "spl_token_lending",
            program_id,
            processor!(process_instruction),
        );

        let sol_market_price = 2204;
        let market_pubkey = Pubkey::new_unique();
        test.add_account_with_file_data(
            market_pubkey,
            3591360,
            Pubkey::new(&[0; 32]),
            "sol_usdc_dex_market.bin",
        );

        let market_bids_pubkey =
            Pubkey::from_str("4VndUfHkmh6RWTQbXSVjY3wbSfqGjoPbuPHMoatV272H").unwrap();
        test.add_account_with_file_data(
            market_bids_pubkey,
            457104960,
            Pubkey::new(&[0; 32]),
            "sol_usdc_dex_market_bids.bin",
        );

        let market_asks_pubkey =
            Pubkey::from_str("6LTxKpMyGnbHM5rRx7f3eZHF9q3gnUBV5ucXF9LvrB3M").unwrap();
        test.add_account_with_file_data(
            market_asks_pubkey,
            457104960,
            Pubkey::new(&[0; 32]),
            "sol_usdc_dex_market_asks.bin",
        );

        // Add USD Coin mint
        let usdc_mint_authority = Keypair::new();
        let mut mint_buffer = [0u8; Mint::LEN];
        Mint {
            is_initialized: true,
            mint_authority: COption::Some(usdc_mint_authority.pubkey()),
            ..Mint::default()
        }
        .pack_into_slice(&mut mint_buffer);
        let usdc_mint = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        test.add_account_with_base64_data(
            usdc_mint,
            1,
            spl_token::id(),
            base64::encode(&mint_buffer[..]).as_str(),
        );

        test.add_program(
            "spl_token",
            spl_token::id(),
            processor!(spl_token::processor::Processor::process),
        );

        test.set_bpf_compute_max_units(3000000);

        let (mut banks_client, payer, recent_blockhash, bank_forks) = test.start().await;

        let pool_keypair = Keypair::new();
        let pool_pubkey = pool_keypair.pubkey();
        let (pool_authority_pubkey, _bump_seed) =
            Pubkey::find_program_address(&[&pool_pubkey.to_bytes()[..32]], &program_id);

        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &pool_pubkey,
                    3354720,
                    PoolInfo::LEN as u64,
                    &program_id,
                ),
                init_pool(program_id, pool_pubkey, usdc_mint),
            ],
            Some(&payer.pubkey()),
        );
        transaction.sign(&[&payer, &pool_keypair], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        // Verify Pool Account
        let pool_account: Account = banks_client
            .get_account(pool_pubkey)
            .await
            .unwrap()
            .unwrap();
        let pool_info = PoolInfo::unpack(&pool_account.data[..]).unwrap();
        assert_eq!(pool_info.is_initialized, true);
        assert_eq!(pool_info.quote_token_mint, usdc_mint);
        assert_eq!(pool_info.num_reserves, 0);
        let zeroed = Pubkey::new(&[0; 32]);
        for reserve in &pool_info.reserves[..] {
            assert_eq!(reserve, &zeroed);
        }

        let sol_reserve_keypair = Keypair::new();
        let sol_reserve_pubkey = sol_reserve_keypair.pubkey();
        let sol_collateral_token_mint_pubkey =
            create_mint_account(&mut banks_client, &payer, Some(pool_authority_pubkey)).await;

        let usdc_reserve_keypair = Keypair::new();
        let usdc_reserve_pubkey = usdc_reserve_keypair.pubkey();
        let usdc_collateral_token_mint_pubkey =
            create_mint_account(&mut banks_client, &payer, Some(pool_authority_pubkey)).await;

        let user_sol_token_pubkey = create_token_account(
            &mut banks_client,
            spl_token::native_mint::id(),
            &payer,
            Some(pool_authority_pubkey),
            Some(1000),
        )
        .await;
        let user_sol_collateral_token_pubkey = create_token_account(
            &mut banks_client,
            sol_collateral_token_mint_pubkey,
            &payer,
            Some(pool_authority_pubkey),
            None,
        )
        .await;
        let user_usdc_token_pubkey = create_token_account(
            &mut banks_client,
            usdc_mint,
            &payer,
            Some(pool_authority_pubkey),
            None,
        )
        .await;

        let sol_reserve_token_pubkey = create_token_account(
            &mut banks_client,
            spl_token::native_mint::id(),
            &payer,
            Some(pool_authority_pubkey),
            Some(1000),
        )
        .await;
        let sol_reserve_collateral_token_pubkey = create_token_account(
            &mut banks_client,
            sol_collateral_token_mint_pubkey,
            &payer,
            Some(pool_authority_pubkey),
            None,
        )
        .await;

        let usdc_reserve_token_pubkey = create_token_account(
            &mut banks_client,
            usdc_mint,
            &payer,
            Some(pool_authority_pubkey),
            None,
        )
        .await;
        mint_to(
            &mut banks_client,
            usdc_mint,
            &payer,
            usdc_reserve_token_pubkey,
            &usdc_mint_authority,
            1000 * sol_market_price,
        )
        .await;
        let usdc_reserve_collateral_token_pubkey = create_token_account(
            &mut banks_client,
            usdc_collateral_token_mint_pubkey,
            &payer,
            Some(pool_authority_pubkey),
            None,
        )
        .await;

        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &sol_reserve_pubkey,
                    2429040,
                    ReserveInfo::LEN as u64,
                    &program_id,
                ),
                init_reserve(
                    program_id,
                    sol_reserve_pubkey,
                    pool_pubkey,
                    sol_reserve_token_pubkey,
                    sol_reserve_collateral_token_pubkey,
                    sol_collateral_token_mint_pubkey,
                    Some(market_pubkey),
                ),
                create_account(
                    &payer.pubkey(),
                    &usdc_reserve_pubkey,
                    2429040,
                    ReserveInfo::LEN as u64,
                    &program_id,
                ),
                init_reserve(
                    program_id,
                    usdc_reserve_pubkey,
                    pool_pubkey,
                    usdc_reserve_token_pubkey,
                    usdc_reserve_collateral_token_pubkey,
                    usdc_collateral_token_mint_pubkey,
                    None,
                ),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(
            &[&payer, &sol_reserve_keypair, &usdc_reserve_keypair],
            recent_blockhash,
        );

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        // Verify Pool Account
        let pool_account: Account = banks_client
            .get_account(pool_pubkey)
            .await
            .unwrap()
            .unwrap();
        let pool_info = PoolInfo::unpack(&pool_account.data[..]).unwrap();
        assert_eq!(pool_info.is_initialized, true);
        assert_eq!(pool_info.quote_token_mint, usdc_mint);
        assert_eq!(pool_info.num_reserves, 2);
        assert_eq!(pool_info.reserves[0], sol_reserve_pubkey);
        assert_eq!(pool_info.reserves[1], usdc_reserve_pubkey);
        let zeroed = Pubkey::new(&[0; 32]);
        for reserve in &pool_info.reserves[2..] {
            assert_eq!(reserve, &zeroed);
        }

        // Verify reserve Account
        let reserve_account: Account = banks_client
            .get_account(sol_reserve_pubkey)
            .await
            .unwrap()
            .unwrap();
        let reserve_info = ReserveInfo::unpack(&reserve_account.data[..]).unwrap();
        assert_eq!(reserve_info.is_initialized, true);
        assert_eq!(reserve_info.pool, pool_pubkey);
        assert_eq!(reserve_info.liquidity_reserve, sol_reserve_token_pubkey);
        assert_eq!(
            reserve_info.collateral_reserve,
            sol_reserve_collateral_token_pubkey
        );
        assert_eq!(
            reserve_info.collateral_mint,
            sol_collateral_token_mint_pubkey
        );
        assert_eq!(reserve_info.dex_market, COption::Some(market_pubkey));
        assert_eq!(reserve_info.market_price, 0);
        assert_eq!(reserve_info.market_price_updated_slot, 0);

        let mut transaction = Transaction::new_with_payer(
            &[deposit(
                program_id,
                sol_reserve_pubkey,
                pool_authority_pubkey,
                1000,
                user_sol_token_pubkey,
                sol_reserve_token_pubkey,
                user_sol_collateral_token_pubkey,
                sol_collateral_token_mint_pubkey,
            )],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        let reserve_token_account: Account = banks_client
            .get_account(sol_reserve_token_pubkey)
            .await
            .unwrap()
            .unwrap();

        let reserve_token_info =
            spl_token::state::Account::unpack(&reserve_token_account.data[..]).unwrap();
        assert_eq!(reserve_token_info.amount, 2000);

        let obligation_keypair = Keypair::new();
        let obligation_pubkey = obligation_keypair.pubkey();
        let memory_keypair = Keypair::new();
        let memory_pubkey = memory_keypair.pubkey();
        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &memory_pubkey,
                    0,
                    65528,
                    &solana_program::system_program::id(),
                ),
                set_price(
                    program_id,
                    sol_reserve_pubkey,
                    market_pubkey,
                    market_bids_pubkey,
                    market_asks_pubkey,
                    memory_pubkey,
                ),
                create_account(
                    &payer.pubkey(),
                    &obligation_pubkey,
                    17260801,
                    ObligationInfo::LEN as u64,
                    &program_id,
                ),
                borrow(
                    program_id,
                    sol_reserve_pubkey,
                    usdc_reserve_pubkey,
                    pool_authority_pubkey,
                    usdc_reserve_token_pubkey,
                    user_usdc_token_pubkey,
                    user_sol_collateral_token_pubkey,
                    sol_reserve_collateral_token_pubkey,
                    1000,
                    obligation_pubkey,
                    payer.pubkey(),
                ),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(
            &[&payer, &memory_keypair, &obligation_keypair],
            recent_blockhash,
        );

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        {
            use solana_sdk::sysvar::clock;
            let bank = bank_forks.write().unwrap().working_bank();
            let account = bank.get_account(&clock::id()).unwrap();
            let mut clock = bank.clock();
            clock.slot += crate::state::SLOTS_PER_YEAR;
            bank.store_account(
                &clock::id(),
                &solana_sdk::account::create_account(&clock, account.lamports),
            );
        }

        let mut transaction = Transaction::new_with_payer(
            &[repay(
                program_id,
                usdc_reserve_pubkey,
                sol_reserve_pubkey,
                pool_authority_pubkey,
                user_usdc_token_pubkey,
                usdc_reserve_token_pubkey,
                sol_reserve_collateral_token_pubkey,
                user_sol_collateral_token_pubkey,
                2204000,
                obligation_pubkey,
                payer.pubkey(),
            )],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        // Verify obligation Account
        let obligation_account: Account = banks_client
            .get_account(obligation_pubkey)
            .await
            .unwrap()
            .unwrap();
        let obligation_info = ObligationInfo::unpack(&obligation_account.data[..]).unwrap();
        assert_eq!(obligation_info.borrow_amount.round_u64(), 661200);
    }
}

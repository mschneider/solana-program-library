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
    use crate::instruction::{deposit, init_pool, init_reserve};
    use crate::state::{PoolInfo, ReserveInfo};
    use assert_matches::*;
    use solana_program::program_pack::Pack;
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
    ) -> Pubkey {
        let token_keypair = Keypair::new();
        let token_pubkey = token_keypair.pubkey();
        let authority_pubkey = authority.unwrap_or_else(|| payer.pubkey());

        let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &token_pubkey,
                    2039280,
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

    #[tokio::test]
    async fn test_transaction() {
        let program_id = Pubkey::new_unique();

        let mut test = ProgramTest::new(
            "spl_token_lending",
            program_id,
            processor!(process_instruction),
        );

        let market_pubkey = Pubkey::new_unique();
        test.add_account_with_file_data(
            market_pubkey,
            3591360,
            Pubkey::new(&[0; 32]),
            "raw_sol_usdc_market",
        );

        // Add USD Coin mint
        let mut mint_buffer = [0u8; Mint::LEN];
        Mint {
            is_initialized: true,
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

        let (mut banks_client, payer, recent_blockhash) = test.start().await;

        let quote_token_mint_pubkey = usdc_mint;

        let pool_keypair = Keypair::new();
        let pool_pubkey = pool_keypair.pubkey();
        let (authority_pubkey, _bump_seed) =
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
                init_pool(&program_id, &pool_pubkey, &quote_token_mint_pubkey),
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
        assert_eq!(pool_info.quote_token_mint, quote_token_mint_pubkey);
        assert_eq!(pool_info.num_reserves, 0);
        let zeroed = Pubkey::new(&[0; 32]);
        for reserve in &pool_info.reserves[..] {
            assert_eq!(reserve, &zeroed);
        }

        let reserve_keypair = Keypair::new();
        let reserve_pubkey = reserve_keypair.pubkey();

        let reserve_token_mint_pubkey =
            Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();
        let collateral_token_mint_pubkey =
            create_mint_account(&mut banks_client, &payer, Some(authority_pubkey)).await;

        let deposit_source_token_pubkey = create_token_account(
            &mut banks_client,
            reserve_token_mint_pubkey,
            &payer,
            Some(authority_pubkey),
        )
        .await;
        let reserve_token_pubkey = create_token_account(
            &mut banks_client,
            reserve_token_mint_pubkey,
            &payer,
            Some(authority_pubkey),
        )
        .await;
        let reserve_collateral_token_pubkey = create_token_account(
            &mut banks_client,
            collateral_token_mint_pubkey,
            &payer,
            Some(authority_pubkey),
        )
        .await;
        let collateral_token_pubkey = create_token_account(
            &mut banks_client,
            collateral_token_mint_pubkey,
            &payer,
            Some(authority_pubkey),
        )
        .await;

        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &reserve_pubkey,
                    2122800,
                    ReserveInfo::LEN as u64,
                    &program_id,
                ),
                init_reserve(
                    &program_id,
                    &reserve_pubkey,
                    &pool_pubkey,
                    &reserve_token_pubkey,
                    &reserve_collateral_token_pubkey,
                    &collateral_token_mint_pubkey,
                    &market_pubkey,
                ),
            ],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer, &reserve_keypair], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        // Verify Pool Account
        let pool_account: Account = banks_client
            .get_account(pool_pubkey)
            .await
            .unwrap()
            .unwrap();
        let pool_info = PoolInfo::unpack(&pool_account.data[..]).unwrap();
        assert_eq!(pool_info.is_initialized, true);
        assert_eq!(pool_info.quote_token_mint, quote_token_mint_pubkey);
        assert_eq!(pool_info.num_reserves, 1);
        let zeroed = Pubkey::new(&[0; 32]);
        assert_eq!(pool_info.reserves[0], reserve_pubkey);
        for reserve in &pool_info.reserves[1..] {
            assert_eq!(reserve, &zeroed);
        }

        // Verify reserve Account
        let reserve_account: Account = banks_client
            .get_account(reserve_pubkey)
            .await
            .unwrap()
            .unwrap();
        let reserve_info = ReserveInfo::unpack(&reserve_account.data[..]).unwrap();
        assert_eq!(reserve_info.is_initialized, true);
        assert_eq!(reserve_info.pool, pool_pubkey);
        assert_eq!(reserve_info.reserve, reserve_token_pubkey);
        assert_eq!(reserve_info.collateral, reserve_collateral_token_pubkey);
        assert_eq!(
            reserve_info.liquidity_token_mint,
            collateral_token_mint_pubkey
        );
        assert_eq!(reserve_info.dex_market, market_pubkey);
        assert_eq!(reserve_info.market_price, 0);
        assert_eq!(reserve_info.market_price_updated_slot, 0);

        let mut transaction = Transaction::new_with_payer(
            &[deposit(
                &program_id,
                &reserve_pubkey,
                &authority_pubkey,
                1000,
                &deposit_source_token_pubkey,
                &reserve_token_pubkey,
                &collateral_token_pubkey,
                &collateral_token_mint_pubkey,
            )],
            Some(&payer.pubkey()),
        );

        transaction.sign(&[&payer], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));
    }
}

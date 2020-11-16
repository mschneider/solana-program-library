use assert_matches::*;
use solana_program::{program_option::COption, program_pack::Pack, pubkey::Pubkey};
use solana_program_test::*;
use solana_sdk::{
    account::Account, signature::Keypair, signature::Signer, system_instruction::create_account,
    transaction::Transaction,
};
use spl_token::state::{Account as Token, Mint};
use spl_token_lending::{
    instruction::{borrow, deposit, init_lending_market, init_reserve, set_price},
    processor::process_instruction,
    state::{LendingMarketInfo, ObligationInfo, ReserveInfo},
};
use std::str::FromStr;

pub fn setup_test() -> (ProgramTest, TestMarket) {
    let mut test = ProgramTest::new(
        "spl_token_lending",
        spl_token_lending::id(),
        processor!(process_instruction),
    );

    test.add_program(
        "spl_token",
        spl_token::id(),
        processor!(spl_token::processor::Processor::process),
    );

    let market = TestMarket::setup(&mut test);

    (test, market)
}

pub struct TestLendingMarket {
    pub keypair: Keypair,
    pub authority_pubkey: Pubkey,
}

impl TestLendingMarket {
    pub async fn init(
        banks_client: &mut BanksClient,
        quote_token_mint: Pubkey,
        payer: &Keypair,
    ) -> Self {
        let keypair = Keypair::new();
        let pubkey = keypair.pubkey();
        let (authority_pubkey, _bump_seed) =
            Pubkey::find_program_address(&[&pubkey.to_bytes()[..32]], &spl_token_lending::id());

        let rent = banks_client.get_rent().await.unwrap();
        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &pubkey,
                    rent.minimum_balance(LendingMarketInfo::LEN),
                    LendingMarketInfo::LEN as u64,
                    &spl_token_lending::id(),
                ),
                init_lending_market(spl_token_lending::id(), pubkey, quote_token_mint),
            ],
            Some(&payer.pubkey()),
        );

        let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
        transaction.sign(&[&payer, &keypair], recent_blockhash);
        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        TestLendingMarket {
            keypair,
            authority_pubkey,
        }
    }

    pub async fn deposit(
        &self,
        banks_client: &mut BanksClient,
        payer: &Keypair,
        reserve: &TestReserve,
        amount: u64,
    ) {
        let mut transaction = Transaction::new_with_payer(
            &[deposit(
                spl_token_lending::id(),
                reserve.pubkey,
                self.authority_pubkey,
                amount,
                reserve.user_token_pubkey,
                reserve.liquidity_supply_pubkey,
                reserve.user_collateral_token_pubkey,
                reserve.collateral_mint_pubkey,
            )],
            Some(&payer.pubkey()),
        );

        let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
        transaction.sign(&[payer], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));
    }

    pub async fn borrow(
        &self,
        banks_client: &mut BanksClient,
        payer: &Keypair,
        deposit_reserve: &TestReserve,
        borrow_reserve: &TestReserve,
        amount: u64,
        obligation_token_owner: Pubkey,
    ) -> TestObligation {
        let rent = banks_client.get_rent().await.unwrap();
        let obligation_keypair = Keypair::new();
        let obligation_token_mint_keypair = Keypair::new();
        let obligation_token_account_keypair = Keypair::new();

        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &obligation_token_mint_keypair.pubkey(),
                    rent.minimum_balance(Mint::LEN),
                    Mint::LEN as u64,
                    &spl_token::id(),
                ),
                create_account(
                    &payer.pubkey(),
                    &obligation_token_account_keypair.pubkey(),
                    rent.minimum_balance(Token::LEN),
                    Token::LEN as u64,
                    &spl_token::id(),
                ),
                create_account(
                    &payer.pubkey(),
                    &obligation_keypair.pubkey(),
                    rent.minimum_balance(ObligationInfo::LEN),
                    ObligationInfo::LEN as u64,
                    &spl_token_lending::id(),
                ),
                borrow(
                    spl_token_lending::id(),
                    deposit_reserve.pubkey,
                    borrow_reserve.pubkey,
                    self.authority_pubkey,
                    borrow_reserve.liquidity_supply_pubkey,
                    borrow_reserve.user_token_pubkey,
                    deposit_reserve.user_collateral_token_pubkey,
                    deposit_reserve.collateral_supply_pubkey,
                    amount,
                    obligation_keypair.pubkey(),
                    obligation_token_mint_keypair.pubkey(),
                    obligation_token_account_keypair.pubkey(),
                    obligation_token_owner,
                ),
            ],
            Some(&payer.pubkey()),
        );

        let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
        transaction.sign(
            &[
                payer,
                &obligation_keypair,
                &obligation_token_account_keypair,
                &obligation_token_mint_keypair,
            ],
            recent_blockhash,
        );

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));
        TestObligation {
            pubkey: obligation_keypair.pubkey(),
            token_mint: obligation_token_mint_keypair.pubkey(),
            token_account: obligation_token_account_keypair.pubkey(),
        }
    }

    pub async fn get_info(&self, banks_client: &mut BanksClient) -> LendingMarketInfo {
        let lending_market_account: Account = banks_client
            .get_account(self.keypair.pubkey())
            .await
            .unwrap()
            .unwrap();
        LendingMarketInfo::unpack(&lending_market_account.data[..]).unwrap()
    }
}

pub struct TestObligation {
    pub pubkey: Pubkey,
    pub token_mint: Pubkey,
    pub token_account: Pubkey,
}

impl TestObligation {
    pub async fn get_info(&self, banks_client: &mut BanksClient) -> ObligationInfo {
        let obligation_account: Account = banks_client
            .get_account(self.pubkey)
            .await
            .unwrap()
            .unwrap();
        ObligationInfo::unpack(&obligation_account.data[..]).unwrap()
    }
}

pub struct TestReserve {
    pub pubkey: Pubkey,
    pub user_token_pubkey: Pubkey,
    pub user_collateral_token_pubkey: Pubkey,
    pub liquidity_supply_pubkey: Pubkey,
    pub collateral_supply_pubkey: Pubkey,
    pub collateral_mint_pubkey: Pubkey,
}

impl TestReserve {
    pub async fn init(
        banks_client: &mut BanksClient,
        token_mint_pubkey: Pubkey,
        lending_market: &TestLendingMarket,
        payer: &Keypair,
        user_amount: Option<u64>,
        reserve_amount: u64,
        token_mint_authority: Option<&Keypair>,
        market: &TestMarket,
    ) -> Self {
        let keypair = Keypair::new();
        let pubkey = keypair.pubkey();
        let collateral_mint_keypair = Keypair::new();
        let user_collateral_token_keypair = Keypair::new();
        let collateral_supply_keypair = Keypair::new();

        let user_token_pubkey = create_token_account(
            banks_client,
            token_mint_pubkey,
            &payer,
            Some(lending_market.authority_pubkey),
            user_amount,
        )
        .await;

        let liquidity_supply_pubkey = if let Some(token_mint_authority) = token_mint_authority {
            let liquidity_supply_pubkey = create_token_account(
                banks_client,
                token_mint_pubkey,
                &payer,
                Some(lending_market.authority_pubkey),
                None,
            )
            .await;

            mint_to(
                banks_client,
                token_mint_pubkey,
                &payer,
                liquidity_supply_pubkey,
                token_mint_authority,
                reserve_amount,
            )
            .await;

            liquidity_supply_pubkey
        } else {
            create_token_account(
                banks_client,
                token_mint_pubkey,
                &payer,
                Some(lending_market.authority_pubkey),
                Some(reserve_amount),
            )
            .await
        };

        let rent = banks_client.get_rent().await.unwrap();
        let mut transaction = Transaction::new_with_payer(
            &[
                create_account(
                    &payer.pubkey(),
                    &collateral_mint_keypair.pubkey(),
                    rent.minimum_balance(Mint::LEN),
                    Mint::LEN as u64,
                    &spl_token::id(),
                ),
                create_account(
                    &payer.pubkey(),
                    &collateral_supply_keypair.pubkey(),
                    rent.minimum_balance(Token::LEN),
                    Token::LEN as u64,
                    &spl_token::id(),
                ),
                create_account(
                    &payer.pubkey(),
                    &user_collateral_token_keypair.pubkey(),
                    rent.minimum_balance(Token::LEN),
                    Token::LEN as u64,
                    &spl_token::id(),
                ),
                create_account(
                    &payer.pubkey(),
                    &pubkey,
                    rent.minimum_balance(ReserveInfo::LEN),
                    ReserveInfo::LEN as u64,
                    &spl_token_lending::id(),
                ),
                init_reserve(
                    spl_token_lending::id(),
                    pubkey,
                    lending_market.keypair.pubkey(),
                    liquidity_supply_pubkey,
                    collateral_mint_keypair.pubkey(),
                    collateral_supply_keypair.pubkey(),
                    user_collateral_token_keypair.pubkey(),
                    Some(market.pubkey),
                ),
            ],
            Some(&payer.pubkey()),
        );

        let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
        transaction.sign(
            &vec![
                payer,
                &keypair,
                &lending_market.keypair,
                &collateral_mint_keypair,
                &collateral_supply_keypair,
                &user_collateral_token_keypair,
            ],
            recent_blockhash,
        );

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

        Self {
            pubkey,
            user_token_pubkey,
            user_collateral_token_pubkey: user_collateral_token_keypair.pubkey(),
            liquidity_supply_pubkey,
            collateral_supply_pubkey: collateral_supply_keypair.pubkey(),
            collateral_mint_pubkey: collateral_mint_keypair.pubkey(),
        }
    }

    pub async fn get_info(&self, banks_client: &mut BanksClient) -> ReserveInfo {
        let reserve_account: Account = banks_client
            .get_account(self.pubkey)
            .await
            .unwrap()
            .unwrap();
        ReserveInfo::unpack(&reserve_account.data[..]).unwrap()
    }
}

pub struct TestMarket {
    pub pubkey: Pubkey,
    pub price: u64,
    pub bids_pubkey: Pubkey,
    pub asks_pubkey: Pubkey,
    pub usdc_mint_pubkey: Pubkey,
    pub usdc_mint_authority: Keypair,
}

impl TestMarket {
    pub fn setup(test: &mut ProgramTest) -> TestMarket {
        let price = 2204; // USDC (3 decimals) per SOL
        let pubkey = Pubkey::new_unique();
        test.add_account_with_file_data(
            pubkey,
            u32::MAX as u64,
            Pubkey::new(&[0; 32]),
            "sol_usdc_dex_market.bin",
        );

        let bids_pubkey = Pubkey::from_str("4VndUfHkmh6RWTQbXSVjY3wbSfqGjoPbuPHMoatV272H").unwrap();
        test.add_account_with_file_data(
            bids_pubkey,
            u32::MAX as u64,
            Pubkey::new(&[0; 32]),
            "sol_usdc_dex_market_bids.bin",
        );

        let asks_pubkey = Pubkey::from_str("6LTxKpMyGnbHM5rRx7f3eZHF9q3gnUBV5ucXF9LvrB3M").unwrap();
        test.add_account_with_file_data(
            asks_pubkey,
            u32::MAX as u64,
            Pubkey::new(&[0; 32]),
            "sol_usdc_dex_market_asks.bin",
        );

        let usdc_mint_authority = Keypair::new();
        let mut mint_buffer = [0u8; Mint::LEN];
        Mint {
            is_initialized: true,
            mint_authority: COption::Some(usdc_mint_authority.pubkey()),
            ..Mint::default()
        }
        .pack_into_slice(&mut mint_buffer);

        let usdc_mint_pubkey =
            Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        test.add_account_with_base64_data(
            usdc_mint_pubkey,
            u32::MAX as u64,
            spl_token::id(),
            base64::encode(&mint_buffer[..]).as_str(),
        );

        Self {
            pubkey,
            price,
            bids_pubkey,
            usdc_mint_pubkey,
            usdc_mint_authority,
            asks_pubkey,
        }
    }

    pub async fn set_price(
        &self,
        banks_client: &mut BanksClient,
        reserve_pubkey: Pubkey,
        payer: &Keypair,
    ) {
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
                    spl_token_lending::id(),
                    reserve_pubkey,
                    self.pubkey,
                    self.bids_pubkey,
                    self.asks_pubkey,
                    memory_pubkey,
                ),
            ],
            Some(&payer.pubkey()),
        );

        let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
        transaction.sign(&[&payer, &memory_keypair], recent_blockhash);

        assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));
    }
}

pub async fn create_token_account(
    banks_client: &mut BanksClient,
    mint_pubkey: Pubkey,
    payer: &Keypair,
    authority: Option<Pubkey>,
    native_amount: Option<u64>,
) -> Pubkey {
    let token_keypair = Keypair::new();
    let token_pubkey = token_keypair.pubkey();
    let authority_pubkey = authority.unwrap_or_else(|| payer.pubkey());

    let rent = banks_client.get_rent().await.unwrap();
    let lamports = rent.minimum_balance(Token::LEN) + native_amount.unwrap_or_default();
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

    let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
    transaction.sign(&[&payer, &token_keypair], recent_blockhash);

    assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));

    token_pubkey
}

pub async fn mint_to(
    banks_client: &mut BanksClient,
    mint_pubkey: Pubkey,
    payer: &Keypair,
    account_pubkey: Pubkey,
    authority: &Keypair,
    amount: u64,
) {
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

    let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();
    transaction.sign(&[payer, authority], recent_blockhash);

    assert_matches!(banks_client.process_transaction(transaction).await, Ok(()));
}

pub async fn get_token_balance(banks_client: &mut BanksClient, pubkey: Pubkey) -> u64 {
    let token: Account = banks_client.get_account(pubkey).await.unwrap().unwrap();

    spl_token::state::Account::unpack(&token.data[..])
        .unwrap()
        .amount
}

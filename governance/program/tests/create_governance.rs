#![cfg(feature = "test-bpf")]

use solana_program_test::*;
use solana_sdk::{
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use spl_governance::{
    id,
    instruction::{create_dummy_account, create_governance},
    processor::process_instruction,
    state::governance::{Governance, GOVERNANCE_NAME_LENGTH},
    PROGRAM_AUTHORITY_SEED,
};

use solana_program::{program_pack::Pack, pubkey::Pubkey};

#[tokio::test]
async fn test_dummy_created() {
    let (mut banks_client, payer, recent_blockhash) = ProgramTest::new(
        "spl_governance",
        spl_governance::id(),
        processor!(process_instruction),
    )
    .start()
    .await;

    let rent = banks_client.get_rent().await.unwrap();

    let i1 = create_dummy_account().unwrap();

    let mut transaction = Transaction::new_with_payer(&[i1], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    banks_client.process_transaction(transaction).await.unwrap();
}

#[tokio::test]
async fn test_created() {
    // Arrange
    let (mut banks_client, payer, recent_blockhash) = ProgramTest::new(
        "spl_governance",
        spl_governance::id(),
        processor!(process_instruction),
    )
    .start()
    .await;

    let governed_program_key = Pubkey::new_unique();
    let governed_program_data_key = Pubkey::new_unique();
    let governed_program_upgrade_authority_keypair = Keypair::new();

    let (governance_key, _) = Pubkey::find_program_address(
        &[PROGRAM_AUTHORITY_SEED, governed_program_key.as_ref()],
        &id(),
    );

    let governance_mint_key = Pubkey::new_unique();
    let council_mint_key = Option::None::<Pubkey>;

    let vote_threshold: u8 = 60;
    let minimum_slot_waiting_period: u64 = 10;
    let time_limit: u64 = 100;
    let name = [0u8; GOVERNANCE_NAME_LENGTH];

    let i1 = create_governance(
        &governance_key,
        &governed_program_key,
        &governed_program_data_key,
        &governed_program_upgrade_authority_keypair.pubkey(),
        &governance_mint_key,
        &payer.pubkey(),
        &council_mint_key,
        vote_threshold,
        minimum_slot_waiting_period,
        time_limit,
        &name,
    )
    .unwrap();

    let mut transaction = Transaction::new_with_payer(&[i1], Some(&payer.pubkey()));
    transaction.sign(
        &[&payer, &governed_program_upgrade_authority_keypair],
        recent_blockhash,
    );

    // Act
    banks_client.process_transaction(transaction).await.unwrap();

    // Assert
    let governance_account_raw = banks_client
        .get_account(governance_key)
        .await
        .unwrap()
        .unwrap();

    let governance_account = Governance::unpack(&governance_account_raw.data).unwrap();

    assert_eq!(vote_threshold, governance_account.vote_threshold);
    assert_eq!(
        minimum_slot_waiting_period,
        governance_account.minimum_slot_waiting_period
    );
    assert_eq!(time_limit, governance_account.time_limit);
    assert_eq!(name, governance_account.name);
    assert_eq!(governance_mint_key, governance_account.governance_mint);
    assert_eq!(true, governance_account.council_mint.is_none());
}

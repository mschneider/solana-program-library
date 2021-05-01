#![cfg(feature = "test-bpf")]

use solana_program_test::*;
use solana_sdk::{signature::Signer, transaction::Transaction};
use spl_governance::{id, instruction::create_gov_account, processor::process_instruction};

use solana_program::pubkey::Pubkey;

#[tokio::test]
async fn test_created() {
    let (mut banks_client, payer, recent_blockhash) = ProgramTest::new(
        "spl_governance",
        spl_governance::id(),
        processor!(process_instruction),
    )
    .start()
    .await;

    let rent = banks_client.get_rent().await.unwrap();

    let i1 = create_gov_account(&id()).unwrap();

    let mut transaction = Transaction::new_with_payer(&[i1], Some(&payer.pubkey()));
    transaction.sign(&[&payer], recent_blockhash);
    banks_client.process_transaction(transaction).await;
}

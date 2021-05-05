use solana_program::pubkey::Pubkey;
use solana_program_test::ProgramTest;
use solana_program_test::*;
use solana_sdk::{hash::Hash, signature::Keypair, transaction::Transaction};
use spl_governance::{instruction::create_dummy_account, processor::process_instruction};

use solana_sdk::signature::Signer;

pub struct GovernanceProgramTest {
    pub banks_client: BanksClient,
    pub payer: Keypair,
    pub recent_blockhash: Hash,
    pub governed_program: Option<GovernedProgram>,
}

pub struct GovernedProgram {
    pub address: Pubkey,
}

impl GovernanceProgramTest {
    pub async fn start_new() -> Self {
        let program_test = ProgramTest::new(
            "spl_governance",
            spl_governance::id(),
            processor!(process_instruction),
        );

        let (banks_client, payer, recent_blockhash) = program_test.start().await;

        Self {
            banks_client,
            payer,
            recent_blockhash,
            governed_program: None,
        }
    }

    pub fn with_governed_program(&mut self) -> &mut Self {
        self.governed_program = Some(GovernedProgram {
            address: Pubkey::new_unique(),
        });
        self
    }

    pub async fn with_dummy_account(&mut self) -> &mut Self {
        let i1 = create_dummy_account().unwrap();

        let mut transaction = Transaction::new_with_payer(&[i1], Some(&self.payer.pubkey()));
        transaction.sign(&[&self.payer], self.recent_blockhash);
        self.banks_client
            .process_transaction(transaction)
            .await
            .unwrap();

        self
    }
}

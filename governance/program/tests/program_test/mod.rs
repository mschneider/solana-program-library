use std::{env, fs::File, io::Read, path::PathBuf};

use solana_program::{
    bpf_loader_upgradeable::{self, UpgradeableLoaderState},
    instruction::Instruction,
    pubkey::Pubkey,
    rent::Rent,
};
use solana_program_test::ProgramTest;
use solana_program_test::*;
use solana_sdk::{hash::Hash, signature::Keypair, transaction::Transaction};
use spl_governance::{instruction::create_dummy_account, processor::process_instruction};

use solana_sdk::signature::Signer;

pub struct GovernanceProgramTest {
    pub banks_client: BanksClient,
    pub payer: Keypair,
    pub recent_blockhash: Hash,
    pub rent: Rent,
    pub governed_program: Option<GovernedProgram>,
}

pub struct GovernedProgram {
    pub address: Pubkey,
    pub upgrade_authority: Keypair,
    pub data_address: Pubkey,
}

fn get_bpf_program_path(name: &str) -> PathBuf {
    let mut pathbuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    pathbuf.push("tests/program_test");
    pathbuf.push(name);
    pathbuf.set_extension("_so");
    pathbuf
}

fn read_bpf_program(name: &str) -> Vec<u8> {
    let path = get_bpf_program_path(name);
    let mut file = File::open(&path).unwrap_or_else(|err| {
        panic!("Failed to open {}: {}", path.display(), err);
    });
    let mut elf = Vec::new();
    file.read_to_end(&mut elf).unwrap();

    elf
}

impl GovernanceProgramTest {
    pub async fn start_new() -> Self {
        let mut program_test = ProgramTest::new(
            "spl_governance",
            spl_governance::id(),
            processor!(process_instruction),
        );

        program_test.add_program(
            "solana_bpf_loader_upgradeable_program",
            bpf_loader_upgradeable::id(),
            Some(solana_bpf_loader_program::process_instruction),
        );

        let (mut banks_client, payer, recent_blockhash) = program_test.start().await;

        let rent = banks_client.get_rent().await.unwrap();

        Self {
            banks_client,
            payer,
            recent_blockhash,
            rent,
            governed_program: None,
        }
    }

    async fn process_transaction(
        &mut self,
        instructions: &[Instruction],
        signers: Option<&[&Keypair]>,
    ) {
        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));

        let mut all_signers = vec![&self.payer];

        if let Some(signers) = signers {
            all_signers.extend_from_slice(signers);
        }

        transaction.sign(&all_signers, self.recent_blockhash);

        self.banks_client
            .process_transaction(transaction)
            .await
            .unwrap();
    }

    pub async fn with_governed_program(&mut self) -> &mut Self {
        let program_keypair = Keypair::new();
        let program_buffer_keypair = Keypair::new();
        let program_upgrade_authority = Keypair::new();

        let (program_data_address, _) = Pubkey::find_program_address(
            &[program_keypair.pubkey().as_ref()],
            &bpf_loader_upgradeable::id(),
        );

        // Load solana_bpf_rust_upgradeable program taken from solana test programs
        let program_data = read_bpf_program("solana_bpf_rust_upgradeable");

        let program_buffer_rent = self
            .rent
            .minimum_balance(UpgradeableLoaderState::programdata_len(program_data.len()).unwrap());

        let mut instructions = bpf_loader_upgradeable::create_buffer(
            &self.payer.pubkey(),
            &program_buffer_keypair.pubkey(),
            &program_upgrade_authority.pubkey(),
            program_buffer_rent,
            program_data.len(),
        )
        .unwrap();

        let chunk_size = 800;

        for (chunk, i) in program_data.chunks(chunk_size).zip(0..) {
            instructions.push(bpf_loader_upgradeable::write(
                &program_buffer_keypair.pubkey(),
                &program_upgrade_authority.pubkey(),
                (i * chunk_size) as u32,
                chunk.to_vec(),
            ));
        }

        let program_account_rent = self
            .rent
            .minimum_balance(UpgradeableLoaderState::program_len().unwrap());

        let deploy_instructions = bpf_loader_upgradeable::deploy_with_max_program_len(
            &self.payer.pubkey(),
            &program_keypair.pubkey(),
            &program_buffer_keypair.pubkey(),
            &program_upgrade_authority.pubkey(),
            program_account_rent,
            program_data.len(),
        )
        .unwrap();

        instructions.extend_from_slice(&deploy_instructions);

        self.process_transaction(
            &instructions[..],
            Some(&[
                &program_upgrade_authority,
                &program_keypair,
                &program_buffer_keypair,
            ]),
        )
        .await;

        self.governed_program = Some(GovernedProgram {
            address: program_keypair.pubkey(),
            upgrade_authority: program_upgrade_authority,
            data_address: program_data_address,
        });
        self
    }

    pub async fn with_dummy_account(&mut self) -> &mut Self {
        let i1 = create_dummy_account().unwrap();

        self.process_transaction(&[i1], None).await;

        self
    }
}

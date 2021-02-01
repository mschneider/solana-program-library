pub mod process_add_signer;
pub mod process_init_timelock_program;
pub mod process_init_timelock_set;
pub mod process_remove_signer;

use crate::instruction::TimelockInstruction;
use process_add_signer::process_add_signer;
use process_init_timelock_program::process_init_timelock_program;
use process_init_timelock_set::process_init_timelock_set;
use process_remove_signer::process_remove_signer;

use solana_program::{account_info::AccountInfo, entrypoint::ProgramResult, msg, pubkey::Pubkey};

/// Processes an instruction
pub fn process_instruction<'a>(
    program_id: &'a Pubkey,
    accounts: &'a [AccountInfo<'a>],
    input: &[u8],
) -> ProgramResult {
    let instruction = TimelockInstruction::unpack(input)?;
    match instruction {
        TimelockInstruction::InitTimelockProgram => {
            msg!("Instruction: Init Timelock Program");
            process_init_timelock_program(program_id, accounts)
        }
        TimelockInstruction::InitTimelockSet { config } => {
            msg!("Instruction: Init Timelock Set");
            process_init_timelock_set(program_id, accounts)
        }
        TimelockInstruction::AddSigner => {
            msg!("Instruction: Add Signer");
            process_add_signer(program_id, accounts)
        }
        TimelockInstruction::RemoveSigner => {
            msg!("Instruction: Remove Signer");
            process_remove_signer(program_id, accounts)
        }
        TimelockInstruction::AddCustomSingleSignerV1Transaction { slot, instruction } => Ok(()),
        TimelockInstruction::RemoveTransaction {} => Ok(()),
        TimelockInstruction::UpdateTransactionSlot { slot } => Ok(()),
        TimelockInstruction::DeleteTimelockSet {} => Ok(()),
        TimelockInstruction::Sign {} => Ok(()),
        TimelockInstruction::Vote {
            voting_token_amount,
        } => Ok(()),
        TimelockInstruction::MintVotingTokens {
            voting_token_amount,
        } => Ok(()),
    }
}
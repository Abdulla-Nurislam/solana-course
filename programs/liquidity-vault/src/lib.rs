use anchor_lang::prelude::*;

pub mod error;
pub mod instructions;
pub mod math;
pub mod state;

use instructions::*;

declare_id!("CFvWtQbdLxeJz55CfJLbpqyqFaUFsNZPuWMU7KcPgnE2");

#[program]
pub mod liquidity_vault {
    use super::*;

    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        instructions::initialize_vault::initialize_vault(ctx)
    }

    pub fn create_user_card(ctx: Context<CreateUserCard>) -> Result<()> {
        instructions::create_user_card::create_user_card(ctx)
    }

    /// Неделя 9: Депозит базовых токенов в vault и выпуск share-токенов пропорционально доле.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        instructions::deposit::deposit(ctx, amount)
    }
}

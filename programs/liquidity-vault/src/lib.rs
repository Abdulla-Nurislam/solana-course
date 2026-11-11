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

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        instructions::deposit::deposit(ctx, amount)
    }

    /// Неделя 10: Вывод средств из vault через CPI со сжиганием share-токенов и возвратом доли подписью PDA.
    pub fn withdraw(ctx: Context<Withdraw>, shares: u64) -> Result<()> {
        instructions::withdraw::withdraw(ctx, shares)
    }
}

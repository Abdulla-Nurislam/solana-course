use anchor_lang::prelude::*;

pub mod error;
pub mod instructions;
pub mod math;
pub mod raydium;
pub mod state;

use instructions::*;

declare_id!("CFvWtQbdLxeJz55CfJLbpqyqFaUFsNZPuWMU7KcPgnE2");

#[program]
pub mod liquidity_vault {
    use super::*;

    /// Неделя 8: Создание каркаса vault (настройки `VaultState`, хранилище `token_vault` и `share_mint`).
    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        instructions::initialize_vault::initialize_vault(ctx)
    }

    /// Неделя 8: Создание персональной карточки пользователя `UserCard` на PDA.
    pub fn create_user_card(ctx: Context<CreateUserCard>) -> Result<()> {
        instructions::create_user_card::create_user_card(ctx)
    }

    /// Неделя 9: Депозит базовых токенов в vault и выпуск share-токенов пропорционально доле.
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        instructions::deposit::deposit(ctx, amount)
    }

    /// Неделя 10: Вывод средств из vault через CPI со сжиганием share-токенов и возвратом доли подписью PDA.
    pub fn withdraw(ctx: Context<Withdraw>, shares: u64) -> Result<()> {
        instructions::withdraw::withdraw(ctx, shares)
    }

    /// Неделя 12: Привязка стратегии Raydium CLMM и создание второго хранилища `secondary_vault`.
    pub fn init_strategy(ctx: Context<InitStrategy>) -> Result<()> {
        instructions::init_strategy::init_strategy(ctx)
    }

    /// Неделя 12: Обмен токенов из vault через пул Raydium CLMM (`swap_v2`) с проверкой `min_amount_out`.
    pub fn swap<'info>(
        ctx: Context<'info, SwapVault<'info>>,
        amount_in: u64,
        min_amount_out: u64,
        sqrt_price_limit_x64: u128,
        a_to_b: bool,
    ) -> Result<()> {
        instructions::swap::swap(ctx, amount_in, min_amount_out, sqrt_price_limit_x64, a_to_b)
    }

    /// Неделя 13: Открытие концентрированной позиции vault в пуле Raydium CLMM в заданном диапазоне тиков.
    pub fn open_position<'info>(
        ctx: Context<'info, OpenVaultPosition<'info>>,
        tick_lower_index: i32,
        tick_upper_index: i32,
        tick_array_lower_start_index: i32,
        tick_array_upper_start_index: i32,
        liquidity: u128,
        amount_0_max: u64,
        amount_1_max: u64,
    ) -> Result<()> {
        instructions::position::open_position(
            ctx,
            tick_lower_index,
            tick_upper_index,
            tick_array_lower_start_index,
            tick_array_upper_start_index,
            liquidity,
            amount_0_max,
            amount_1_max,
        )
    }

    /// Неделя 13: Сбор накопленных торговых комиссий позиции vault в Raydium CLMM (`liquidity = 0`).
    pub fn collect_fees<'info>(
        ctx: Context<'info, CollectVaultFees<'info>>,
    ) -> Result<()> {
        instructions::position::collect_fees(ctx)
    }

    /// Неделя 13: Вывод всей ликвидности из позиции с проверкой минимумов и закрытие NFT-позиции.
    pub fn close_position<'info>(
        ctx: Context<'info, CloseVaultPosition<'info>>,
        amount_0_min: u64,
        amount_1_min: u64,
    ) -> Result<()> {
        instructions::position::close_position(ctx, amount_0_min, amount_1_min)
    }
}

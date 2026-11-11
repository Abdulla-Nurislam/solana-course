use anchor_lang::prelude::*;
use anchor_spl::{
    token_2022::Token2022,
    token_interface::{Mint, TokenAccount, TokenInterface},
};

use crate::{
    error::VaultError,
    raydium::{invoke_swap_v2, parse_pool_state},
    state::{Strategy, VaultState, STRATEGY_SEED, VAULT_SEED},
};

#[derive(Accounts)]
pub struct SwapVault<'info> {
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED, vault.deposit_mint.as_ref()],
        bump = vault.bump,
        has_one = admin @ VaultError::Unauthorized,
        has_one = token_vault @ VaultError::InvalidVaultTokenAccount
    )]
    pub vault: Account<'info, VaultState>,

    #[account(
        mut,
        seeds = [STRATEGY_SEED, vault.key().as_ref()],
        bump = strategy.bump,
        has_one = vault @ VaultError::Unauthorized,
        has_one = secondary_vault @ VaultError::InvalidVaultTokenAccount,
        has_one = clmm_program @ VaultError::InvalidClmmProgram,
        has_one = amm_config @ VaultError::InvalidAmmConfig,
        has_one = pool_state @ VaultError::InvalidPoolState
    )]
    pub strategy: Account<'info, Strategy>,

    #[account(
        mut,
        token::mint = deposit_mint,
        token::authority = vault
    )]
    pub token_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = secondary_mint,
        token::authority = vault
    )]
    pub secondary_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(address = vault.deposit_mint @ VaultError::InvalidPoolMints)]
    pub deposit_mint: InterfaceAccount<'info, Mint>,

    #[account(address = strategy.secondary_mint @ VaultError::InvalidPoolMints)]
    pub secondary_mint: InterfaceAccount<'info, Mint>,

    /// CHECK: Проверено через `has_one = clmm_program` в `strategy`.
    #[account(constraint = clmm_program.executable @ VaultError::InvalidClmmProgram)]
    pub clmm_program: UncheckedAccount<'info>,

    /// CHECK: Проверено через `has_one = amm_config` в `strategy`.
    pub amm_config: UncheckedAccount<'info>,

    /// CHECK: Проверено через `has_one = pool_state` в `strategy` и `parse_pool_state`.
    #[account(mut)]
    pub pool_state: UncheckedAccount<'info>,

    /// CHECK: Входное хранилище пула Raydium CLMM; сверяется с `PoolState`.
    #[account(mut)]
    pub input_vault: UncheckedAccount<'info>,

    /// CHECK: Выходное хранилище пула Raydium CLMM; сверяется с `PoolState`.
    #[account(mut)]
    pub output_vault: UncheckedAccount<'info>,

    /// CHECK: Аккаунт оракула наблюдений пула Raydium CLMM; сверяется с `PoolState`.
    #[account(mut)]
    pub observation_state: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub token_program_2022: Program<'info, Token2022>,

    /// CHECK: SPL Memo Program (`MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`).
    pub memo_program: UncheckedAccount<'info>,
}

pub fn swap<'info>(
    ctx: Context<'info, SwapVault<'info>>,
    amount_in: u64,
    min_amount_out: u64,
    sqrt_price_limit_x64: u128,
    a_to_b: bool,
) -> Result<()> {
    require!(amount_in > 0, VaultError::ZeroAmount);
    require!(min_amount_out > 0, VaultError::InvalidMinOutput);

    let parsed_pool = parse_pool_state(
        &ctx.accounts.pool_state.to_account_info(),
        &ctx.accounts.clmm_program.key(),
    )?;
    require_keys_eq!(
        parsed_pool.observation_key,
        ctx.accounts.observation_state.key(),
        VaultError::InvalidPoolState
    );

    let (
        input_token_account,
        output_token_account,
        input_mint_info,
        output_mint_info,
        output_before,
    ) = if a_to_b {
        // Обмен базового токена vault (deposit_mint) на второй токен (secondary_mint)
        require!(
            ctx.accounts.token_vault.amount >= amount_in,
            VaultError::InsufficientVaultLiquidity
        );
        (
            ctx.accounts.token_vault.to_account_info(),
            ctx.accounts.secondary_vault.to_account_info(),
            ctx.accounts.deposit_mint.to_account_info(),
            ctx.accounts.secondary_mint.to_account_info(),
            ctx.accounts.secondary_vault.amount,
        )
    } else {
        // Обратный обмен второго токена (secondary_mint) в базовый токен (deposit_mint)
        require!(
            ctx.accounts.secondary_vault.amount >= amount_in,
            VaultError::InsufficientVaultLiquidity
        );
        (
            ctx.accounts.secondary_vault.to_account_info(),
            ctx.accounts.token_vault.to_account_info(),
            ctx.accounts.secondary_mint.to_account_info(),
            ctx.accounts.deposit_mint.to_account_info(),
            ctx.accounts.token_vault.amount,
        )
    };

    // Проверяем соответствие хранилищ пула направлению обмена
    let (expected_in_vault, expected_out_vault) =
        if *input_mint_info.key == parsed_pool.token_mint_0 {
            (parsed_pool.token_vault_0, parsed_pool.token_vault_1)
        } else {
            (parsed_pool.token_vault_1, parsed_pool.token_vault_0)
        };
    require_keys_eq!(
        ctx.accounts.input_vault.key(),
        expected_in_vault,
        VaultError::InvalidPoolVaults
    );
    require_keys_eq!(
        ctx.accounts.output_vault.key(),
        expected_out_vault,
        VaultError::InvalidPoolVaults
    );

    let deposit_mint_key = ctx.accounts.vault.deposit_mint;
    let signer_seeds: &[&[&[u8]]] = &[&[
        VAULT_SEED,
        deposit_mint_key.as_ref(),
        &[ctx.accounts.vault.bump],
    ]];

    invoke_swap_v2(
        &ctx.accounts.clmm_program.to_account_info(),
        &ctx.accounts.vault.to_account_info(),
        &ctx.accounts.amm_config.to_account_info(),
        &ctx.accounts.pool_state.to_account_info(),
        &input_token_account,
        &output_token_account,
        &ctx.accounts.input_vault.to_account_info(),
        &ctx.accounts.output_vault.to_account_info(),
        &ctx.accounts.observation_state.to_account_info(),
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.token_program_2022.to_account_info(),
        &ctx.accounts.memo_program.to_account_info(),
        &input_mint_info,
        &output_mint_info,
        ctx.remaining_accounts,
        amount_in,
        min_amount_out,
        sqrt_price_limit_x64,
        true,
        signer_seeds,
    )?;

    let output_after = if a_to_b {
        ctx.accounts.secondary_vault.reload()?;
        ctx.accounts.secondary_vault.amount
    } else {
        ctx.accounts.token_vault.reload()?;
        ctx.accounts.token_vault.amount
    };

    let received = output_after
        .checked_sub(output_before)
        .ok_or(VaultError::MathOverflow)?;
    require!(received >= min_amount_out, VaultError::SlippageExceeded);

    Ok(())
}

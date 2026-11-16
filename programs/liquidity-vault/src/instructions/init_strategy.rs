use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::{
    error::VaultError,
    raydium::parse_pool_state,
    state::{Strategy, VaultState, STRATEGY_SEED, STRATEGY_VAULT_SEED, VAULT_SEED},
};

#[derive(Accounts)]
pub struct InitStrategy<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        seeds = [VAULT_SEED, vault.deposit_mint.as_ref()],
        bump = vault.bump,
        has_one = admin @ VaultError::Unauthorized
    )]
    pub vault: Account<'info, VaultState>,

    #[account(
        init,
        payer = admin,
        space = 8 + Strategy::INIT_SPACE,
        seeds = [STRATEGY_SEED, vault.key().as_ref()],
        bump
    )]
    pub strategy: Account<'info, Strategy>,

    #[account(
        mint::token_program = token_program,
        constraint = secondary_mint.key() != vault.deposit_mint @ VaultError::InvalidPoolMints
    )]
    pub secondary_mint: InterfaceAccount<'info, Mint>,

    #[account(
        init,
        payer = admin,
        token::mint = secondary_mint,
        token::authority = vault,
        token::token_program = token_program,
        seeds = [STRATEGY_VAULT_SEED, vault.key().as_ref()],
        bump
    )]
    pub secondary_vault: InterfaceAccount<'info, TokenAccount>,

    /// CHECK: Исполняемая программа Raydium CLMM; проверяется `executable` и владение `pool_state`.
    #[account(constraint = clmm_program.executable @ VaultError::InvalidClmmProgram)]
    pub clmm_program: UncheckedAccount<'info>,

    /// CHECK: Аккаунт конфигурации комиссий пула Raydium CLMM; сверяется с `PoolState.amm_config`.
    pub amm_config: UncheckedAccount<'info>,

    /// CHECK: Аккаунт `PoolState` Raydium CLMM; проверяется владельцем, дискриминатором и минтами.
    pub pool_state: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn init_strategy(ctx: Context<InitStrategy>) -> Result<()> {
    let parsed_pool = parse_pool_state(
        &ctx.accounts.pool_state.to_account_info(),
        &ctx.accounts.clmm_program.key(),
    )?;

    require_keys_eq!(
        parsed_pool.amm_config,
        ctx.accounts.amm_config.key(),
        VaultError::InvalidAmmConfig
    );

    let deposit_mint = ctx.accounts.vault.deposit_mint;
    let secondary_mint = ctx.accounts.secondary_mint.key();
    let mints_match = (parsed_pool.token_mint_0 == deposit_mint
        && parsed_pool.token_mint_1 == secondary_mint)
        || (parsed_pool.token_mint_0 == secondary_mint
            && parsed_pool.token_mint_1 == deposit_mint);
    require!(mints_match, VaultError::InvalidPoolMints);

    let strategy = &mut ctx.accounts.strategy;
    strategy.vault = ctx.accounts.vault.key();
    strategy.clmm_program = ctx.accounts.clmm_program.key();
    strategy.pool_state = ctx.accounts.pool_state.key();
    strategy.amm_config = ctx.accounts.amm_config.key();
    strategy.secondary_mint = secondary_mint;
    strategy.secondary_vault = ctx.accounts.secondary_vault.key();
    strategy.position_nft_mint = Pubkey::default();
    strategy.position_nft_account = Pubkey::default();
    strategy.personal_position = Pubkey::default();
    strategy.tick_lower_index = 0;
    strategy.tick_upper_index = 0;
    strategy.liquidity = 0;
    strategy.active_position = false;
    strategy.bump = ctx.bumps.strategy;
    strategy.secondary_vault_bump = ctx.bumps.secondary_vault;

    Ok(())
}

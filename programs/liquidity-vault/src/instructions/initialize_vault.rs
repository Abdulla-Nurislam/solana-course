use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::state::{VaultState, SHARE_MINT_SEED, VAULT_SEED, VAULT_TOKEN_SEED};

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(mint::token_program = token_program)]
    pub deposit_mint: InterfaceAccount<'info, Mint>,

    #[account(
        init,
        payer = admin,
        space = 8 + VaultState::INIT_SPACE,
        seeds = [VAULT_SEED, deposit_mint.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, VaultState>,

    #[account(
        init,
        payer = admin,
        token::mint = deposit_mint,
        token::authority = vault,
        token::token_program = token_program,
        seeds = [VAULT_TOKEN_SEED, vault.key().as_ref()],
        bump
    )]
    pub token_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        init,
        payer = admin,
        mint::decimals = deposit_mint.decimals,
        mint::authority = vault,
        mint::token_program = token_program,
        seeds = [SHARE_MINT_SEED, vault.key().as_ref()],
        bump
    )]
    pub share_mint: InterfaceAccount<'info, Mint>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    vault.admin = ctx.accounts.admin.key();
    vault.deposit_mint = ctx.accounts.deposit_mint.key();
    vault.share_mint = ctx.accounts.share_mint.key();
    vault.token_vault = ctx.accounts.token_vault.key();
    vault.total_deposited = 0;
    vault.total_shares = 0;
    vault.bump = ctx.bumps.vault;
    vault.token_vault_bump = ctx.bumps.token_vault;
    vault.share_mint_bump = ctx.bumps.share_mint;
    Ok(())
}

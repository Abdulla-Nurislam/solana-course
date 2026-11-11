use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    self, Burn, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{
    error::VaultError,
    math::calculate_tokens_to_withdraw,
    state::{UserCard, VaultState, USER_CARD_SEED, VAULT_SEED},
};

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED, deposit_mint.key().as_ref()],
        bump = vault.bump,
        has_one = deposit_mint @ VaultError::InvalidPoolMints,
        has_one = token_vault @ VaultError::InvalidVaultTokenAccount,
        has_one = share_mint @ VaultError::InvalidVaultTokenAccount
    )]
    pub vault: Account<'info, VaultState>,

    #[account(
        mut,
        seeds = [USER_CARD_SEED, vault.key().as_ref(), user.key().as_ref()],
        bump = user_card.bump,
        constraint = user_card.vault == vault.key() @ VaultError::Unauthorized,
        constraint = user_card.owner == user.key() @ VaultError::Unauthorized
    )]
    pub user_card: Account<'info, UserCard>,

    #[account(mint::token_program = token_program)]
    pub deposit_mint: InterfaceAccount<'info, Mint>,

    #[account(mut, mint::token_program = token_program)]
    pub share_mint: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        token::mint = deposit_mint,
        token::authority = user,
        token::token_program = token_program
    )]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = deposit_mint,
        token::authority = vault,
        token::token_program = token_program
    )]
    pub token_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = share_mint,
        token::authority = user,
        token::token_program = token_program
    )]
    pub user_share_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

pub fn withdraw(ctx: Context<Withdraw>, shares: u64) -> Result<()> {
    require!(shares > 0, VaultError::ZeroShares);
    require!(
        ctx.accounts.user_share_account.amount >= shares,
        VaultError::InsufficientShares
    );

    let vault_balance = ctx.accounts.token_vault.amount;
    let total_shares = ctx.accounts.share_mint.supply;
    let tokens_out = calculate_tokens_to_withdraw(shares, vault_balance, total_shares)?;
    require!(
        vault_balance >= tokens_out,
        VaultError::InsufficientVaultLiquidity
    );

    // 1. Сжигание share-токенов пользователя (CPI с подписью пользователя).
    let burn_ctx = CpiContext::new(
        ctx.accounts.token_program.key(),
        Burn {
            mint: ctx.accounts.share_mint.to_account_info(),
            from: ctx.accounts.user_share_account.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        },
    );
    token_interface::burn(burn_ctx, shares)?;

    // 2. Перевод базовых токенов из хранилища пользователю (CPI с подписью PDA vault).
    let deposit_mint_key = ctx.accounts.deposit_mint.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        VAULT_SEED,
        deposit_mint_key.as_ref(),
        &[ctx.accounts.vault.bump],
    ]];
    let transfer_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        TransferChecked {
            from: ctx.accounts.token_vault.to_account_info(),
            mint: ctx.accounts.deposit_mint.to_account_info(),
            to: ctx.accounts.user_token_account.to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        },
        signer_seeds,
    );
    token_interface::transfer_checked(
        transfer_ctx,
        tokens_out,
        ctx.accounts.deposit_mint.decimals,
    )?;

    // 3. Обновление состояния VaultState и UserCard.
    let vault = &mut ctx.accounts.vault;
    vault.total_shares = vault.total_shares.saturating_sub(shares);
    vault.total_deposited = vault.total_deposited.saturating_sub(tokens_out);

    let user_card = &mut ctx.accounts.user_card;
    user_card.shares_minted = user_card.shares_minted.saturating_sub(shares);
    user_card.deposited_amount = user_card.deposited_amount.saturating_sub(tokens_out);

    Ok(())
}

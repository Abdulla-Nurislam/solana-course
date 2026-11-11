use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{self, Mint, MintTo, TokenAccount, TokenInterface, TransferChecked},
};

use crate::{
    error::VaultError,
    math::calculate_shares_to_mint,
    state::{UserCard, VaultState, USER_CARD_SEED, VAULT_SEED},
};

#[derive(Accounts)]
pub struct Deposit<'info> {
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
        init_if_needed,
        payer = user,
        associated_token::mint = share_mint,
        associated_token::authority = user,
        associated_token::token_program = token_program
    )]
    pub user_share_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    let vault_balance = ctx.accounts.token_vault.amount;
    let total_shares = ctx.accounts.share_mint.supply;
    let shares_to_mint = calculate_shares_to_mint(amount, vault_balance, total_shares)?;

    // 1. Перевод токенов пользователя в хранилище (подписывает пользователь).
    let transfer_ctx = CpiContext::new(
        ctx.accounts.token_program.key(),
        TransferChecked {
            from: ctx.accounts.user_token_account.to_account_info(),
            mint: ctx.accounts.deposit_mint.to_account_info(),
            to: ctx.accounts.token_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        },
    );
    token_interface::transfer_checked(transfer_ctx, amount, ctx.accounts.deposit_mint.decimals)?;

    // 2. Выпуск share-токенов пользователю от имени PDA хранилища.
    let deposit_mint_key = ctx.accounts.deposit_mint.key();
    let signer_seeds: &[&[&[u8]]] = &[&[
        VAULT_SEED,
        deposit_mint_key.as_ref(),
        &[ctx.accounts.vault.bump],
    ]];
    let mint_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.key(),
        MintTo {
            mint: ctx.accounts.share_mint.to_account_info(),
            to: ctx.accounts.user_share_account.to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        },
        signer_seeds,
    );
    token_interface::mint_to(mint_ctx, shares_to_mint)?;

    // 3. Обновление учёта в VaultState и UserCard.
    let vault = &mut ctx.accounts.vault;
    vault.total_deposited = vault
        .total_deposited
        .checked_add(amount)
        .ok_or(VaultError::MathOverflow)?;
    vault.total_shares = vault
        .total_shares
        .checked_add(shares_to_mint)
        .ok_or(VaultError::MathOverflow)?;

    let user_card = &mut ctx.accounts.user_card;
    user_card.deposited_amount = user_card
        .deposited_amount
        .checked_add(amount)
        .ok_or(VaultError::MathOverflow)?;
    user_card.shares_minted = user_card
        .shares_minted
        .checked_add(shares_to_mint)
        .ok_or(VaultError::MathOverflow)?;

    Ok(())
}

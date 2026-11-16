use anchor_lang::prelude::*;

pub const VAULT_SEED: &[u8] = b"vault";
pub const VAULT_TOKEN_SEED: &[u8] = b"vault_token";
pub const SHARE_MINT_SEED: &[u8] = b"share_mint";
pub const USER_CARD_SEED: &[u8] = b"user_card";

/// Главный аккаунт настроек и учёта хранилища (Неделя 8).
#[account]
#[derive(InitSpace)]
pub struct VaultState {
    pub admin: Pubkey,
    pub deposit_mint: Pubkey,
    pub share_mint: Pubkey,
    pub token_vault: Pubkey,
    pub total_deposited: u64,
    pub total_shares: u64,
    pub bump: u8,
    pub token_vault_bump: u8,
    pub share_mint_bump: u8,
}

/// Персональная карточка пользователя в хранилище (Неделя 8).
#[account]
#[derive(InitSpace)]
pub struct UserCard {
    pub vault: Pubkey,
    pub owner: Pubkey,
    pub deposited_amount: u64,
    pub shares_minted: u64,
    pub bump: u8,
}

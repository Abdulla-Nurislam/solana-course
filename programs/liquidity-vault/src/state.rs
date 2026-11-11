use anchor_lang::prelude::*;

pub const VAULT_SEED: &[u8] = b"vault";
pub const VAULT_TOKEN_SEED: &[u8] = b"vault_token";
pub const SHARE_MINT_SEED: &[u8] = b"share_mint";
pub const USER_CARD_SEED: &[u8] = b"user_card";
pub const STRATEGY_SEED: &[u8] = b"strategy";
pub const STRATEGY_VAULT_SEED: &[u8] = b"strategy_vault";

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

#[account]
#[derive(InitSpace)]
pub struct UserCard {
    pub vault: Pubkey,
    pub owner: Pubkey,
    pub deposited_amount: u64,
    pub shares_minted: u64,
    pub bump: u8,
}

/// Стратегия Raydium CLMM и второе хранилище (Неделя 12).
#[account]
#[derive(InitSpace)]
pub struct Strategy {
    pub vault: Pubkey,
    pub clmm_program: Pubkey,
    pub pool_state: Pubkey,
    pub amm_config: Pubkey,
    pub secondary_mint: Pubkey,
    pub secondary_vault: Pubkey,
    pub position_nft_mint: Pubkey,
    pub position_nft_account: Pubkey,
    pub personal_position: Pubkey,
    pub tick_lower_index: i32,
    pub tick_upper_index: i32,
    pub liquidity: u128,
    pub active_position: bool,
    pub bump: u8,
    pub secondary_vault_bump: u8,
}

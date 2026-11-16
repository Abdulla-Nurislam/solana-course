use anchor_lang::prelude::*;

#[error_code]
pub enum VaultError {
    #[msg("Unauthorized caller for this vault operation")]
    Unauthorized,
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Computed share amount must be greater than zero")]
    ZeroShares,
    #[msg("Arithmetic overflow or underflow in vault calculation")]
    MathOverflow,
    #[msg("Insufficient share tokens for withdrawal")]
    InsufficientShares,
    #[msg("Insufficient liquid tokens in vault (close CLMM position first)")]
    InsufficientVaultLiquidity,
    #[msg("Invalid vault or strategy token account")]
    InvalidVaultTokenAccount,
    #[msg("Invalid Raydium CLMM PoolState account")]
    InvalidPoolState,
    #[msg("Pool token mints do not match vault deposit mint and secondary mint")]
    InvalidPoolMints,
    #[msg("Slippage tolerance exceeded: received less than minimum required")]
    SlippageExceeded,
    #[msg("Close the active CLMM position before deposit or withdraw")]
    PositionActive,
}

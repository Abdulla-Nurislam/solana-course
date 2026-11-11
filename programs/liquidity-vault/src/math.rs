use anchor_lang::prelude::*;

use crate::error::VaultError;

/// Расчёт количества share-токенов при депозите (Неделя 9).
pub fn calculate_shares_to_mint(
    deposit_amount: u64,
    vault_balance: u64,
    total_shares: u64,
) -> Result<u64> {
    require!(deposit_amount > 0, VaultError::ZeroAmount);

    if total_shares == 0 || vault_balance == 0 {
        return Ok(deposit_amount);
    }

    let shares_u128 = (deposit_amount as u128)
        .checked_mul(total_shares as u128)
        .ok_or(VaultError::MathOverflow)?
        .checked_div(vault_balance as u128)
        .ok_or(VaultError::MathOverflow)?;

    let shares = u64::try_from(shares_u128).map_err(|_| VaultError::MathOverflow)?;
    require!(shares > 0, VaultError::ZeroShares);

    Ok(shares)
}

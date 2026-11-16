use anchor_lang::prelude::*;

use crate::error::VaultError;

pub const VIRTUAL_SHARES: u128 = 1;
pub const VIRTUAL_ASSETS: u128 = 1;

const Q64: u128 = 1u128 << 64;

/// Расчёт количества share-токенов при депозите (Неделя 9).
pub fn calculate_shares_to_mint(
    deposit_amount: u64,
    vault_value: u64,
    total_shares: u64,
) -> Result<u64> {
    require!(deposit_amount > 0, VaultError::ZeroAmount);

    let shares_u128 = (deposit_amount as u128)
        .checked_mul((total_shares as u128) + VIRTUAL_SHARES)
        .ok_or(VaultError::MathOverflow)?
        .checked_div((vault_value as u128) + VIRTUAL_ASSETS)
        .ok_or(VaultError::MathOverflow)?;

    let shares = u64::try_from(shares_u128).map_err(|_| VaultError::MathOverflow)?;
    require!(shares > 0, VaultError::ZeroShares);

    Ok(shares)
}

/// Оценка остатка второго токена стратегии в единицах `deposit_mint` по спотовой цене пула.
pub fn secondary_value_in_deposit(
    secondary_amount: u64,
    sqrt_price_x64: u128,
    deposit_is_token_0: bool,
) -> Result<u64> {
    if secondary_amount == 0 {
        return Ok(0);
    }
    require!(sqrt_price_x64 > 0, VaultError::InvalidPoolState);

    let amount = secondary_amount as u128;
    let value = if deposit_is_token_0 {
        let step = amount.checked_mul(Q64).ok_or(VaultError::MathOverflow)? / sqrt_price_x64;
        step.checked_mul(Q64).ok_or(VaultError::MathOverflow)? / sqrt_price_x64
    } else {
        let step = amount.checked_mul(sqrt_price_x64).ok_or(VaultError::MathOverflow)? / Q64;
        step.checked_mul(sqrt_price_x64).ok_or(VaultError::MathOverflow)? / Q64
    };

    u64::try_from(value).map_err(|_| error!(VaultError::MathOverflow))
}

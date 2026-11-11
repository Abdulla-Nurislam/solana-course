use anchor_lang::prelude::*;

use crate::error::VaultError;

/// Расчёт количества share-токенов при депозите (Неделя 9).
///
/// Правило:
/// - Первый вкладчик (`total_shares == 0 || vault_balance == 0`) получает `1:1` (`shares = deposit_amount`).
/// - Каждый следующий получает пропорционально в `u128` с округлением вниз в пользу vault:
///   `shares = deposit_amount * total_shares / vault_balance`.
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

/// Расчёт количества базовых токенов к возврату при сжигании share-токенов (Неделя 10).
///
/// Правило:
/// `tokens_out = shares_to_burn * vault_balance / total_shares` (в `u128`, округление вниз).
pub fn calculate_tokens_to_withdraw(
    shares_to_burn: u64,
    vault_balance: u64,
    total_shares: u64,
) -> Result<u64> {
    require!(shares_to_burn > 0, VaultError::ZeroShares);
    require!(
        total_shares > 0 && shares_to_burn <= total_shares,
        VaultError::InsufficientShares
    );

    let tokens_u128 = (shares_to_burn as u128)
        .checked_mul(vault_balance as u128)
        .ok_or(VaultError::MathOverflow)?
        .checked_div(total_shares as u128)
        .ok_or(VaultError::MathOverflow)?;

    let tokens = u64::try_from(tokens_u128).map_err(|_| VaultError::MathOverflow)?;
    require!(tokens > 0, VaultError::ZeroAmount);

    Ok(tokens)
}


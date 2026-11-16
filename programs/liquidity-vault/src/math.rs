use anchor_lang::prelude::*;

use crate::error::VaultError;

/// Виртуальные доли и виртуальные активы (ERC-4626 «virtual offset»).
///
/// Добавляются к знаменателю и числителю курса доли, чтобы атака «первого вкладчика»
/// (депозит 1 микротокена + прямой донат в `token_vault`) перестала быть выгодной:
/// донат делится с виртуальной долей, и атакующий теряет не меньше, чем отнимает у жертвы.
/// При пустом хранилище курс остаётся `1:1`.
pub const VIRTUAL_SHARES: u128 = 1;
pub const VIRTUAL_ASSETS: u128 = 1;

const Q64: u128 = 1u128 << 64;

/// Расчёт количества share-токенов при депозите (Неделя 9).
///
/// `vault_value` — полная стоимость активов хранилища в единицах `deposit_mint`
/// (см. `nav::vault_value`), а не только остаток `token_vault`.
///
/// `shares = deposit_amount * (total_shares + 1) / (vault_value + 1)` в `u128`,
/// округление вниз в пользу хранилища.
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

/// Расчёт количества базовых токенов к возврату при сжигании share-токенов (Неделя 10).
///
/// `tokens_out = shares_to_burn * (vault_value + 1) / (total_shares + 1)` в `u128`,
/// округление вниз. Результат никогда не превышает `vault_value`.
pub fn calculate_tokens_to_withdraw(
    shares_to_burn: u64,
    vault_value: u64,
    total_shares: u64,
) -> Result<u64> {
    require!(shares_to_burn > 0, VaultError::ZeroShares);
    require!(
        total_shares > 0 && shares_to_burn <= total_shares,
        VaultError::InsufficientShares
    );

    let tokens_u128 = (shares_to_burn as u128)
        .checked_mul((vault_value as u128) + VIRTUAL_ASSETS)
        .ok_or(VaultError::MathOverflow)?
        .checked_div((total_shares as u128) + VIRTUAL_SHARES)
        .ok_or(VaultError::MathOverflow)?;

    let tokens = u64::try_from(tokens_u128).map_err(|_| VaultError::MathOverflow)?;
    require!(tokens > 0, VaultError::ZeroAmount);

    Ok(tokens)
}

/// Оценка остатка второго токена стратегии в единицах `deposit_mint` по спотовой цене пула.
///
/// Цена CLMM: `P = (sqrt_price_x64 / 2^64)^2` — сколько `token_1` дают за 1 `token_0`
/// (в сырых единицах).
/// - `deposit_mint == token_0`: второй токен — `token_1`, стоимость `= amount / P`.
/// - `deposit_mint == token_1`: второй токен — `token_0`, стоимость `= amount * P`.
///
/// Обе ветки считаются в два шага через `2^64`, чтобы не переполнить `u128`,
/// округление вниз.
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
        let step = amount
            .checked_mul(Q64)
            .ok_or(VaultError::MathOverflow)?
            / sqrt_price_x64;
        step.checked_mul(Q64).ok_or(VaultError::MathOverflow)? / sqrt_price_x64
    } else {
        let step = amount
            .checked_mul(sqrt_price_x64)
            .ok_or(VaultError::MathOverflow)?
            / Q64;
        step.checked_mul(sqrt_price_x64)
            .ok_or(VaultError::MathOverflow)?
            / Q64
    };

    u64::try_from(value).map_err(|_| error!(VaultError::MathOverflow))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_deposit_is_one_to_one() {
        assert_eq!(calculate_shares_to_mint(1_000, 0, 0).unwrap(), 1_000);
    }

    #[test]
    fn empty_vault_with_outstanding_shares_does_not_mint_one_to_one() {
        // Раньше ветка `vault_balance == 0` выдавала доли 1:1 и размывала старых вкладчиков.
        // Теперь новый вкладчик получает долю пропорционально (total_shares + 1).
        let shares = calculate_shares_to_mint(1_000, 0, 1_000_000).unwrap();
        assert_eq!(shares, 1_000 * 1_000_001);
    }

    #[test]
    fn donation_attack_is_not_profitable() {
        // Атакующий: депозит 1 -> 1 доля, затем донат 1_000_000 в token_vault.
        let attacker_shares = calculate_shares_to_mint(1, 0, 0).unwrap();
        assert_eq!(attacker_shares, 1);
        let vault_value = 1 + 1_000_000;
        // Жертва вносит 2_000_000.
        let victim_shares =
            calculate_shares_to_mint(2_000_000, vault_value, attacker_shares).unwrap();
        let total_shares = attacker_shares + victim_shares;
        let total_value = vault_value + 2_000_000;
        let attacker_out =
            calculate_tokens_to_withdraw(attacker_shares, total_value, total_shares).unwrap();
        // Атакующий вложил 1_000_001 и не может вернуть больше вложенного.
        assert!(attacker_out <= 1_000_001, "attacker_out = {}", attacker_out);
    }

    #[test]
    fn withdraw_never_exceeds_vault_value() {
        for (shares, value, supply) in [(10, 5, 10), (1, 0, 1), (999, 1_000, 999), (7, 3, 100)] {
            if let Ok(out) = calculate_tokens_to_withdraw(shares, value, supply) {
                assert!(out <= value);
            }
        }
    }

    #[test]
    fn secondary_value_at_price_one() {
        assert_eq!(secondary_value_in_deposit(1_000_000, Q64, true).unwrap(), 1_000_000);
        assert_eq!(secondary_value_in_deposit(1_000_000, Q64, false).unwrap(), 1_000_000);
    }

    #[test]
    fn secondary_value_at_price_four() {
        // sqrt(P) = 2 -> P = 4 token_1 за 1 token_0.
        let sqrt = 2 * Q64;
        // deposit = token_0: 4_000 token_1 стоят 1_000 token_0.
        assert_eq!(secondary_value_in_deposit(4_000, sqrt, true).unwrap(), 1_000);
        // deposit = token_1: 1_000 token_0 стоят 4_000 token_1.
        assert_eq!(secondary_value_in_deposit(1_000, sqrt, false).unwrap(), 4_000);
    }
}

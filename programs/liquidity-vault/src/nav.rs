use anchor_lang::prelude::*;

use crate::{
    error::VaultError,
    math::secondary_value_in_deposit,
    raydium::parse_pool_state,
    state::Strategy,
};

/// Смещение поля `amount` в аккаунте SPL Token / Token-2022 (mint 32 + owner 32).
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;

/// Полная стоимость активов хранилища (NAV) в единицах `deposit_mint`.
///
/// - Стратегии нет (PDA `["strategy", vault]` ещё не создан): `NAV = token_vault`.
/// - Стратегия есть: `NAV = token_vault + secondary_vault`, где остаток второго токена
///   переводится в `deposit_mint` по спотовой цене пула Raydium CLMM.
/// - Пока у стратегии открыта CLMM-позиция, часть средств лежит в пуле и честно оценить
///   её без тиковой математики нельзя, поэтому deposit/withdraw отклоняются
///   (`PositionActive`) — сначала нужно закрыть позицию.
///
/// Адрес `strategy_info` проверяется сидами в контексте инструкции, поэтому подменить
/// его нельзя: если стратегия создана, клиент обязан передать её и связанные аккаунты.
pub fn vault_value<'info>(
    vault_key: &Pubkey,
    deposit_mint: &Pubkey,
    token_vault_amount: u64,
    strategy_info: &AccountInfo<'info>,
    secondary_vault_info: &AccountInfo<'info>,
    pool_state_info: &AccountInfo<'info>,
) -> Result<u64> {
    if strategy_info.owner != &crate::ID || strategy_info.data_is_empty() {
        return Ok(token_vault_amount);
    }

    let strategy = {
        let data = strategy_info.try_borrow_data()?;
        Strategy::try_deserialize(&mut &data[..])?
    };
    require_keys_eq!(strategy.vault, *vault_key, VaultError::Unauthorized);
    require!(!strategy.active_position, VaultError::PositionActive);
    require_keys_eq!(
        secondary_vault_info.key(),
        strategy.secondary_vault,
        VaultError::InvalidVaultTokenAccount
    );
    require_keys_eq!(
        pool_state_info.key(),
        strategy.pool_state,
        VaultError::InvalidPoolState
    );

    let secondary_amount = {
        let data = secondary_vault_info.try_borrow_data()?;
        require!(
            data.len() >= TOKEN_ACCOUNT_AMOUNT_OFFSET + 8,
            VaultError::InvalidVaultTokenAccount
        );
        u64::from_le_bytes(
            data[TOKEN_ACCOUNT_AMOUNT_OFFSET..TOKEN_ACCOUNT_AMOUNT_OFFSET + 8]
                .try_into()
                .unwrap(),
        )
    };

    let pool = parse_pool_state(pool_state_info, &strategy.clmm_program)?;
    let deposit_is_token_0 = pool.token_mint_0 == *deposit_mint;
    let secondary_value =
        secondary_value_in_deposit(secondary_amount, pool.sqrt_price_x64, deposit_is_token_0)?;

    token_vault_amount
        .checked_add(secondary_value)
        .ok_or_else(|| error!(VaultError::MathOverflow))
}

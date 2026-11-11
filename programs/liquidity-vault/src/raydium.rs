use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program::invoke_signed,
    },
};

use crate::error::VaultError;

/// Дискриминатор аккаунта `PoolState` программы Raydium CLMM (`sha256("account:PoolState")[..8]`).
pub const POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];

/// Дискриминатор аккаунта `PersonalPositionState` (`sha256("account:PersonalPositionState")[..8]`).
pub const PERSONAL_POSITION_DISCRIMINATOR: [u8; 8] = [70, 111, 150, 126, 230, 15, 25, 117];

/// Дискриминатор инструкции `swap_v2` (`sha256("global:swap_v2")[..8]`).
pub const SWAP_V2_DISCRIMINATOR: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

/// Дискриминатор инструкции `open_position_with_token22_nft` (`sha256("global:open_position_with_token22_nft")[..8]`).
pub const OPEN_POSITION_TOKEN22_DISCRIMINATOR: [u8; 8] = [77, 255, 174, 82, 125, 29, 201, 46];

/// Дискриминатор инструкции `increase_liquidity_v2` (`sha256("global:increase_liquidity_v2")[..8]`).
pub const INCREASE_LIQUIDITY_V2_DISCRIMINATOR: [u8; 8] = [133, 29, 89, 223, 69, 238, 176, 10];

/// Дискриминатор инструкции `decrease_liquidity_v2` (`sha256("global:decrease_liquidity_v2")[..8]`).
pub const DECREASE_LIQUIDITY_V2_DISCRIMINATOR: [u8; 8] = [58, 127, 188, 62, 79, 82, 196, 96];

/// Дискриминатор инструкции `close_position` (`sha256("global:close_position")[..8]`).
pub const CLOSE_POSITION_DISCRIMINATOR: [u8; 8] = [123, 134, 81, 0, 49, 68, 98, 98];

/// Минимальный размер заголовка `PoolState` для чтения ключевых полей (до `tick_current` включительно).
pub const POOL_STATE_HEADER_LEN: usize = 273;

/// Минимальный размер `PersonalPositionState` для чтения ликвидности и комиссий.
pub const PERSONAL_POSITION_MIN_LEN: usize = 145;

#[derive(Clone, Copy, Debug)]
pub struct ParsedPoolState {
    pub amm_config: Pubkey,
    pub token_mint_0: Pubkey,
    pub token_mint_1: Pubkey,
    pub token_vault_0: Pubkey,
    pub token_vault_1: Pubkey,
    pub observation_key: Pubkey,
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct ParsedPersonalPosition {
    pub nft_mint: Pubkey,
    pub pool_id: Pubkey,
    pub tick_lower_index: i32,
    pub tick_upper_index: i32,
    pub liquidity: u128,
    pub token_fees_owed_0: u64,
    pub token_fees_owed_1: u64,
}

pub fn parse_pool_state(pool_info: &AccountInfo, expected_program: &Pubkey) -> Result<ParsedPoolState> {
    require_keys_eq!(
        *pool_info.owner,
        *expected_program,
        VaultError::InvalidPoolState
    );
    let data = pool_info
        .try_borrow_data()
        .map_err(|_| error!(VaultError::InvalidPoolState))?;
    require!(
        data.len() >= POOL_STATE_HEADER_LEN && data[0..8] == POOL_STATE_DISCRIMINATOR,
        VaultError::InvalidPoolState
    );

    let amm_config = Pubkey::new_from_array(data[9..41].try_into().unwrap());
    let token_mint_0 = Pubkey::new_from_array(data[73..105].try_into().unwrap());
    let token_mint_1 = Pubkey::new_from_array(data[105..137].try_into().unwrap());
    let token_vault_0 = Pubkey::new_from_array(data[137..169].try_into().unwrap());
    let token_vault_1 = Pubkey::new_from_array(data[169..201].try_into().unwrap());
    let observation_key = Pubkey::new_from_array(data[201..233].try_into().unwrap());
    let tick_spacing = u16::from_le_bytes(data[235..237].try_into().unwrap());
    let liquidity = u128::from_le_bytes(data[237..253].try_into().unwrap());
    let sqrt_price_x64 = u128::from_le_bytes(data[253..269].try_into().unwrap());
    let tick_current = i32::from_le_bytes(data[269..273].try_into().unwrap());

    Ok(ParsedPoolState {
        amm_config,
        token_mint_0,
        token_mint_1,
        token_vault_0,
        token_vault_1,
        observation_key,
        tick_spacing,
        liquidity,
        sqrt_price_x64,
        tick_current,
    })
}

pub fn parse_personal_position(
    position_info: &AccountInfo,
    expected_program: &Pubkey,
) -> Result<ParsedPersonalPosition> {
    require_keys_eq!(
        *position_info.owner,
        *expected_program,
        VaultError::InvalidPersonalPosition
    );
    let data = position_info
        .try_borrow_data()
        .map_err(|_| error!(VaultError::InvalidPersonalPosition))?;
    require!(
        data.len() >= PERSONAL_POSITION_MIN_LEN && data[0..8] == PERSONAL_POSITION_DISCRIMINATOR,
        VaultError::InvalidPersonalPosition
    );

    let nft_mint = Pubkey::new_from_array(data[9..41].try_into().unwrap());
    let pool_id = Pubkey::new_from_array(data[41..73].try_into().unwrap());
    let tick_lower_index = i32::from_le_bytes(data[73..77].try_into().unwrap());
    let tick_upper_index = i32::from_le_bytes(data[77..81].try_into().unwrap());
    let liquidity = u128::from_le_bytes(data[81..97].try_into().unwrap());
    let token_fees_owed_0 = u64::from_le_bytes(data[129..137].try_into().unwrap());
    let token_fees_owed_1 = u64::from_le_bytes(data[137..145].try_into().unwrap());

    Ok(ParsedPersonalPosition {
        nft_mint,
        pool_id,
        tick_lower_index,
        tick_upper_index,
        liquidity,
        token_fees_owed_0,
        token_fees_owed_1,
    })
}

/// Вызов `swap_v2` в Raydium CLMM через CPI с подписью PDA хранилища.
#[allow(clippy::too_many_arguments)]
pub fn invoke_swap_v2<'info>(
    clmm_program: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    amm_config: &AccountInfo<'info>,
    pool_state: &AccountInfo<'info>,
    input_token_account: &AccountInfo<'info>,
    output_token_account: &AccountInfo<'info>,
    input_vault: &AccountInfo<'info>,
    output_vault: &AccountInfo<'info>,
    observation_state: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    token_program_2022: &AccountInfo<'info>,
    memo_program: &AccountInfo<'info>,
    input_vault_mint: &AccountInfo<'info>,
    output_vault_mint: &AccountInfo<'info>,
    remaining_accounts: &[AccountInfo<'info>],
    amount: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let mut data = Vec::with_capacity(8 + 8 + 8 + 16 + 1);
    data.extend_from_slice(&SWAP_V2_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit_x64.to_le_bytes());
    data.push(u8::from(is_base_input));

    let mut metas = Vec::with_capacity(13 + remaining_accounts.len());
    metas.push(AccountMeta::new_readonly(*payer.key, true));
    metas.push(AccountMeta::new_readonly(*amm_config.key, false));
    metas.push(AccountMeta::new(*pool_state.key, false));
    metas.push(AccountMeta::new(*input_token_account.key, false));
    metas.push(AccountMeta::new(*output_token_account.key, false));
    metas.push(AccountMeta::new(*input_vault.key, false));
    metas.push(AccountMeta::new(*output_vault.key, false));
    metas.push(AccountMeta::new(*observation_state.key, false));
    metas.push(AccountMeta::new_readonly(*token_program.key, false));
    metas.push(AccountMeta::new_readonly(*token_program_2022.key, false));
    metas.push(AccountMeta::new_readonly(*memo_program.key, false));
    metas.push(AccountMeta::new_readonly(*input_vault_mint.key, false));
    metas.push(AccountMeta::new_readonly(*output_vault_mint.key, false));

    let mut infos = Vec::with_capacity(14 + remaining_accounts.len());
    infos.push(payer.clone());
    infos.push(amm_config.clone());
    infos.push(pool_state.clone());
    infos.push(input_token_account.clone());
    infos.push(output_token_account.clone());
    infos.push(input_vault.clone());
    infos.push(output_vault.clone());
    infos.push(observation_state.clone());
    infos.push(token_program.clone());
    infos.push(token_program_2022.clone());
    infos.push(memo_program.clone());
    infos.push(input_vault_mint.clone());
    infos.push(output_vault_mint.clone());

    for acc in remaining_accounts {
        metas.push(if acc.is_writable {
            AccountMeta::new(*acc.key, acc.is_signer)
        } else {
            AccountMeta::new_readonly(*acc.key, acc.is_signer)
        });
        infos.push(acc.clone());
    }

    let ix = Instruction {
        program_id: *clmm_program.key,
        accounts: metas,
        data,
    };
    invoke_signed(&ix, &infos, signer_seeds)?;
    Ok(())
}


use std::str::FromStr;

use anchor_lang::{
    prelude::Pubkey,
    solana_program::instruction::{AccountMeta, Instruction},
    system_program,
};
use anyhow::{anyhow, bail, Result};
use liquidity_vault::raydium::{
    OPEN_POSITION_TOKEN22_DISCRIMINATOR, PERSONAL_POSITION_DISCRIMINATOR,
    PERSONAL_POSITION_MIN_LEN, POOL_STATE_DISCRIMINATOR, POOL_STATE_HEADER_LEN,
    SWAP_V2_DISCRIMINATOR,
};

use crate::rpc::RpcClient;

/// Адрес программы Raydium CLMM в Solana Devnet.
pub const RAYDIUM_CLMM_DEVNET_STR: &str = "devi51mZmdwUJGU9hjN27vEz64Gps7uUefqxg27EAtH";

/// Адрес `AmmConfig` Raydium CLMM в Devnet (`index = 1, tick_spacing = 60, trade_fee = 2500` = 0.25%).
pub const DEFAULT_AMM_CONFIG_DEVNET_STR: &str = "B9H7TR8PSjJT7nuW2tuPkFC63z7drtMZ4LoCtD7PrCN1";

pub const TOKEN_PROGRAM_STR: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022_PROGRAM_STR: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const ATA_PROGRAM_STR: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub const MEMO_PROGRAM_STR: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

/// Дискриминатор инструкции `create_pool` в Raydium CLMM (`sha256("global:create_pool")[..8]`).
pub const CREATE_POOL_DISCRIMINATOR: [u8; 8] = [233, 146, 209, 142, 207, 104, 64, 188];

pub const TICK_ARRAY_SIZE: i32 = 60;
pub const MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
pub const MAX_SQRT_PRICE_X64: u128 = 79_226_673_521_066_979_257_578_248_091;

pub fn raydium_clmm_program_id() -> Pubkey {
    Pubkey::from_str(RAYDIUM_CLMM_DEVNET_STR).unwrap()
}

pub fn default_amm_config_id() -> Pubkey {
    Pubkey::from_str(DEFAULT_AMM_CONFIG_DEVNET_STR).unwrap()
}

pub fn token_program_id() -> Pubkey {
    Pubkey::from_str(TOKEN_PROGRAM_STR).unwrap()
}

pub fn token_2022_program_id() -> Pubkey {
    Pubkey::from_str(TOKEN_2022_PROGRAM_STR).unwrap()
}

pub fn ata_program_id() -> Pubkey {
    Pubkey::from_str(ATA_PROGRAM_STR).unwrap()
}

pub fn memo_program_id() -> Pubkey {
    Pubkey::from_str(MEMO_PROGRAM_STR).unwrap()
}

pub fn rent_sysvar_id() -> Pubkey {
    Pubkey::from_str("SysvarRent111111111111111111111111111111111").unwrap()
}

#[derive(Clone, Debug)]
pub struct ClientPoolState {
    pub address: Pubkey,
    pub amm_config: Pubkey,
    pub creator: Pubkey,
    pub token_mint_0: Pubkey,
    pub token_mint_1: Pubkey,
    pub token_vault_0: Pubkey,
    pub token_vault_1: Pubkey,
    pub observation_key: Pubkey,
    pub mint_decimals_0: u8,
    pub mint_decimals_1: u8,
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
}

#[derive(Clone, Debug)]
pub struct ClientPersonalPosition {
    pub address: Pubkey,
    pub nft_mint: Pubkey,
    pub pool_id: Pubkey,
    pub tick_lower_index: i32,
    pub tick_upper_index: i32,
    pub liquidity: u128,
    pub fee_growth_inside_0_last_x64: u128,
    pub fee_growth_inside_1_last_x64: u128,
    pub token_fees_owed_0: u64,
    pub token_fees_owed_1: u64,
}

/// Каноническое упорядочивание минтов пула Raydium CLMM (`mint_0 < mint_1` по 32-байтному массиву).
pub fn order_mints(mint_a: Pubkey, mint_b: Pubkey) -> Result<(Pubkey, Pubkey)> {
    if mint_a == mint_b {
        bail!("Pool token mints must be distinct");
    }
    if mint_a.to_bytes() < mint_b.to_bytes() {
        Ok((mint_a, mint_b))
    } else {
        Ok((mint_b, mint_a))
    }
}

pub fn derive_pool_state(
    clmm_program: &Pubkey,
    amm_config: &Pubkey,
    mint_0: &Pubkey,
    mint_1: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"pool",
            amm_config.as_ref(),
            mint_0.as_ref(),
            mint_1.as_ref(),
        ],
        clmm_program,
    )
}

pub fn derive_pool_vault(clmm_program: &Pubkey, pool_state: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"pool_vault", pool_state.as_ref(), mint.as_ref()],
        clmm_program,
    )
}

pub fn derive_observation_state(clmm_program: &Pubkey, pool_state: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"observation", pool_state.as_ref()], clmm_program)
}

pub fn derive_tick_array_bitmap_extension(
    clmm_program: &Pubkey,
    pool_state: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"pool_tick_array_bitmap_extension", pool_state.as_ref()],
        clmm_program,
    )
}

pub fn tick_array_start_index(tick_index: i32, tick_spacing: u16) -> i32 {
    let ticks_in_array = TICK_ARRAY_SIZE * (tick_spacing as i32);
    tick_index.div_euclid(ticks_in_array) * ticks_in_array
}

pub fn derive_tick_array(
    clmm_program: &Pubkey,
    pool_state: &Pubkey,
    start_tick_index: i32,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"tick_array",
            pool_state.as_ref(),
            &start_tick_index.to_be_bytes(),
        ],
        clmm_program,
    )
}

pub fn derive_protocol_position(
    clmm_program: &Pubkey,
    pool_state: &Pubkey,
    tick_lower: i32,
    tick_upper: i32,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"position",
            pool_state.as_ref(),
            &tick_lower.to_be_bytes(),
            &tick_upper.to_be_bytes(),
        ],
        clmm_program,
    )
}

pub fn derive_personal_position(clmm_program: &Pubkey, nft_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"position", nft_mint.as_ref()], clmm_program)
}

/// Перевод цены `price` (сколько `token_1` дают за 1 `token_0` с учётом decimals) в `sqrt_price_x64` (`Q64.64`).
pub fn price_to_sqrt_price_x64(price: f64, decimals_0: u8, decimals_1: u8) -> u128 {
    let decimal_diff = (decimals_1 as i32) - (decimals_0 as i32);
    let raw_price = price * 10f64.powi(decimal_diff);
    let sqrt_p = raw_price.sqrt();
    let q64 = (1u128 << 64) as f64;
    (sqrt_p * q64).round() as u128
}

/// Перевод `sqrt_price_x64` (`Q64.64`) в человекочитаемую цену (`token_1` за 1 `token_0`).
pub fn sqrt_price_x64_to_price(sqrt_price_x64: u128, decimals_0: u8, decimals_1: u8) -> f64 {
    let q64 = (1u128 << 64) as f64;
    let sqrt_p = (sqrt_price_x64 as f64) / q64;
    let raw_price = sqrt_p * sqrt_p;
    let decimal_diff = (decimals_0 as i32) - (decimals_1 as i32);
    raw_price * 10f64.powi(decimal_diff)
}

/// Перевод индекса тика в `sqrt_price_x64` (`Q64.64`).
pub fn tick_to_sqrt_price_x64(tick: i32) -> u128 {
    let sqrt_p = 1.0001f64.powf((tick as f64) / 2.0);
    let q64 = (1u128 << 64) as f64;
    (sqrt_p * q64).round() as u128
}

/// Округление индекса тика к сетке `tick_spacing`.
pub fn align_tick(tick: i32, tick_spacing: u16) -> i32 {
    let spacing = tick_spacing as i32;
    tick.div_euclid(spacing) * spacing
}

/// Вычисление концентрированной ликвидности `L` по заданным максимальным объёмам `amount_0_max` и `amount_1_max`.
pub fn compute_liquidity_from_amounts(
    sqrt_price_x64: u128,
    tick_lower: i32,
    tick_upper: i32,
    amount_0_max: u64,
    amount_1_max: u64,
) -> Result<u128> {
    let sqrt_a = tick_to_sqrt_price_x64(tick_lower);
    let sqrt_b = tick_to_sqrt_price_x64(tick_upper);
    if sqrt_a >= sqrt_b {
        bail!("Invalid tick range: lower >= upper");
    }

    let q64 = (1u128 << 64) as f64;
    let sp = (sqrt_price_x64 as f64) / q64;
    let sa = (sqrt_a as f64) / q64;
    let sb = (sqrt_b as f64) / q64;

    // Учитываем запас 2%, чтобы из-за целочисленного округления Raydium в большую сторону не превысить amount_max
    let safe_0 = (amount_0_max as f64) * 0.98;
    let safe_1 = (amount_1_max as f64) * 0.98;

    let liq = if sp <= sa {
        safe_0 * (sa * sb) / (sb - sa)
    } else if sp < sb {
        let l0 = safe_0 * (sp * sb) / (sb - sp);
        let l1 = safe_1 / (sp - sa);
        l0.min(l1)
    } else {
        safe_1 / (sb - sa)
    };

    let l_u128 = liq.floor() as u128;
    if l_u128 == 0 {
        bail!("Computed liquidity is zero; increase token amounts or narrow tick range");
    }
    Ok(l_u128)
}

pub fn fetch_pool_state(rpc: &RpcClient, pool_address: &Pubkey) -> Result<ClientPoolState> {
    let acc = rpc
        .get_account(pool_address)?
        .ok_or_else(|| anyhow!("Raydium PoolState not found at {}", pool_address))?;
    let data = &acc.data;
    if data.len() < POOL_STATE_HEADER_LEN || data[0..8] != POOL_STATE_DISCRIMINATOR {
        bail!("Account {} is not a valid Raydium PoolState", pool_address);
    }

    Ok(ClientPoolState {
        address: *pool_address,
        amm_config: Pubkey::new_from_array(data[9..41].try_into().unwrap()),
        creator: Pubkey::new_from_array(data[41..73].try_into().unwrap()),
        token_mint_0: Pubkey::new_from_array(data[73..105].try_into().unwrap()),
        token_mint_1: Pubkey::new_from_array(data[105..137].try_into().unwrap()),
        token_vault_0: Pubkey::new_from_array(data[137..169].try_into().unwrap()),
        token_vault_1: Pubkey::new_from_array(data[169..201].try_into().unwrap()),
        observation_key: Pubkey::new_from_array(data[201..233].try_into().unwrap()),
        mint_decimals_0: data[233],
        mint_decimals_1: data[234],
        tick_spacing: u16::from_le_bytes(data[235..237].try_into().unwrap()),
        liquidity: u128::from_le_bytes(data[237..253].try_into().unwrap()),
        sqrt_price_x64: u128::from_le_bytes(data[253..269].try_into().unwrap()),
        tick_current: i32::from_le_bytes(data[269..273].try_into().unwrap()),
    })
}

pub fn fetch_personal_position(
    rpc: &RpcClient,
    position_address: &Pubkey,
) -> Result<ClientPersonalPosition> {
    let acc = rpc
        .get_account(position_address)?
        .ok_or_else(|| anyhow!("PersonalPositionState not found at {}", position_address))?;
    let data = &acc.data;
    if data.len() < PERSONAL_POSITION_MIN_LEN || data[0..8] != PERSONAL_POSITION_DISCRIMINATOR {
        bail!(
            "Account {} is not a valid Raydium PersonalPositionState",
            position_address
        );
    }

    Ok(ClientPersonalPosition {
        address: *position_address,
        nft_mint: Pubkey::new_from_array(data[9..41].try_into().unwrap()),
        pool_id: Pubkey::new_from_array(data[41..73].try_into().unwrap()),
        tick_lower_index: i32::from_le_bytes(data[73..77].try_into().unwrap()),
        tick_upper_index: i32::from_le_bytes(data[77..81].try_into().unwrap()),
        liquidity: u128::from_le_bytes(data[81..97].try_into().unwrap()),
        fee_growth_inside_0_last_x64: u128::from_le_bytes(data[97..113].try_into().unwrap()),
        fee_growth_inside_1_last_x64: u128::from_le_bytes(data[113..129].try_into().unwrap()),
        token_fees_owed_0: u64::from_le_bytes(data[129..137].try_into().unwrap()),
        token_fees_owed_1: u64::from_le_bytes(data[137..145].try_into().unwrap()),
    })
}

/// Определение `input_vault` и `output_vault` пула по входному минту.
pub fn pool_vaults_for_input_mint(
    pool: &ClientPoolState,
    input_mint: &Pubkey,
) -> Result<(Pubkey, Pubkey, Pubkey, Pubkey)> {
    if *input_mint == pool.token_mint_0 {
        Ok((
            pool.token_vault_0,
            pool.token_vault_1,
            pool.token_mint_0,
            pool.token_mint_1,
        ))
    } else if *input_mint == pool.token_mint_1 {
        Ok((
            pool.token_vault_1,
            pool.token_vault_0,
            pool.token_mint_1,
            pool.token_mint_0,
        ))
    } else {
        bail!("Mint {} does not belong to pool {}", input_mint, pool.address)
    }
}

/// Формирование `remaining_accounts` (`[bitmap_extension, current_tick_array, adjacent_tick_array]`) для `swap_v2`.
pub fn swap_remaining_accounts(
    rpc: &RpcClient,
    clmm_program: &Pubkey,
    pool: &ClientPoolState,
    zero_for_one: bool,
) -> Result<Vec<AccountMeta>> {
    let (bitmap_ext, _) = derive_tick_array_bitmap_extension(clmm_program, &pool.address);
    let current_start = tick_array_start_index(pool.tick_current, pool.tick_spacing);
    let (current_ta, _) = derive_tick_array(clmm_program, &pool.address, current_start);

    let mut metas = vec![
        AccountMeta::new(bitmap_ext, false),
        AccountMeta::new(current_ta, false),
    ];

    let step = TICK_ARRAY_SIZE * (pool.tick_spacing as i32);
    let next_start = if zero_for_one {
        current_start - step
    } else {
        current_start + step
    };
    let (next_ta, _) = derive_tick_array(clmm_program, &pool.address, next_start);
    if rpc.get_account(&next_ta)?.is_some() {
        metas.push(AccountMeta::new(next_ta, false));
    }

    Ok(metas)
}

/// Сборка инструкции `create_pool` для Raydium CLMM.
pub fn build_create_pool_ix(
    clmm_program: &Pubkey,
    creator: &Pubkey,
    amm_config: &Pubkey,
    mint_0: &Pubkey,
    mint_1: &Pubkey,
    token_program_0: &Pubkey,
    token_program_1: &Pubkey,
    sqrt_price_x64: u128,
    open_time: u64,
) -> (Instruction, Pubkey) {
    let (pool_state, _) = derive_pool_state(clmm_program, amm_config, mint_0, mint_1);
    let (token_vault_0, _) = derive_pool_vault(clmm_program, &pool_state, mint_0);
    let (token_vault_1, _) = derive_pool_vault(clmm_program, &pool_state, mint_1);
    let (observation_state, _) = derive_observation_state(clmm_program, &pool_state);
    let (tick_array_bitmap, _) = derive_tick_array_bitmap_extension(clmm_program, &pool_state);

    let mut data = Vec::with_capacity(32);
    data.extend_from_slice(&CREATE_POOL_DISCRIMINATOR);
    data.extend_from_slice(&sqrt_price_x64.to_le_bytes());
    data.extend_from_slice(&open_time.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*creator, true),
        AccountMeta::new_readonly(*amm_config, false),
        AccountMeta::new(pool_state, false),
        AccountMeta::new_readonly(*mint_0, false),
        AccountMeta::new_readonly(*mint_1, false),
        AccountMeta::new(token_vault_0, false),
        AccountMeta::new(token_vault_1, false),
        AccountMeta::new(observation_state, false),
        AccountMeta::new(tick_array_bitmap, false),
        AccountMeta::new_readonly(*token_program_0, false),
        AccountMeta::new_readonly(*token_program_1, false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(rent_sysvar_id(), false),
    ];

    (
        Instruction {
            program_id: *clmm_program,
            accounts,
            data,
        },
        pool_state,
    )
}

/// Сборка инструкции `open_position_with_token22_nft` напрямую с кошелька (Неделя 11).
#[allow(clippy::too_many_arguments)]
pub fn build_open_position_with_token22_nft_ix(
    clmm_program: &Pubkey,
    payer: &Pubkey,
    position_nft_owner: &Pubkey,
    position_nft_mint: &Pubkey,
    position_nft_account: &Pubkey,
    pool: &ClientPoolState,
    tick_lower_index: i32,
    tick_upper_index: i32,
    liquidity: u128,
    amount_0_max: u64,
    amount_1_max: u64,
    token_account_0: &Pubkey,
    token_account_1: &Pubkey,
) -> (Instruction, Pubkey) {
    let lower_start = tick_array_start_index(tick_lower_index, pool.tick_spacing);
    let upper_start = tick_array_start_index(tick_upper_index, pool.tick_spacing);
    let (protocol_position, _) =
        derive_protocol_position(clmm_program, &pool.address, tick_lower_index, tick_upper_index);
    let (tick_array_lower, _) = derive_tick_array(clmm_program, &pool.address, lower_start);
    let (tick_array_upper, _) = derive_tick_array(clmm_program, &pool.address, upper_start);
    let (personal_position, _) = derive_personal_position(clmm_program, position_nft_mint);

    let mut data = Vec::with_capacity(58);
    data.extend_from_slice(&OPEN_POSITION_TOKEN22_DISCRIMINATOR);
    data.extend_from_slice(&tick_lower_index.to_le_bytes());
    data.extend_from_slice(&tick_upper_index.to_le_bytes());
    data.extend_from_slice(&lower_start.to_le_bytes());
    data.extend_from_slice(&upper_start.to_le_bytes());
    data.extend_from_slice(&liquidity.to_le_bytes());
    data.extend_from_slice(&amount_0_max.to_le_bytes());
    data.extend_from_slice(&amount_1_max.to_le_bytes());
    data.push(0u8); // with_metadata = false
    data.push(0u8); // base_flag = None

    let accounts = vec![
        AccountMeta::new(*payer, true),
        AccountMeta::new_readonly(*position_nft_owner, false),
        AccountMeta::new(*position_nft_mint, true),
        AccountMeta::new(*position_nft_account, false),
        AccountMeta::new(pool.address, false),
        AccountMeta::new(protocol_position, false),
        AccountMeta::new(tick_array_lower, false),
        AccountMeta::new(tick_array_upper, false),
        AccountMeta::new(personal_position, false),
        AccountMeta::new(*token_account_0, false),
        AccountMeta::new(*token_account_1, false),
        AccountMeta::new(pool.token_vault_0, false),
        AccountMeta::new(pool.token_vault_1, false),
        AccountMeta::new_readonly(rent_sysvar_id(), false),
        AccountMeta::new_readonly(system_program::ID, false),
        AccountMeta::new_readonly(token_program_id(), false),
        AccountMeta::new_readonly(ata_program_id(), false),
        AccountMeta::new_readonly(token_2022_program_id(), false),
        AccountMeta::new_readonly(pool.token_mint_0, false),
        AccountMeta::new_readonly(pool.token_mint_1, false),
    ];

    (
        Instruction {
            program_id: *clmm_program,
            accounts,
            data,
        },
        personal_position,
    )
}


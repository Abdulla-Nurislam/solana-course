use anchor_lang::{
    prelude::Pubkey,
    solana_program::instruction::{AccountMeta, Instruction},
    system_program,
};
use anyhow::Result;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::{
    raydium::{
        align_tick, ata_program_id, build_create_pool_ix, build_open_position_with_token22_nft_ix,
        compute_liquidity_from_amounts, default_amm_config_id, fetch_personal_position,
        fetch_pool_state, order_mints, price_to_sqrt_price_x64, raydium_clmm_program_id,
        sqrt_price_x64_to_price, token_2022_program_id, token_program_id,
    },
    rpc::RpcClient,
    vault::{derive_ata, format_ui_amount, parse_ui_amount, read_mint_supply_and_decimals},
};

const MINT_LEN: u64 = 82;
const MINT_RENT_LAMPORTS: u64 = 1_461_600;

/// Инструкция `SystemProgram::CreateAccount` (индекс `0`).
fn system_create_account_ix(
    from: &Pubkey,
    to: &Pubkey,
    lamports: u64,
    space: u64,
    owner: &Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(4 + 8 + 8 + 32);
    data.extend_from_slice(&0u32.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());
    data.extend_from_slice(&space.to_le_bytes());
    data.extend_from_slice(owner.as_ref());

    Instruction {
        program_id: system_program::ID,
        accounts: vec![AccountMeta::new(*from, true), AccountMeta::new(*to, true)],
        data,
    }
}

/// Инструкция `InitializeMint2` SPL Token (тег `20`).
fn spl_initialize_mint2_ix(mint: &Pubkey, decimals: u8, mint_authority: &Pubkey) -> Instruction {
    let mut data = Vec::with_capacity(1 + 1 + 32 + 1);
    data.push(20u8);
    data.push(decimals);
    data.extend_from_slice(mint_authority.as_ref());
    data.push(0u8); // freeze_authority = None

    Instruction {
        program_id: token_program_id(),
        accounts: vec![AccountMeta::new(*mint, false)],
        data,
    }
}

/// Инструкция `CreateIdempotent` Associated Token Account (тег `1`).
pub fn create_ata_idempotent_ix(
    payer: &Pubkey,
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> (Instruction, Pubkey) {
    let ata = derive_ata(wallet, mint, token_program);
    let ix = Instruction {
        program_id: ata_program_id(),
        accounts: vec![
            AccountMeta::new(*payer, true),
            AccountMeta::new(ata, false),
            AccountMeta::new_readonly(*wallet, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(*token_program, false),
        ],
        data: vec![1u8],
    };
    (ix, ata)
}

/// Инструкция `MintTo` SPL Token (тег `7`).
fn spl_mint_to_ix(
    mint: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    amount: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(7u8);
    data.extend_from_slice(&amount.to_le_bytes());

    Instruction {
        program_id: token_program_id(),
        accounts: vec![
            AccountMeta::new(*mint, false),
            AccountMeta::new(*destination, false),
            AccountMeta::new_readonly(*authority, true),
        ],
        data,
    }
}

/// Инструкция `SetComputeUnitLimit` ComputeBudgetProgram (тег `2`).
pub fn compute_unit_limit_ix(units: u32) -> Instruction {
    let program_id = "ComputeBudget111111111111111111111111111111"
        .parse::<Pubkey>()
        .unwrap();
    let mut data = Vec::with_capacity(5);
    data.push(2u8);
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id,
        accounts: vec![],
        data,
    }
}

/// Неделя 11: Создание тестового SPL-токена и выпуск начального баланса на ATA кошелька.
pub fn cmd_create_token(
    rpc: &RpcClient,
    payer: &Keypair,
    decimals: u8,
    ui_supply: &str,
) -> Result<(Pubkey, Pubkey)> {
    let payer_pk = Pubkey::new_from_array(payer.pubkey().to_bytes());
    let mint_kp = Keypair::new();
    let mint_pk = Pubkey::new_from_array(mint_kp.pubkey().to_bytes());
    let raw_supply = parse_ui_amount(ui_supply, decimals)?;

    let create_acc_ix = system_create_account_ix(
        &payer_pk,
        &mint_pk,
        MINT_RENT_LAMPORTS,
        MINT_LEN,
        &token_program_id(),
    );
    let init_mint_ix = spl_initialize_mint2_ix(&mint_pk, decimals, &payer_pk);
    let (create_ata_ix, ata_pk) =
        create_ata_idempotent_ix(&payer_pk, &payer_pk, &mint_pk, &token_program_id());
    let mint_to_ix = spl_mint_to_ix(&mint_pk, &ata_pk, &payer_pk, raw_supply);

    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(
        &[create_acc_ix, init_mint_ix, create_ata_ix, mint_to_ix],
        Some(&payer.pubkey()),
    );
    let tx = Transaction::new(&[payer, &mint_kp], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    println!("=== Test Token Created (Week 11) ===");
    println!(
        "  Mint Address : {} ({})",
        mint_pk,
        rpc.explorer_address_url(&mint_pk)
    );
    println!("  Decimals     : {}", decimals);
    println!("  Wallet ATA   : {}", ata_pk);
    println!(
        "  Minted Supply: {} ({} raw)",
        format_ui_amount(raw_supply, decimals),
        raw_supply
    );
    println!("  Tx Signature : {}", res.signature);
    println!("  Explorer URL : {}", res.explorer_url);

    Ok((mint_pk, ata_pk))
}

/// Неделя 11: Создание пула концентрированной ликвидности Raydium CLMM в Devnet.
pub fn cmd_create_pool(
    rpc: &RpcClient,
    payer: &Keypair,
    mint_a: &Pubkey,
    mint_b: &Pubkey,
    amm_config: Option<Pubkey>,
    initial_price: f64,
) -> Result<Pubkey> {
    let payer_pk = Pubkey::new_from_array(payer.pubkey().to_bytes());
    let clmm_program = raydium_clmm_program_id();
    let config = amm_config.unwrap_or_else(default_amm_config_id);
    let (mint_0, mint_1) = order_mints(*mint_a, *mint_b)?;

    let (_, decimals_0) = read_mint_supply_and_decimals(rpc, &mint_0)?;
    let (_, decimals_1) = read_mint_supply_and_decimals(rpc, &mint_1)?;
    let sqrt_price_x64 = price_to_sqrt_price_x64(initial_price, decimals_0, decimals_1);

    let (create_ix, pool_state) = build_create_pool_ix(
        &clmm_program,
        &payer_pk,
        &config,
        &mint_0,
        &mint_1,
        &token_program_id(),
        &token_program_id(),
        sqrt_price_x64,
        0,
    );

    if rpc.get_account_data(&pool_state)?.is_some() {
        println!(
            "CLMM Pool already exists at {} ({})",
            pool_state,
            rpc.explorer_address_url(&pool_state)
        );
        cmd_show_pool(rpc, &pool_state, None)?;
        return Ok(pool_state);
    }

    let cu_ix = compute_unit_limit_ix(600_000);
    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[cu_ix, create_ix], Some(&payer.pubkey()));
    let tx = Transaction::new(&[payer], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    println!("=== Raydium CLMM Pool Created (Week 11) ===");
    println!("  CLMM Program : {}", clmm_program);
    println!("  AMM Config   : {}", config);
    println!(
        "  Pool State   : {} ({})",
        pool_state,
        rpc.explorer_address_url(&pool_state)
    );
    println!("  Token Mint 0 : {} (decimals={})", mint_0, decimals_0);
    println!("  Token Mint 1 : {} (decimals={})", mint_1, decimals_1);
    println!("  Initial Price: {:.6} token_1 per token_0", initial_price);
    println!("  sqrtPriceX64 : {}", sqrt_price_x64);
    println!("  Tx Signature : {}", res.signature);
    println!("  Explorer URL : {}", res.explorer_url);

    cmd_show_pool(rpc, &pool_state, None)?;
    Ok(pool_state)
}

/// Неделя 11: Открытие начальной позиции в диапазоне вокруг текущей цены с кошелька (`open_position_with_token22_nft`).
pub fn cmd_open_wallet_position(
    rpc: &RpcClient,
    payer: &Keypair,
    pool_address: &Pubkey,
    tick_lower_opt: Option<i32>,
    tick_upper_opt: Option<i32>,
    ui_amount_0_max: &str,
    ui_amount_1_max: &str,
) -> Result<Pubkey> {
    let payer_pk = Pubkey::new_from_array(payer.pubkey().to_bytes());
    let clmm_program = raydium_clmm_program_id();
    let pool = fetch_pool_state(rpc, pool_address)?;

    // По умолчанию берём диапазон ±10 тик-шагов вокруг текущего тика (около ±6% при tick_spacing=60)
    let default_span = (pool.tick_spacing as i32) * 10;
    let tick_lower = tick_lower_opt
        .map(|t| align_tick(t, pool.tick_spacing))
        .unwrap_or_else(|| align_tick(pool.tick_current - default_span, pool.tick_spacing));
    let tick_upper = tick_upper_opt
        .map(|t| align_tick(t, pool.tick_spacing))
        .unwrap_or_else(|| align_tick(pool.tick_current + default_span, pool.tick_spacing));

    let amount_0_max = parse_ui_amount(ui_amount_0_max, pool.mint_decimals_0)?;
    let amount_1_max = parse_ui_amount(ui_amount_1_max, pool.mint_decimals_1)?;
    let liquidity = compute_liquidity_from_amounts(
        pool.sqrt_price_x64,
        tick_lower,
        tick_upper,
        amount_0_max,
        amount_1_max,
    )?;

    let nft_mint_kp = Keypair::new();
    let nft_mint_pk = Pubkey::new_from_array(nft_mint_kp.pubkey().to_bytes());
    let nft_ata = derive_ata(&payer_pk, &nft_mint_pk, &token_2022_program_id());
    let user_ata_0 = derive_ata(&payer_pk, &pool.token_mint_0, &token_program_id());
    let user_ata_1 = derive_ata(&payer_pk, &pool.token_mint_1, &token_program_id());

    let (open_ix, personal_position) = build_open_position_with_token22_nft_ix(
        &clmm_program,
        &payer_pk,
        &payer_pk,
        &nft_mint_pk,
        &nft_ata,
        &pool,
        tick_lower,
        tick_upper,
        liquidity,
        amount_0_max,
        amount_1_max,
        &user_ata_0,
        &user_ata_1,
    );

    let cu_ix = compute_unit_limit_ix(600_000);
    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[cu_ix, open_ix], Some(&payer.pubkey()));
    let tx = Transaction::new(&[payer, &nft_mint_kp], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    println!("=== Wallet CLMM Position Opened (Week 11) ===");
    println!("  Pool State        : {}", pool_address);
    println!(
        "  Position NFT Mint : {} ({})",
        nft_mint_pk,
        rpc.explorer_address_url(&nft_mint_pk)
    );
    println!("  Position NFT ATA  : {}", nft_ata);
    println!(
        "  Personal Position : {} ({})",
        personal_position,
        rpc.explorer_address_url(&personal_position)
    );
    println!("  Tick Range        : [{} .. {}]", tick_lower, tick_upper);
    println!("  Liquidity (L)     : {}", liquidity);
    println!("  Tx Signature      : {}", res.signature);
    println!("  Explorer URL      : {}", res.explorer_url);

    Ok(personal_position)
}

/// Чтение состояния пула Raydium CLMM и (опционально) позиции.
pub fn cmd_show_pool(
    rpc: &RpcClient,
    pool_address: &Pubkey,
    position_address: Option<&Pubkey>,
) -> Result<()> {
    let pool = fetch_pool_state(rpc, pool_address)?;
    let price =
        sqrt_price_x64_to_price(pool.sqrt_price_x64, pool.mint_decimals_0, pool.mint_decimals_1);
    let v0_bal = crate::vault::read_token_balance(rpc, &pool.token_vault_0)?;
    let v1_bal = crate::vault::read_token_balance(rpc, &pool.token_vault_1)?;

    println!("=== Raydium CLMM Pool State ===");
    println!(
        "  Pool Address    : {} ({})",
        pool.address,
        rpc.explorer_address_url(&pool.address)
    );
    println!("  AMM Config      : {}", pool.amm_config);
    println!(
        "  Token Mint 0    : {} (decimals={})",
        pool.token_mint_0, pool.mint_decimals_0
    );
    println!(
        "  Token Mint 1    : {} (decimals={})",
        pool.token_mint_1, pool.mint_decimals_1
    );
    println!(
        "  Pool Vault 0    : {} (balance={})",
        pool.token_vault_0,
        format_ui_amount(v0_bal, pool.mint_decimals_0)
    );
    println!(
        "  Pool Vault 1    : {} (balance={})",
        pool.token_vault_1,
        format_ui_amount(v1_bal, pool.mint_decimals_1)
    );
    println!("  Observation Key : {}", pool.observation_key);
    println!("  Tick Spacing    : {}", pool.tick_spacing);
    println!("  Current Tick    : {}", pool.tick_current);
    println!("  sqrtPriceX64    : {}", pool.sqrt_price_x64);
    println!("  Current Price   : {:.6} token_1 per token_0", price);
    println!("  Active Liquidity: {}", pool.liquidity);

    if let Some(pos_addr) = position_address {
        let pos = fetch_personal_position(rpc, pos_addr)?;
        println!("--- PersonalPositionState ({}) ---", pos.address);
        println!("  NFT Mint        : {}", pos.nft_mint);
        println!(
            "  Tick Range      : [{} .. {}]",
            pos.tick_lower_index, pos.tick_upper_index
        );
        println!("  Liquidity       : {}", pos.liquidity);
        println!("  Fees Owed 0     : {}", pos.token_fees_owed_0);
        println!("  Fees Owed 1     : {}", pos.token_fees_owed_1);
    }

    Ok(())
}

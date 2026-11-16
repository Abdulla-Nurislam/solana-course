use anchor_lang::{
    prelude::Pubkey,
    solana_program::instruction::Instruction,
    system_program, InstructionData, ToAccountMetas,
};
use anyhow::{anyhow, bail, Result};
use liquidity_vault::{accounts, instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::{
    pool::{compute_unit_limit_ix, create_ata_idempotent_ix},
    raydium::{
        align_tick, ata_program_id, build_wallet_swap_v2_ix, compute_liquidity_from_amounts,
        derive_personal_position, derive_protocol_position, derive_tick_array,
        fetch_personal_position, fetch_pool_state, memo_program_id, pool_vaults_for_input_mint,
        raydium_clmm_program_id, rent_sysvar_id, sqrt_price_x64_to_price, swap_remaining_accounts,
        tick_array_start_index, token_2022_program_id, token_program_id,
    },
    rpc::RpcClient,
    strategy::parse_token_account_balance,
    vault::{
        derive_ata, derive_strategy_pda, derive_vault_pda, fetch_strategy, fetch_vault_state,
        format_ui_amount, parse_ui_amount, read_mint_supply_and_decimals, read_token_balance,
    },
};

/// Неделя 13: Открытие концентрированной позиции vault в пуле Raydium CLMM через CPI (`open_position`).
pub fn cmd_open_vault_position(
    rpc: &RpcClient,
    admin: &Keypair,
    deposit_mint: &Pubkey,
    tick_lower_opt: Option<i32>,
    tick_upper_opt: Option<i32>,
    ui_amount_0_max: &str,
    ui_amount_1_max: &str,
) -> Result<String> {
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let vault_state = fetch_vault_state(rpc, &vault)?;
    let (strategy_pda, _) = derive_strategy_pda(&vault);
    let strategy = fetch_strategy(rpc, &strategy_pda)?
        .ok_or_else(|| anyhow!("Strategy not initialized for vault {}", vault))?;
    if strategy.active_position {
        bail!(
            "Vault already has an active position at {}",
            strategy.personal_position
        );
    }

    let pool = fetch_pool_state(rpc, &strategy.pool_state)?;
    let clmm_program = raydium_clmm_program_id();

    let default_span = (pool.tick_spacing as i32) * 5;
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

    let lower_start = tick_array_start_index(tick_lower, pool.tick_spacing);
    let upper_start = tick_array_start_index(tick_upper, pool.tick_spacing);
    let (protocol_position, _) =
        derive_protocol_position(&clmm_program, &pool.address, tick_lower, tick_upper);
    let (tick_array_lower, _) = derive_tick_array(&clmm_program, &pool.address, lower_start);
    let (tick_array_upper, _) = derive_tick_array(&clmm_program, &pool.address, upper_start);

    let nft_mint_kp = Keypair::new();
    let position_nft_mint = Pubkey::new_from_array(nft_mint_kp.pubkey().to_bytes());
    let position_nft_account = derive_ata(&vault, &position_nft_mint, &token_2022_program_id());
    let (personal_position, _) = derive_personal_position(&clmm_program, &position_nft_mint);

    let dep_before = read_token_balance(rpc, &vault_state.token_vault)?;
    let sec_before = read_token_balance(rpc, &strategy.secondary_vault)?;

    let ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::OpenVaultPosition {
            admin: admin_pk,
            vault,
            strategy: strategy_pda,
            position_nft_mint,
            position_nft_account,
            pool_state: pool.address,
            protocol_position,
            tick_array_lower,
            tick_array_upper,
            personal_position,
            token_vault: vault_state.token_vault,
            secondary_vault: strategy.secondary_vault,
            pool_vault_0: pool.token_vault_0,
            pool_vault_1: pool.token_vault_1,
            deposit_mint: *deposit_mint,
            secondary_mint: strategy.secondary_mint,
            clmm_program,
            rent: rent_sysvar_id(),
            system_program: system_program::ID,
            token_program: token_program_id(),
            associated_token_program: ata_program_id(),
            token_program_2022: token_2022_program_id(),
        }
        .to_account_metas(None),
        data: instruction::OpenPosition {
            tick_lower_index: tick_lower,
            tick_upper_index: tick_upper,
            tick_array_lower_start_index: lower_start,
            tick_array_upper_start_index: upper_start,
            liquidity,
            amount_0_max,
            amount_1_max,
        }
        .data(),
    };

    let cu_ix = compute_unit_limit_ix(600_000);
    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[cu_ix, ix], Some(&admin.pubkey()));
    let tx = Transaction::new(&[admin, &nft_mint_kp], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    let dep_after = read_token_balance(rpc, &vault_state.token_vault)?;
    let sec_after = read_token_balance(rpc, &strategy.secondary_vault)?;
    let (_, dep_dec) = read_mint_supply_and_decimals(rpc, deposit_mint)?;
    let (_, sec_dec) = read_mint_supply_and_decimals(rpc, &strategy.secondary_mint)?;

    println!("=== Vault CLMM Position Opened via CPI (Week 13) ===");
    println!("  Pool State        : {}", pool.address);
    println!(
        "  Position NFT Mint : {} ({})",
        position_nft_mint,
        rpc.explorer_address_url(&position_nft_mint)
    );
    println!(
        "  Position NFT ATA  : {} (owner = vault PDA)",
        position_nft_account
    );
    println!(
        "  Personal Position : {} ({})",
        personal_position,
        rpc.explorer_address_url(&personal_position)
    );
    println!("  Tick Range        : [{} .. {}]", tick_lower, tick_upper);
    println!("  Liquidity (L)     : {}", liquidity);
    println!(
        "  Spent Deposit Tok : {}",
        format_ui_amount(dep_before.saturating_sub(dep_after), dep_dec)
    );
    println!(
        "  Spent Second. Tok : {}",
        format_ui_amount(sec_before.saturating_sub(sec_after), sec_dec)
    );
    println!("  Tx Signature      : {}", res.signature);
    println!("  Explorer URL      : {}", res.explorer_url);

    Ok(res.signature)
}

/// Неделя 13: Сбор накопленных комиссий позиции vault в Raydium CLMM через CPI (`collect_fees`).
pub fn cmd_collect_vault_fees(
    rpc: &RpcClient,
    admin: &Keypair,
    deposit_mint: &Pubkey,
) -> Result<String> {
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let vault_state = fetch_vault_state(rpc, &vault)?;
    let (strategy_pda, _) = derive_strategy_pda(&vault);
    let strategy = fetch_strategy(rpc, &strategy_pda)?
        .ok_or_else(|| anyhow!("Strategy not initialized for vault {}", vault))?;
    if !strategy.active_position {
        bail!("Vault does not have an active CLMM position");
    }

    let pool = fetch_pool_state(rpc, &strategy.pool_state)?;
    let clmm_program = raydium_clmm_program_id();
    let lower_start = tick_array_start_index(strategy.tick_lower_index, pool.tick_spacing);
    let upper_start = tick_array_start_index(strategy.tick_upper_index, pool.tick_spacing);
    let (protocol_position, _) = derive_protocol_position(
        &clmm_program,
        &pool.address,
        strategy.tick_lower_index,
        strategy.tick_upper_index,
    );
    let (tick_array_lower, _) = derive_tick_array(&clmm_program, &pool.address, lower_start);
    let (tick_array_upper, _) = derive_tick_array(&clmm_program, &pool.address, upper_start);

    let dep_before = read_token_balance(rpc, &vault_state.token_vault)?;
    let sec_before = read_token_balance(rpc, &strategy.secondary_vault)?;

    let ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::CollectVaultFees {
            admin: admin_pk,
            vault,
            strategy: strategy_pda,
            position_nft_account: strategy.position_nft_account,
            personal_position: strategy.personal_position,
            pool_state: pool.address,
            protocol_position,
            pool_vault_0: pool.token_vault_0,
            pool_vault_1: pool.token_vault_1,
            tick_array_lower,
            tick_array_upper,
            token_vault: vault_state.token_vault,
            secondary_vault: strategy.secondary_vault,
            deposit_mint: *deposit_mint,
            secondary_mint: strategy.secondary_mint,
            clmm_program,
            token_program: token_program_id(),
            token_program_2022: token_2022_program_id(),
            memo_program: memo_program_id(),
        }
        .to_account_metas(None),
        data: instruction::CollectFees {}.data(),
    };

    let cu_ix = compute_unit_limit_ix(400_000);
    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[cu_ix, ix], Some(&admin.pubkey()));
    let tx = Transaction::new(&[admin], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    let dep_after = read_token_balance(rpc, &vault_state.token_vault)?;
    let sec_after = read_token_balance(rpc, &strategy.secondary_vault)?;
    let (_, dep_dec) = read_mint_supply_and_decimals(rpc, deposit_mint)?;
    let (_, sec_dec) = read_mint_supply_and_decimals(rpc, &strategy.secondary_mint)?;

    let fee_dep = dep_after.saturating_sub(dep_before);
    let fee_sec = sec_after.saturating_sub(sec_before);

    println!("=== Vault CLMM Fees Collected via CPI (Week 13) ===");
    println!("  Personal Position  : {}", strategy.personal_position);
    println!(
        "  Collected Deposit  : {} ({} raw)",
        format_ui_amount(fee_dep, dep_dec),
        fee_dep
    );
    println!(
        "  Collected Secondary: {} ({} raw)",
        format_ui_amount(fee_sec, sec_dec),
        fee_sec
    );
    println!("  Tx Signature       : {}", res.signature);
    println!("  Explorer URL       : {}", res.explorer_url);

    Ok(res.signature)
}

/// Неделя 13: Закрытие позиции vault в Raydium CLMM с расчётом минимумов через simulation (`close_position`).
pub fn cmd_close_vault_position(
    rpc: &RpcClient,
    admin: &Keypair,
    deposit_mint: &Pubkey,
    slippage_bps: u64,
) -> Result<String> {
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let vault_state = fetch_vault_state(rpc, &vault)?;
    let (strategy_pda, _) = derive_strategy_pda(&vault);
    let strategy = fetch_strategy(rpc, &strategy_pda)?
        .ok_or_else(|| anyhow!("Strategy not initialized for vault {}", vault))?;
    if !strategy.active_position {
        bail!("Vault does not have an active CLMM position to close");
    }

    let pool = fetch_pool_state(rpc, &strategy.pool_state)?;
    let pos_before = fetch_personal_position(rpc, &strategy.personal_position)?;
    let clmm_program = raydium_clmm_program_id();
    let lower_start = tick_array_start_index(strategy.tick_lower_index, pool.tick_spacing);
    let upper_start = tick_array_start_index(strategy.tick_upper_index, pool.tick_spacing);
    let (protocol_position, _) = derive_protocol_position(
        &clmm_program,
        &pool.address,
        strategy.tick_lower_index,
        strategy.tick_upper_index,
    );
    let (tick_array_lower, _) = derive_tick_array(&clmm_program, &pool.address, lower_start);
    let (tick_array_upper, _) = derive_tick_array(&clmm_program, &pool.address, upper_start);

    let dep_before = read_token_balance(rpc, &vault_state.token_vault)?;
    let sec_before = read_token_balance(rpc, &strategy.secondary_vault)?;

    let build_close_ix = |amount_0_min: u64, amount_1_min: u64| -> Instruction {
        Instruction {
            program_id: liquidity_vault::id(),
            accounts: accounts::CloseVaultPosition {
                admin: admin_pk,
                vault,
                strategy: strategy_pda,
                position_nft_mint: strategy.position_nft_mint,
                position_nft_account: strategy.position_nft_account,
                personal_position: strategy.personal_position,
                pool_state: pool.address,
                protocol_position,
                pool_vault_0: pool.token_vault_0,
                pool_vault_1: pool.token_vault_1,
                tick_array_lower,
                tick_array_upper,
                token_vault: vault_state.token_vault,
                secondary_vault: strategy.secondary_vault,
                deposit_mint: *deposit_mint,
                secondary_mint: strategy.secondary_mint,
                clmm_program,
                token_program: token_program_id(),
                token_program_2022: token_2022_program_id(),
                memo_program: memo_program_id(),
                system_program: system_program::ID,
            }
            .to_account_metas(None),
            data: instruction::ClosePosition {
                amount_0_min,
                amount_1_min,
            }
            .data(),
        }
    };

    // 1. Симулируем закрытие с нулевыми минимумами, чтобы узнать точный возврат обоих токенов
    let cu_ix = compute_unit_limit_ix(500_000);
    let sim_ix = build_close_ix(0, 0);
    let bh = rpc.get_latest_blockhash()?;
    let sim_msg = Message::new(&[cu_ix.clone(), sim_ix], Some(&admin.pubkey()));
    let sim_tx = Transaction::new(&[admin], sim_msg, bh.to_bytes().into());
    let sim = rpc.simulate_with_accounts(
        &sim_tx,
        &[vault_state.token_vault, strategy.secondary_vault],
    )?;
    if let Some(err) = sim.err {
        bail!(
            "Close position quote simulation failed: {}\nLogs:\n{}",
            err,
            sim.logs.join("\n")
        );
    }

    let dep_sim_after = parse_token_account_balance(
        &sim.accounts
            .first()
            .and_then(|a| a.as_ref())
            .ok_or_else(|| anyhow!("Missing post-sim token_vault"))?
            .data,
    )?;
    let sec_sim_after = parse_token_account_balance(
        &sim.accounts
            .get(1)
            .and_then(|a| a.as_ref())
            .ok_or_else(|| anyhow!("Missing post-sim secondary_vault"))?
            .data,
    )?;

    let quoted_dep = dep_sim_after.saturating_sub(dep_before);
    let quoted_sec = sec_sim_after.saturating_sub(sec_before);

    let deposit_is_token_0 = pool.token_mint_0 == *deposit_mint;
    let (quoted_0, quoted_1) = if deposit_is_token_0 {
        (quoted_dep, quoted_sec)
    } else {
        (quoted_sec, quoted_dep)
    };

    let bps = slippage_bps.min(5_000);
    let amount_0_min = ((quoted_0 as u128) * ((10_000 - bps) as u128) / 10_000) as u64;
    let amount_1_min = ((quoted_1 as u128) * ((10_000 - bps) as u128) / 10_000) as u64;

    let close_ix = build_close_ix(amount_0_min, amount_1_min);
    let bh2 = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[cu_ix, close_ix], Some(&admin.pubkey()));
    let tx = Transaction::new(&[admin], msg, bh2.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    let dep_after = read_token_balance(rpc, &vault_state.token_vault)?;
    let sec_after = read_token_balance(rpc, &strategy.secondary_vault)?;
    let (_, dep_dec) = read_mint_supply_and_decimals(rpc, deposit_mint)?;
    let (_, sec_dec) = read_mint_supply_and_decimals(rpc, &strategy.secondary_mint)?;

    let returned_dep = dep_after.saturating_sub(dep_before);
    let returned_sec = sec_after.saturating_sub(sec_before);

    println!("=== Vault CLMM Position Closed via CPI (Week 13) ===");
    println!("  Closed Position    : {}", strategy.personal_position);
    println!("  Removed Liquidity  : {}", pos_before.liquidity);
    println!(
        "  Min Out (0 / 1)    : {} / {} raw (slippage={} bps)",
        amount_0_min, amount_1_min, slippage_bps
    );
    println!(
        "  Returned Deposit   : {} ({} raw)",
        format_ui_amount(returned_dep, dep_dec),
        returned_dep
    );
    println!(
        "  Returned Secondary : {} ({} raw)",
        format_ui_amount(returned_sec, sec_dec),
        returned_sec
    );
    println!("  Tx Signature       : {}", res.signature);
    println!("  Explorer URL       : {}", res.explorer_url);

    Ok(res.signature)
}

/// Неделя 13: Ребалансировка позиции (закрытие старой позиции и открытие новой вокруг текущей цены пула).
#[allow(clippy::too_many_arguments)]
pub fn cmd_rebalance_vault_position(
    rpc: &RpcClient,
    admin: &Keypair,
    deposit_mint: &Pubkey,
    new_tick_lower: Option<i32>,
    new_tick_upper: Option<i32>,
    ui_amount_0_max: &str,
    ui_amount_1_max: &str,
    slippage_bps: u64,
) -> Result<()> {
    let (vault, _) = derive_vault_pda(deposit_mint);
    let (strategy_pda, _) = derive_strategy_pda(&vault);
    let strategy = fetch_strategy(rpc, &strategy_pda)?
        .ok_or_else(|| anyhow!("Strategy not initialized for vault {}", vault))?;

    println!("=== Rebalancing Vault CLMM Position (Week 13) ===");
    if strategy.active_position {
        println!(
            "Step 1/2: Closing previous position [{} .. {}]...",
            strategy.tick_lower_index, strategy.tick_upper_index
        );
        cmd_close_vault_position(rpc, admin, deposit_mint, slippage_bps)?;
    }
    println!("Step 2/2: Opening new position around current pool price...");
    cmd_open_vault_position(
        rpc,
        admin,
        deposit_mint,
        new_tick_lower,
        new_tick_upper,
        ui_amount_0_max,
        ui_amount_1_max,
    )?;
    Ok(())
}

/// Неделя 13: Обмен с кошелька стороннего трейдера напрямую в пуле Raydium CLMM (для начисления комиссий и сдвига цены).
pub fn cmd_trader_swap(
    rpc: &RpcClient,
    trader: &Keypair,
    pool_address: &Pubkey,
    ui_amount_in: &str,
    zero_for_one: bool,
) -> Result<String> {
    let trader_pk = Pubkey::new_from_array(trader.pubkey().to_bytes());
    let clmm_program = raydium_clmm_program_id();
    let pool_before = fetch_pool_state(rpc, pool_address)?;
    let price_before = sqrt_price_x64_to_price(
        pool_before.sqrt_price_x64,
        pool_before.mint_decimals_0,
        pool_before.mint_decimals_1,
    );

    let (input_mint, output_mint, in_dec, out_dec) = if zero_for_one {
        (
            pool_before.token_mint_0,
            pool_before.token_mint_1,
            pool_before.mint_decimals_0,
            pool_before.mint_decimals_1,
        )
    } else {
        (
            pool_before.token_mint_1,
            pool_before.token_mint_0,
            pool_before.mint_decimals_1,
            pool_before.mint_decimals_0,
        )
    };

    let amount_in = parse_ui_amount(ui_amount_in, in_dec)?;
    let (input_vault, output_vault, _, _) =
        pool_vaults_for_input_mint(&pool_before, &input_mint)?;
    let remaining_metas =
        swap_remaining_accounts(rpc, &clmm_program, &pool_before, zero_for_one)?;

    let (create_in_ata_ix, in_ata) =
        create_ata_idempotent_ix(&trader_pk, &trader_pk, &input_mint, &token_program_id());
    let (create_out_ata_ix, out_ata) =
        create_ata_idempotent_ix(&trader_pk, &trader_pk, &output_mint, &token_program_id());

    let out_before = read_token_balance(rpc, &out_ata)?;

    let swap_ix = build_wallet_swap_v2_ix(
        &clmm_program,
        &trader_pk,
        &pool_before,
        &in_ata,
        &out_ata,
        &input_vault,
        &output_vault,
        &input_mint,
        &output_mint,
        remaining_metas,
        amount_in,
        1,
        0,
    );

    let cu_ix = compute_unit_limit_ix(400_000);
    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(
        &[cu_ix, create_in_ata_ix, create_out_ata_ix, swap_ix],
        Some(&trader.pubkey()),
    );
    let tx = Transaction::new(&[trader], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    let out_after = read_token_balance(rpc, &out_ata)?;
    let pool_after = fetch_pool_state(rpc, pool_address)?;
    let price_after = sqrt_price_x64_to_price(
        pool_after.sqrt_price_x64,
        pool_after.mint_decimals_0,
        pool_after.mint_decimals_1,
    );

    println!("=== External Trader Swap in Raydium CLMM (Week 13) ===");
    println!(
        "  Direction      : {}",
        if zero_for_one {
            "token_0 -> token_1"
        } else {
            "token_1 -> token_0"
        }
    );
    println!(
        "  Amount In      : {} ({} raw)",
        format_ui_amount(amount_in, in_dec),
        amount_in
    );
    println!(
        "  Received Out   : {} ({} raw)",
        format_ui_amount(out_after.saturating_sub(out_before), out_dec),
        out_after.saturating_sub(out_before)
    );
    println!(
        "  Pool Tick      : {} -> {}",
        pool_before.tick_current, pool_after.tick_current
    );
    println!(
        "  Pool Price     : {:.6} -> {:.6}",
        price_before, price_after
    );
    println!("  Tx Signature   : {}", res.signature);
    println!("  Explorer URL   : {}", res.explorer_url);

    Ok(res.signature)
}

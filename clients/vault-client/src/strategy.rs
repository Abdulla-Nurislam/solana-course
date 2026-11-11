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
    pool::compute_unit_limit_ix,
    raydium::{
        fetch_pool_state, memo_program_id, pool_vaults_for_input_mint, raydium_clmm_program_id,
        rent_sysvar_id, sqrt_price_x64_to_price, swap_remaining_accounts, token_2022_program_id,
        token_program_id,
    },
    rpc::RpcClient,
    vault::{
        derive_secondary_vault_pda, derive_strategy_pda, derive_vault_pda, fetch_strategy,
        fetch_vault_state, format_ui_amount, parse_ui_amount, read_mint_supply_and_decimals,
        read_token_balance, read_token_balance_bytes,
    },
};

pub use crate::vault::read_token_balance_bytes as parse_token_account_balance;

/// Неделя 12: Инициализация стратегии и второго хранилища (`init_strategy`).
pub fn cmd_init_strategy(
    rpc: &RpcClient,
    admin: &Keypair,
    deposit_mint: &Pubkey,
    secondary_mint: &Pubkey,
    pool_state: &Pubkey,
) -> Result<Pubkey> {
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let (strategy, _) = derive_strategy_pda(&vault);
    let (secondary_vault, _) = derive_secondary_vault_pda(&vault);

    if rpc.get_account(&strategy)?.is_some() {
        println!(
            "Strategy already initialized at {} ({})",
            strategy,
            rpc.explorer_address_url(&strategy)
        );
        println!(
            "  secondary_vault : {} ({})",
            secondary_vault,
            rpc.explorer_address_url(&secondary_vault)
        );
        return Ok(strategy);
    }

    let pool = fetch_pool_state(rpc, pool_state)?;
    let clmm_program = raydium_clmm_program_id();

    let ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::InitStrategy {
            admin: admin_pk,
            vault,
            strategy,
            secondary_mint: *secondary_mint,
            secondary_vault,
            clmm_program,
            amm_config: pool.amm_config,
            pool_state: *pool_state,
            token_program: token_program_id(),
            system_program: system_program::ID,
            rent: rent_sysvar_id(),
        }
        .to_account_metas(None),
        data: instruction::InitStrategy {}.data(),
    };

    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[ix], Some(&admin.pubkey()));
    let tx = Transaction::new(&[admin], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    println!("=== Strategy Initialized (Week 12) ===");
    println!("  VaultState      : {}", vault);
    println!(
        "  Strategy PDA    : {} ({})",
        strategy,
        rpc.explorer_address_url(&strategy)
    );
    println!("  Pool State      : {}", pool_state);
    println!("  AMM Config      : {}", pool.amm_config);
    println!("  Secondary Mint  : {}", secondary_mint);
    println!(
        "  Secondary Vault : {} ({})",
        secondary_vault,
        rpc.explorer_address_url(&secondary_vault)
    );
    println!("  Tx Signature    : {}", res.signature);
    println!("  Explorer URL    : {}", res.explorer_url);

    Ok(strategy)
}

/// Неделя 12: Котировка через simulation и обмен токенов из vault через Raydium CLMM (`swap`).
pub fn cmd_vault_swap(
    rpc: &RpcClient,
    admin: &Keypair,
    deposit_mint: &Pubkey,
    ui_amount_in: &str,
    a_to_b: bool,
    slippage_bps: u64,
    reject: bool,
) -> Result<String> {
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let vault_state = fetch_vault_state(rpc, &vault)?;
    let (strategy_pda, _) = derive_strategy_pda(&vault);
    let strategy = fetch_strategy(rpc, &strategy_pda)?
        .ok_or_else(|| anyhow!("Strategy not initialized for vault {}", vault))?;

    let (_, dep_decimals) = read_mint_supply_and_decimals(rpc, deposit_mint)?;
    let (_, sec_decimals) = read_mint_supply_and_decimals(rpc, &strategy.secondary_mint)?;

    let (input_mint, in_decimals, out_decimals, output_vault_account) = if a_to_b {
        (
            *deposit_mint,
            dep_decimals,
            sec_decimals,
            strategy.secondary_vault,
        )
    } else {
        (
            strategy.secondary_mint,
            sec_decimals,
            dep_decimals,
            vault_state.token_vault,
        )
    };

    let amount_in = parse_ui_amount(ui_amount_in, in_decimals)?;
    let pool_before = fetch_pool_state(rpc, &strategy.pool_state)?;
    let price_before = sqrt_price_x64_to_price(
        pool_before.sqrt_price_x64,
        pool_before.mint_decimals_0,
        pool_before.mint_decimals_1,
    );

    let (input_pool_vault, output_pool_vault, _, _) =
        pool_vaults_for_input_mint(&pool_before, &input_mint)?;
    let zero_for_one = input_mint == pool_before.token_mint_0;
    let clmm_program = raydium_clmm_program_id();
    let remaining_metas =
        swap_remaining_accounts(rpc, &clmm_program, &pool_before, zero_for_one)?;

    let output_before = read_token_balance(rpc, &output_vault_account)?;

    let build_swap_ix = |min_amount_out: u64| -> Instruction {
        let mut metas = accounts::SwapVault {
            admin: admin_pk,
            vault,
            strategy: strategy_pda,
            token_vault: vault_state.token_vault,
            secondary_vault: strategy.secondary_vault,
            deposit_mint: *deposit_mint,
            secondary_mint: strategy.secondary_mint,
            clmm_program,
            amm_config: strategy.amm_config,
            pool_state: strategy.pool_state,
            input_vault: input_pool_vault,
            output_vault: output_pool_vault,
            observation_state: pool_before.observation_key,
            token_program: token_program_id(),
            token_program_2022: token_2022_program_id(),
            memo_program: memo_program_id(),
        }
        .to_account_metas(None);
        metas.extend(remaining_metas.clone());

        Instruction {
            program_id: liquidity_vault::id(),
            accounts: metas,
            data: instruction::Swap {
                amount_in,
                min_amount_out,
                sqrt_price_limit_x64: 0,
                a_to_b,
            }
            .data(),
        }
    };

    // 1. Симуляция обмена с `min_amount_out = 1` для получения точной котировки пула
    let cu_ix = compute_unit_limit_ix(400_000);
    let quote_ix = build_swap_ix(1);
    let bh = rpc.get_latest_blockhash()?;
    let quote_msg = Message::new(&[cu_ix.clone(), quote_ix], Some(&admin.pubkey()));
    let quote_tx = Transaction::new(&[admin], quote_msg, bh.to_bytes().into());
    let sim = rpc.simulate_with_accounts(&quote_tx, &[output_vault_account])?;
    if let Some(err) = sim.err {
        bail!(
            "Swap quote simulation failed: {}\nLogs:\n{}",
            err,
            sim.logs.join("\n")
        );
    }

    let post_acc = sim
        .accounts
        .first()
        .and_then(|a| a.as_ref())
        .ok_or_else(|| anyhow!("Missing post-simulation output token account"))?;
    let output_after_sim = read_token_balance_bytes(&post_acc.data)?;
    let quoted_out = output_after_sim.saturating_sub(output_before);
    if quoted_out == 0 {
        bail!("Quoted swap output is 0; check pool liquidity");
    }

    let min_amount_out = if reject {
        // Заведомо недостижимый минимум для демонстрации отказа SlippageExceeded в devnet
        quoted_out.saturating_add(1_000_000)
    } else {
        let bps = slippage_bps.min(5_000);
        ((quoted_out as u128) * ((10_000 - bps) as u128) / 10_000) as u64
    }
    .max(1);

    let swap_ix = build_swap_ix(min_amount_out);
    let bh2 = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[cu_ix, swap_ix], Some(&admin.pubkey()));
    let tx = Transaction::new(&[admin], msg, bh2.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, reject)?;

    if reject {
        println!("=== Vault Swap Rejected by Slippage Protection (Week 12) ===");
        println!(
            "  Direction        : {}",
            if a_to_b {
                "deposit_mint -> secondary_mint"
            } else {
                "secondary_mint -> deposit_mint"
            }
        );
        println!(
            "  Amount In        : {} ({} raw)",
            format_ui_amount(amount_in, in_decimals),
            amount_in
        );
        println!(
            "  Quoted Out       : {} ({} raw)",
            format_ui_amount(quoted_out, out_decimals),
            quoted_out
        );
        println!(
            "  Required Min Out : {} ({} raw) [intentionally above quote]",
            format_ui_amount(min_amount_out, out_decimals),
            min_amount_out
        );
        println!("  On-chain Error   : {:?}", res.err);
        println!("  Tx Signature     : {}", res.signature);
        println!("  Explorer URL     : {}", res.explorer_url);
        return Ok(res.signature);
    }

    let output_after = read_token_balance(rpc, &output_vault_account)?;
    let actual_received = output_after.saturating_sub(output_before);
    let pool_after = fetch_pool_state(rpc, &strategy.pool_state)?;
    let price_after = sqrt_price_x64_to_price(
        pool_after.sqrt_price_x64,
        pool_after.mint_decimals_0,
        pool_after.mint_decimals_1,
    );

    println!("=== Vault Swap Completed via Raydium CLMM CPI (Week 12) ===");
    println!(
        "  Direction        : {}",
        if a_to_b {
            "deposit_mint -> secondary_mint"
        } else {
            "secondary_mint -> deposit_mint"
        }
    );
    println!(
        "  Amount In        : {} ({} raw)",
        format_ui_amount(amount_in, in_decimals),
        amount_in
    );
    println!(
        "  Quoted Out       : {} ({} raw)",
        format_ui_amount(quoted_out, out_decimals),
        quoted_out
    );
    println!(
        "  Min Amount Out   : {} ({} raw, slippage={} bps)",
        format_ui_amount(min_amount_out, out_decimals),
        min_amount_out,
        slippage_bps
    );
    println!(
        "  Actual Received  : {} ({} raw)",
        format_ui_amount(actual_received, out_decimals),
        actual_received
    );
    println!(
        "  Pool Tick        : {} -> {}",
        pool_before.tick_current, pool_after.tick_current
    );
    println!(
        "  Pool Price       : {:.6} -> {:.6} token_1/token_0",
        price_before, price_after
    );
    println!("  Tx Signature     : {}", res.signature);
    println!("  Explorer URL     : {}", res.explorer_url);

    Ok(res.signature)
}

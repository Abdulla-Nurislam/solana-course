use anchor_lang::{
    prelude::Pubkey,
    solana_program::instruction::Instruction,
    system_program, AccountDeserialize, InstructionData, ToAccountMetas,
};
use anyhow::{anyhow, bail, Result};
use liquidity_vault::{
    accounts, instruction,
    math::{calculate_shares_to_mint, calculate_tokens_to_withdraw},
    state::{
        Strategy, UserCard, VaultState, SHARE_MINT_SEED, STRATEGY_SEED, STRATEGY_VAULT_SEED,
        USER_CARD_SEED, VAULT_SEED, VAULT_TOKEN_SEED,
    },
};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::{
    raydium::{ata_program_id, rent_sysvar_id, token_program_id},
    rpc::RpcClient,
};

pub fn derive_vault_pda(deposit_mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[VAULT_SEED, deposit_mint.as_ref()], &liquidity_vault::id())
}

pub fn derive_token_vault_pda(vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[VAULT_TOKEN_SEED, vault.as_ref()], &liquidity_vault::id())
}

pub fn derive_share_mint_pda(vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SHARE_MINT_SEED, vault.as_ref()], &liquidity_vault::id())
}

pub fn derive_user_card_pda(vault: &Pubkey, user: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[USER_CARD_SEED, vault.as_ref(), user.as_ref()],
        &liquidity_vault::id(),
    )
}

pub fn derive_strategy_pda(vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[STRATEGY_SEED, vault.as_ref()], &liquidity_vault::id())
}

pub fn derive_secondary_vault_pda(vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[STRATEGY_VAULT_SEED, vault.as_ref()],
        &liquidity_vault::id(),
    )
}

pub fn derive_ata(wallet: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ata_program_id(),
    )
    .0
}

/// Разбор десятичной строки в атомарные единицы (`u64`) без использования `f64`.
pub fn parse_ui_amount(input: &str, decimals: u8) -> Result<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("Empty amount string");
    }
    let parts: Vec<&str> = trimmed.split('.').collect();
    if parts.len() > 2 {
        bail!("Invalid decimal number: {}", input);
    }

    let whole: u128 = if parts[0].is_empty() {
        0
    } else {
        parts[0].parse()?
    };

    let scale = 10u128.pow(decimals as u32);
    let frac: u128 = if parts.len() == 2 {
        let frac_str = parts[1];
        if frac_str.len() > decimals as usize {
            bail!(
                "Too many decimal places in '{}' (max {})",
                input,
                decimals
            );
        }
        let padded = format!("{:0<width$}", frac_str, width = decimals as usize);
        if padded.is_empty() {
            0
        } else {
            padded.parse()?
        }
    } else {
        0
    };

    let raw = whole
        .checked_mul(scale)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(|| anyhow!("Amount overflow: {}", input))?;

    u64::try_from(raw).map_err(|_| anyhow!("Amount exceeds u64: {}", input))
}

/// Форматирование атомарного количества токенов в строку с десятичной точкой без `f64`.
pub fn format_ui_amount(raw: u64, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let scale = 10u64.pow(decimals as u32);
    let whole = raw / scale;
    let frac = raw % scale;
    let frac_str = format!("{:0width$}", frac, width = decimals as usize);
    let trimmed_frac = frac_str.trim_end_matches('0');
    if trimmed_frac.is_empty() {
        format!("{}.0", whole)
    } else {
        format!("{}.{}", whole, trimmed_frac)
    }
}

pub fn read_mint_supply_and_decimals(rpc: &RpcClient, mint: &Pubkey) -> Result<(u64, u8)> {
    let acc = rpc
        .get_account(mint)?
        .ok_or_else(|| anyhow!("Mint account not found: {}", mint))?;
    if acc.data.len() < 82 {
        bail!("Invalid SPL Mint data length for {}", mint);
    }
    let supply = u64::from_le_bytes(acc.data[36..44].try_into().unwrap());
    let decimals = acc.data[44];
    Ok((supply, decimals))
}

pub fn read_token_balance_bytes(data: &[u8]) -> Result<u64> {
    if data.len() < 72 {
        bail!("Invalid SPL TokenAccount data length: {}", data.len());
    }
    Ok(u64::from_le_bytes(data[64..72].try_into().unwrap()))
}

pub fn read_token_balance(rpc: &RpcClient, token_account: &Pubkey) -> Result<u64> {
    match rpc.get_account(token_account)? {
        Some(acc) => read_token_balance_bytes(&acc.data),
        None => Ok(0),
    }
}

pub fn fetch_vault_state(rpc: &RpcClient, vault: &Pubkey) -> Result<VaultState> {
    let acc = rpc
        .get_account(vault)?
        .ok_or_else(|| anyhow!("VaultState not initialized at {}", vault))?;
    let mut slice: &[u8] = &acc.data;
    VaultState::try_deserialize(&mut slice).map_err(|e| anyhow!("Failed to decode VaultState: {}", e))
}

pub fn fetch_user_card(rpc: &RpcClient, card: &Pubkey) -> Result<Option<UserCard>> {
    match rpc.get_account(card)? {
        Some(acc) => {
            let mut slice: &[u8] = &acc.data;
            let parsed = UserCard::try_deserialize(&mut slice)
                .map_err(|e| anyhow!("Failed to decode UserCard: {}", e))?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

pub fn fetch_strategy(rpc: &RpcClient, strategy: &Pubkey) -> Result<Option<Strategy>> {
    match rpc.get_account(strategy)? {
        Some(acc) => {
            let mut slice: &[u8] = &acc.data;
            let parsed = Strategy::try_deserialize(&mut slice)
                .map_err(|e| anyhow!("Failed to decode Strategy: {}", e))?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

/// Неделя 8: Создание хранилища (`initialize_vault`).
pub fn cmd_init_vault(rpc: &RpcClient, admin: &Keypair, deposit_mint: &Pubkey) -> Result<()> {
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let (token_vault, _) = derive_token_vault_pda(&vault);
    let (share_mint, _) = derive_share_mint_pda(&vault);

    if rpc.get_account(&vault)?.is_some() {
        println!("Vault already initialized at {}", vault);
        println!("  VaultState  : {} ({})", vault, rpc.explorer_address_url(&vault));
        println!(
            "  token_vault : {} ({})",
            token_vault,
            rpc.explorer_address_url(&token_vault)
        );
        println!(
            "  share_mint  : {} ({})",
            share_mint,
            rpc.explorer_address_url(&share_mint)
        );
        return Ok(());
    }

    let ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::InitializeVault {
            admin: admin_pk,
            deposit_mint: *deposit_mint,
            vault,
            token_vault,
            share_mint,
            token_program: token_program_id(),
            system_program: system_program::ID,
            rent: rent_sysvar_id(),
        }
        .to_account_metas(None),
        data: instruction::InitializeVault {}.data(),
    };

    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[ix], Some(&admin.pubkey()));
    let tx = Transaction::new(&[admin], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    println!("=== Vault Initialized (Week 8) ===");
    println!("  Program ID   : {}", liquidity_vault::id());
    println!("  Admin        : {}", admin_pk);
    println!("  Deposit Mint : {}", deposit_mint);
    println!(
        "  VaultState   : {} ({})",
        vault,
        rpc.explorer_address_url(&vault)
    );
    println!(
        "  Token Vault  : {} ({})",
        token_vault,
        rpc.explorer_address_url(&token_vault)
    );
    println!(
        "  Share Mint   : {} ({})",
        share_mint,
        rpc.explorer_address_url(&share_mint)
    );
    println!("  Tx Signature : {}", res.signature);
    println!("  Explorer URL : {}", res.explorer_url);
    Ok(())
}

/// Неделя 8: Создание карточки пользователя (`create_user_card`).
pub fn cmd_create_card(rpc: &RpcClient, user: &Keypair, deposit_mint: &Pubkey) -> Result<()> {
    let user_pk = Pubkey::new_from_array(user.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let (user_card, _) = derive_user_card_pda(&vault, &user_pk);

    if rpc.get_account(&user_card)?.is_some() {
        println!(
            "UserCard already exists at {} ({})",
            user_card,
            rpc.explorer_address_url(&user_card)
        );
        return Ok(());
    }

    let ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::CreateUserCard {
            user: user_pk,
            vault,
            user_card,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: instruction::CreateUserCard {}.data(),
    };

    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[ix], Some(&user.pubkey()));
    let tx = Transaction::new(&[user], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    println!("=== UserCard Created (Week 8) ===");
    println!("  User         : {}", user_pk);
    println!("  VaultState   : {}", vault);
    println!(
        "  UserCard PDA : {} ({})",
        user_card,
        rpc.explorer_address_url(&user_card)
    );
    println!("  Tx Signature : {}", res.signature);
    println!("  Explorer URL : {}", res.explorer_url);
    Ok(())
}

/// Неделя 8–13: Чтение полного состояния хранилища, карточки пользователя и стратегии.
pub fn cmd_show_vault(rpc: &RpcClient, user: &Keypair, deposit_mint: &Pubkey) -> Result<()> {
    let user_pk = Pubkey::new_from_array(user.pubkey().to_bytes());
    let (vault_pda, _) = derive_vault_pda(deposit_mint);
    let vault_state = fetch_vault_state(rpc, &vault_pda)?;
    let (_, decimals) = read_mint_supply_and_decimals(rpc, deposit_mint)?;
    let (share_supply, share_decimals) =
        read_mint_supply_and_decimals(rpc, &vault_state.share_mint)?;
    let vault_token_balance = read_token_balance(rpc, &vault_state.token_vault)?;

    let (user_card_pda, _) = derive_user_card_pda(&vault_pda, &user_pk);
    let user_card = fetch_user_card(rpc, &user_card_pda)?;

    let user_token_ata = derive_ata(&user_pk, deposit_mint, &token_program_id());
    let user_share_ata = derive_ata(&user_pk, &vault_state.share_mint, &token_program_id());
    let user_token_bal = read_token_balance(rpc, &user_token_ata)?;
    let user_share_bal = read_token_balance(rpc, &user_share_ata)?;

    println!("=== Liquidity Vault Status ===");
    println!("  Cluster          : {}", rpc.cluster);
    println!("  Program ID       : {}", liquidity_vault::id());
    println!(
        "  VaultState PDA   : {} ({})",
        vault_pda,
        rpc.explorer_address_url(&vault_pda)
    );
    println!("  Admin            : {}", vault_state.admin);
    println!("  Deposit Mint     : {} (decimals={})", deposit_mint, decimals);
    println!(
        "  Token Vault PDA  : {} (balance={} [{} raw])",
        vault_state.token_vault,
        format_ui_amount(vault_token_balance, decimals),
        vault_token_balance
    );
    println!(
        "  Share Mint PDA   : {} (supply={} [{} raw])",
        vault_state.share_mint,
        format_ui_amount(share_supply, share_decimals),
        share_supply
    );
    println!(
        "  Total Deposited  : {} ({} raw)",
        format_ui_amount(vault_state.total_deposited, decimals),
        vault_state.total_deposited
    );
    println!(
        "  Total Shares     : {} ({} raw)",
        format_ui_amount(vault_state.total_shares, share_decimals),
        vault_state.total_shares
    );

    println!("--- Wallet & UserCard ({}) ---", user_pk);
    println!(
        "  User Token ATA   : {} (balance={})",
        user_token_ata,
        format_ui_amount(user_token_bal, decimals)
    );
    println!(
        "  User Share ATA   : {} (balance={})",
        user_share_ata,
        format_ui_amount(user_share_bal, share_decimals)
    );
    if let Some(card) = user_card {
        println!(
            "  UserCard PDA     : {} (deposited={}, shares={})",
            user_card_pda,
            format_ui_amount(card.deposited_amount, decimals),
            format_ui_amount(card.shares_minted, share_decimals)
        );
    } else {
        println!("  UserCard PDA     : {} (not created yet)", user_card_pda);
    }

    let (strategy_pda, _) = derive_strategy_pda(&vault_pda);
    if let Some(strat) = fetch_strategy(rpc, &strategy_pda)? {
        let (_, sec_decimals) = read_mint_supply_and_decimals(rpc, &strat.secondary_mint)?;
        let sec_bal = read_token_balance(rpc, &strat.secondary_vault)?;
        println!("--- Raydium CLMM Strategy ({}) ---", strategy_pda);
        println!("  Pool State       : {}", strat.pool_state);
        println!("  AMM Config       : {}", strat.amm_config);
        println!("  Secondary Mint   : {}", strat.secondary_mint);
        println!(
            "  Secondary Vault  : {} (balance={} [{} raw])",
            strat.secondary_vault,
            format_ui_amount(sec_bal, sec_decimals),
            sec_bal
        );
        println!("  Active Position  : {}", strat.active_position);
        if strat.active_position {
            println!("  Position NFT Mint: {}", strat.position_nft_mint);
            println!("  Personal Position: {}", strat.personal_position);
            println!(
                "  Tick Range       : [{} .. {}]",
                strat.tick_lower_index, strat.tick_upper_index
            );
            println!("  Position Liq (L) : {}", strat.liquidity);
        }
    }

    Ok(())
}

/// Неделя 9: Депозит токенов в vault с выпуском share-токенов по доле (`deposit`).
pub fn cmd_deposit(
    rpc: &RpcClient,
    user: &Keypair,
    deposit_mint: &Pubkey,
    ui_amount: &str,
) -> Result<()> {
    let user_pk = Pubkey::new_from_array(user.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let vault_state = fetch_vault_state(rpc, &vault)?;
    let (_, decimals) = read_mint_supply_and_decimals(rpc, deposit_mint)?;
    let amount = parse_ui_amount(ui_amount, decimals)?;

    let (user_card, _) = derive_user_card_pda(&vault, &user_pk);
    if rpc.get_account(&user_card)?.is_none() {
        println!("UserCard not found, creating automatically...");
        cmd_create_card(rpc, user, deposit_mint)?;
    }

    let user_token_account = derive_ata(&user_pk, deposit_mint, &token_program_id());
    let user_share_account = derive_ata(&user_pk, &vault_state.share_mint, &token_program_id());

    let user_token_before = read_token_balance(rpc, &user_token_account)?;
    let user_shares_before = read_token_balance(rpc, &user_share_account)?;
    let vault_balance_before = read_token_balance(rpc, &vault_state.token_vault)?;
    let (share_supply_before, _) = read_mint_supply_and_decimals(rpc, &vault_state.share_mint)?;

    let expected_shares =
        calculate_shares_to_mint(amount, vault_balance_before, share_supply_before)
            .map_err(|e| anyhow!("Share calculation failed: {}", e))?;

    let ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::Deposit {
            user: user_pk,
            vault,
            user_card,
            deposit_mint: *deposit_mint,
            share_mint: vault_state.share_mint,
            user_token_account,
            token_vault: vault_state.token_vault,
            user_share_account,
            token_program: token_program_id(),
            associated_token_program: ata_program_id(),
            system_program: system_program::ID,
        }
        .to_account_metas(None),
        data: instruction::Deposit { amount }.data(),
    };

    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[ix], Some(&user.pubkey()));
    let tx = Transaction::new(&[user], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    let user_token_after = read_token_balance(rpc, &user_token_account)?;
    let user_shares_after = read_token_balance(rpc, &user_share_account)?;
    let minted_shares = user_shares_after.saturating_sub(user_shares_before);
    if minted_shares != expected_shares {
        bail!(
            "Share mint mismatch: expected {}, got {}",
            expected_shares,
            minted_shares
        );
    }

    println!("=== Deposit Completed (Week 9) ===");
    println!(
        "  Deposited Amount : {} ({} raw)",
        format_ui_amount(amount, decimals),
        amount
    );
    println!(
        "  Minted Shares    : {} ({} raw)",
        format_ui_amount(minted_shares, decimals),
        minted_shares
    );
    println!(
        "  Wallet Token Bal : {} -> {}",
        format_ui_amount(user_token_before, decimals),
        format_ui_amount(user_token_after, decimals)
    );
    println!(
        "  Wallet Share Bal : {} -> {}",
        format_ui_amount(user_shares_before, decimals),
        format_ui_amount(user_shares_after, decimals)
    );
    println!("  Tx Signature     : {}", res.signature);
    println!("  Explorer URL     : {}", res.explorer_url);
    Ok(())
}

/// Неделя 10: Вывод токенов из vault со сжиганием share-токенов (`withdraw`).
pub fn cmd_withdraw(
    rpc: &RpcClient,
    user: &Keypair,
    deposit_mint: &Pubkey,
    ui_shares: &str,
) -> Result<()> {
    let user_pk = Pubkey::new_from_array(user.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let vault_state = fetch_vault_state(rpc, &vault)?;
    let (_, decimals) = read_mint_supply_and_decimals(rpc, deposit_mint)?;
    let shares = parse_ui_amount(ui_shares, decimals)?;

    let (user_card, _) = derive_user_card_pda(&vault, &user_pk);
    let user_token_account = derive_ata(&user_pk, deposit_mint, &token_program_id());
    let user_share_account = derive_ata(&user_pk, &vault_state.share_mint, &token_program_id());

    let user_token_before = read_token_balance(rpc, &user_token_account)?;
    let user_shares_before = read_token_balance(rpc, &user_share_account)?;
    let vault_balance_before = read_token_balance(rpc, &vault_state.token_vault)?;
    let (share_supply_before, _) = read_mint_supply_and_decimals(rpc, &vault_state.share_mint)?;

    let expected_tokens_out =
        calculate_tokens_to_withdraw(shares, vault_balance_before, share_supply_before)
            .map_err(|e| anyhow!("Withdraw calculation failed: {}", e))?;

    let ix = Instruction {
        program_id: liquidity_vault::id(),
        accounts: accounts::Withdraw {
            user: user_pk,
            vault,
            user_card,
            deposit_mint: *deposit_mint,
            share_mint: vault_state.share_mint,
            user_token_account,
            token_vault: vault_state.token_vault,
            user_share_account,
            token_program: token_program_id(),
        }
        .to_account_metas(None),
        data: instruction::Withdraw { shares }.data(),
    };

    let bh = rpc.get_latest_blockhash()?;
    let msg = Message::new(&[ix], Some(&user.pubkey()));
    let tx = Transaction::new(&[user], msg, bh.to_bytes().into());
    let res = rpc.send_and_confirm(&tx, false)?;

    let user_token_after = read_token_balance(rpc, &user_token_account)?;
    let user_shares_after = read_token_balance(rpc, &user_share_account)?;
    let returned_tokens = user_token_after.saturating_sub(user_token_before);
    if returned_tokens != expected_tokens_out {
        bail!(
            "Returned token mismatch: expected {}, got {}",
            expected_tokens_out,
            returned_tokens
        );
    }

    println!("=== Withdraw Completed (Week 10) ===");
    println!(
        "  Burned Shares    : {} ({} raw)",
        format_ui_amount(shares, decimals),
        shares
    );
    println!(
        "  Returned Tokens  : {} ({} raw)",
        format_ui_amount(returned_tokens, decimals),
        returned_tokens
    );
    println!(
        "  Wallet Token Bal : {} -> {}",
        format_ui_amount(user_token_before, decimals),
        format_ui_amount(user_token_after, decimals)
    );
    println!(
        "  Wallet Share Bal : {} -> {}",
        format_ui_amount(user_shares_before, decimals),
        format_ui_amount(user_shares_after, decimals)
    );
    println!("  Tx Signature     : {}", res.signature);
    println!("  Explorer URL     : {}", res.explorer_url);
    Ok(())
}

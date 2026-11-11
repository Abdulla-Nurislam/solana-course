use std::str::FromStr;

use anchor_lang::{
    prelude::Pubkey,
    solana_program::instruction::Instruction,
    system_program, AccountDeserialize, InstructionData, ToAccountMetas,
};
use anyhow::{anyhow, bail, Result};
use liquidity_vault::{
    accounts, instruction,
    math::calculate_shares_to_mint,
    state::{UserCard, VaultState, SHARE_MINT_SEED, USER_CARD_SEED, VAULT_SEED, VAULT_TOKEN_SEED},
};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

use crate::rpc::RpcClient;

pub fn token_program_id() -> Pubkey {
    Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap()
}

pub fn ata_program_id() -> Pubkey {
    Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap()
}

pub fn rent_sysvar_id() -> Pubkey {
    Pubkey::from_str("SysvarRent111111111111111111111111111111111").unwrap()
}

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

pub fn derive_ata(wallet: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ata_program_id(),
    )
    .0
}

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

pub fn read_token_balance_bytes(data: &[u8]) -> Result<u64> {
    if data.len() < 72 {
        bail!("Invalid TokenAccount length");
    }
    Ok(u64::from_le_bytes(data[64..72].try_into().unwrap()))
}

/// Разбор десятичной строки в атомарные единицы (`u64`) без использования `f64`.
pub fn parse_ui_amount(input: &str, decimals: u8) -> Result<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("Empty amount");
    }
    let parts: Vec<&str> = trimmed.split('.').collect();
    if parts.len() > 2 {
        bail!("Invalid decimal: {}", input);
    }
    let whole: u128 = if parts[0].is_empty() { 0 } else { parts[0].parse()? };
    let scale = 10u128.pow(decimals as u32);
    let frac: u128 = if parts.len() == 2 {
        if parts[1].len() > decimals as usize {
            bail!("Too many decimals in {}", input);
        }
        let padded = format!("{:0<width$}", parts[1], width = decimals as usize);
        if padded.is_empty() { 0 } else { padded.parse()? }
    } else {
        0
    };
    let raw = whole.checked_mul(scale).and_then(|w| w.checked_add(frac)).ok_or_else(|| anyhow!("Overflow"))?;
    Ok(u64::try_from(raw)?)
}

pub fn read_mint_supply_and_decimals(rpc: &RpcClient, mint: &Pubkey) -> Result<(u64, u8)> {
    let data = rpc.get_account_data(mint)?.ok_or_else(|| anyhow!("Mint not found: {}", mint))?;
    if data.len() < 82 {
        bail!("Invalid Mint length");
    }
    Ok((u64::from_le_bytes(data[36..44].try_into().unwrap()), data[44]))
}

pub fn read_token_balance(rpc: &RpcClient, token_account: &Pubkey) -> Result<u64> {
    match rpc.get_account_data(token_account)? {
        Some(data) if data.len() >= 72 => Ok(u64::from_le_bytes(data[64..72].try_into().unwrap())),
        Some(_) => bail!("Invalid TokenAccount length"),
        None => Ok(0),
    }
}

pub fn fetch_vault_state(rpc: &RpcClient, vault: &Pubkey) -> Result<VaultState> {
    let data = rpc
        .get_account_data(vault)?
        .ok_or_else(|| anyhow!("VaultState not initialized at {}", vault))?;
    let mut slice: &[u8] = &data;
    VaultState::try_deserialize(&mut slice).map_err(|e| anyhow!("Decode VaultState failed: {}", e))
}

pub fn fetch_user_card(rpc: &RpcClient, card: &Pubkey) -> Result<Option<UserCard>> {
    match rpc.get_account_data(card)? {
        Some(data) => {
            let mut slice: &[u8] = &data;
            Ok(Some(UserCard::try_deserialize(&mut slice)?))
        }
        None => Ok(None),
    }
}

pub fn cmd_init_vault(rpc: &RpcClient, admin: &Keypair, deposit_mint: &Pubkey) -> Result<()> {
    let admin_pk = Pubkey::new_from_array(admin.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let (token_vault, _) = derive_token_vault_pda(&vault);
    let (share_mint, _) = derive_share_mint_pda(&vault);
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
    println!("Vault initialized: {} ({})", vault, res.explorer_url);
    Ok(())
}

pub fn cmd_create_card(rpc: &RpcClient, user: &Keypair, deposit_mint: &Pubkey) -> Result<()> {
    let user_pk = Pubkey::new_from_array(user.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let (user_card, _) = derive_user_card_pda(&vault, &user_pk);
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
    println!("UserCard created: {} ({})", user_card, res.explorer_url);
    Ok(())
}

pub fn cmd_show_vault(rpc: &RpcClient, user: &Keypair, deposit_mint: &Pubkey) -> Result<()> {
    let user_pk = Pubkey::new_from_array(user.pubkey().to_bytes());
    let (vault_pda, _) = derive_vault_pda(deposit_mint);
    let state = fetch_vault_state(rpc, &vault_pda)?;
    let (card_pda, _) = derive_user_card_pda(&vault_pda, &user_pk);
    let card = fetch_user_card(rpc, &card_pda)?;
    println!("VaultState: {} (shares={})", vault_pda, state.total_shares);
    println!("UserCard  : {} (exists={})", card_pda, card.is_some());
    Ok(())
}

/// Неделя 9: Депозит токенов в vault с выпуском share-токенов.
pub fn cmd_deposit(rpc: &RpcClient, user: &Keypair, deposit_mint: &Pubkey, ui_amount: &str) -> Result<()> {
    let user_pk = Pubkey::new_from_array(user.pubkey().to_bytes());
    let (vault, _) = derive_vault_pda(deposit_mint);
    let vault_state = fetch_vault_state(rpc, &vault)?;
    let (_, decimals) = read_mint_supply_and_decimals(rpc, deposit_mint)?;
    let amount = parse_ui_amount(ui_amount, decimals)?;
    let (user_card, _) = derive_user_card_pda(&vault, &user_pk);
    let user_token_account = derive_ata(&user_pk, deposit_mint, &token_program_id());
    let user_share_account = derive_ata(&user_pk, &vault_state.share_mint, &token_program_id());

    let shares_before = read_token_balance(rpc, &user_share_account)?;
    let vault_bal = read_token_balance(rpc, &vault_state.token_vault)?;
    let (share_supply, _) = read_mint_supply_and_decimals(rpc, &vault_state.share_mint)?;
    let expected_shares = calculate_shares_to_mint(amount, vault_bal, share_supply)?;

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
    let minted = read_token_balance(rpc, &user_share_account)?.saturating_sub(shares_before);
    if minted != expected_shares {
        bail!("Share mint mismatch: expected {}, got {}", expected_shares, minted);
    }
    println!("Deposited {} raw, minted {} shares ({})", amount, minted, res.explorer_url);
    Ok(())
}

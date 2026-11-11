use std::str::FromStr;

use anchor_lang::{
    prelude::Pubkey,
    solana_program::instruction::Instruction,
    system_program, AccountDeserialize, InstructionData, ToAccountMetas,
};
use anyhow::{anyhow, bail, Result};
use liquidity_vault::{
    accounts, instruction,
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

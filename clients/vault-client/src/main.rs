pub mod rpc;
pub mod vault;

use std::{env, fs, path::PathBuf, str::FromStr};

use anchor_lang::prelude::Pubkey;
use anyhow::{anyhow, bail, Context, Result};
use solana_keypair::Keypair;
use solana_signer::Signer;

use crate::rpc::RpcClient;

pub const DEFAULT_DEPOSIT_MINT_DEVNET: &str = "wKGeYnbJWf2iqakmqFkYGcnmWHQ2roJ6jJ1664Zf2oR";

fn load_keypair(custom_path: Option<&str>) -> Result<Keypair> {
    let candidates: Vec<PathBuf> = if let Some(p) = custom_path {
        vec![PathBuf::from(p)]
    } else {
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        vec![
            PathBuf::from("../.keys/id.json"),
            PathBuf::from(".keys/id.json"),
            PathBuf::from(format!("{}/.config/solana/id.json", home)),
        ]
    };
    for path in &candidates {
        if path.exists() {
            let text = fs::read_to_string(path)?;
            let bytes: Vec<u8> = serde_json::from_str(&text)?;
            let arr: [u8; 64] = bytes.try_into().map_err(|_| anyhow!("Keypair not 64 bytes"))?;
            let secret: [u8; 32] = arr[0..32].try_into().unwrap();
            return Ok(Keypair::new_from_array(secret));
        }
    }
    bail!("No Solana keypair found")
}

fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|idx| args.get(idx + 1).cloned())
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        println!("Usage: vault-client <init|card|show> [--mint <PUBKEY>] [--cluster devnet|localnet]");
        return Ok(());
    }
    let command = args[0].as_str();
    let cluster = parse_flag_value(&args, "--cluster").unwrap_or_else(|| "devnet".to_string());
    let rpc = RpcClient::new(&cluster, parse_flag_value(&args, "--rpc-url").as_deref());
    let genesis = rpc.verify_cluster_genesis()?;
    let wallet = load_keypair(parse_flag_value(&args, "--keypair").as_deref())?;
    let wallet_pk = Pubkey::new_from_array(wallet.pubkey().to_bytes());
    let bal = rpc.get_balance(&wallet_pk)?;
    println!("[RPC] cluster={} genesis={} wallet={} sol={:.4}", cluster, genesis, wallet_pk, (bal as f64) / 1e9);

    let mint_str = parse_flag_value(&args, "--mint").unwrap_or_else(|| DEFAULT_DEPOSIT_MINT_DEVNET.to_string());
    let mint = Pubkey::from_str(&mint_str).with_context(|| "Invalid --mint pubkey")?;

    match command {
        "init" => vault::cmd_init_vault(&rpc, &wallet, &mint)?,
        "card" => vault::cmd_create_card(&rpc, &wallet, &mint)?,
        "show" => vault::cmd_show_vault(&rpc, &wallet, &mint)?,
        "deposit" => {
            let amount = parse_flag_value(&args, "--amount").ok_or_else(|| anyhow!("Missing --amount"))?;
            vault::cmd_deposit(&rpc, &wallet, &mint, &amount)?;
        }
        other => bail!("Unknown command '{}'", other),
    }
    Ok(())
}

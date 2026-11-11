pub mod pool;
pub mod raydium;
pub mod rpc;
pub mod strategy;
pub mod vault;

use std::{env, fs, path::PathBuf, str::FromStr};

use anchor_lang::prelude::Pubkey;
use anyhow::{anyhow, bail, Context, Result};
use solana_keypair::Keypair;
use solana_signer::Signer;

use crate::rpc::RpcClient;

pub const DEFAULT_DEPOSIT_MINT_DEVNET: &str = "wKGeYnbJWf2iqakmqFkYGcnmWHQ2roJ6jJ1664Zf2oR";
pub const DEFAULT_SECONDARY_MINT_DEVNET: &str = "EWXdK61y5MT8HfZypYJBLNj2A2paRAc5pXojcpeFkRwu";
pub const DEFAULT_POOL_STATE_DEVNET: &str = "DKRQA3X9R6oneVnfgXEtCZdQmPRyASYV6rLXv7dBBD3n";

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

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        println!("Usage: vault-client <init|card|show|deposit|withdraw|create-token|create-pool|open-position|pool|strategy|vault-swap>");
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

    let resolve_mint = || Pubkey::from_str(&parse_flag_value(&args, "--mint").unwrap_or_else(|| DEFAULT_DEPOSIT_MINT_DEVNET.to_string())).context("Invalid --mint");
    let resolve_sec = || Pubkey::from_str(&parse_flag_value(&args, "--secondary-mint").unwrap_or_else(|| DEFAULT_SECONDARY_MINT_DEVNET.to_string())).context("Invalid --secondary-mint");
    let resolve_pool = || Pubkey::from_str(&parse_flag_value(&args, "--pool").unwrap_or_else(|| DEFAULT_POOL_STATE_DEVNET.to_string())).context("Invalid --pool");

    match command {
        "init" => vault::cmd_init_vault(&rpc, &wallet, &resolve_mint()?)?,
        "card" => vault::cmd_create_card(&rpc, &wallet, &resolve_mint()?)?,
        "show" => vault::cmd_show_vault(&rpc, &wallet, &resolve_mint()?)?,
        "deposit" => {
            let amount = parse_flag_value(&args, "--amount").ok_or_else(|| anyhow!("Missing --amount"))?;
            vault::cmd_deposit(&rpc, &wallet, &resolve_mint()?, &amount)?;
        }
        "withdraw" => {
            let shares = parse_flag_value(&args, "--shares").or_else(|| parse_flag_value(&args, "--amount")).ok_or_else(|| anyhow!("Missing --shares"))?;
            vault::cmd_withdraw(&rpc, &wallet, &resolve_mint()?, &shares)?;
        }
        "create-token" => {
            let decimals: u8 = parse_flag_value(&args, "--decimals").unwrap_or_else(|| "6".to_string()).parse()?;
            let supply = parse_flag_value(&args, "--supply").unwrap_or_else(|| "1000000".to_string());
            pool::cmd_create_token(&rpc, &wallet, decimals, &supply)?;
        }
        "create-pool" => {
            let price: f64 = parse_flag_value(&args, "--price").unwrap_or_else(|| "1.0".to_string()).parse()?;
            let amm = parse_flag_value(&args, "--amm-config").map(|s| Pubkey::from_str(&s)).transpose()?;
            pool::cmd_create_pool(&rpc, &wallet, &resolve_mint()?, &resolve_sec()?, amm, price)?;
        }
        "open-position" => {
            let tl = parse_flag_value(&args, "--tick-lower").map(|s| s.parse()).transpose()?;
            let tu = parse_flag_value(&args, "--tick-upper").map(|s| s.parse()).transpose()?;
            let a0 = parse_flag_value(&args, "--amount-0").unwrap_or_else(|| "50000".to_string());
            let a1 = parse_flag_value(&args, "--amount-1").unwrap_or_else(|| "50000".to_string());
            pool::cmd_open_wallet_position(&rpc, &wallet, &resolve_pool()?, tl, tu, &a0, &a1)?;
        }
        "pool" => {
            let pos = parse_flag_value(&args, "--position").map(|s| Pubkey::from_str(&s)).transpose()?;
            pool::cmd_show_pool(&rpc, &resolve_pool()?, pos.as_ref())?;
        }
        "strategy" => strategy::cmd_init_strategy(&rpc, &wallet, &resolve_mint()?, &resolve_sec()?, &resolve_pool()?)?,
        "vault-swap" => {
            let amount = parse_flag_value(&args, "--amount").ok_or_else(|| anyhow!("Missing --amount"))?;
            let a_to_b = !has_flag(&args, "--b-to-a");
            let slippage: u64 = parse_flag_value(&args, "--slippage-bps").unwrap_or_else(|| "100".to_string()).parse()?;
            let reject = has_flag(&args, "--reject");
            strategy::cmd_vault_swap(&rpc, &wallet, &resolve_mint()?, &amount, a_to_b, slippage, reject)?;
        }
        other => bail!("Unknown command '{}'", other),
    }
    Ok(())
}

pub mod pool;
pub mod position;
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

/// Адреса развёрнутых артефактов в Solana Devnet по умолчанию (обновляются после деплоя).
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
            let text = fs::read_to_string(path)
                .with_context(|| format!("Failed to read keypair file {}", path.display()))?;
            let bytes: Vec<u8> = serde_json::from_str(&text)
                .with_context(|| format!("Invalid JSON in keypair file {}", path.display()))?;
            let arr: [u8; 64] = bytes
                .try_into()
                .map_err(|_| anyhow!("Keypair in {} is not 64 bytes", path.display()))?;
            let secret: [u8; 32] = arr[0..32]
                .try_into()
                .map_err(|_| anyhow!("Invalid secret key slice in {}", path.display()))?;
            return Ok(Keypair::new_from_array(secret));
        }
    }

    bail!(
        "No Solana keypair found in {:?}",
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    )
}

fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|idx| args.get(idx + 1).cloned())
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn print_help() {
    println!(
        "vault-client — Rust CLI for Liquidity Vault & Raydium CLMM (Weeks 8–13)

USAGE:
  cargo run -p vault-client -- <COMMAND> [OPTIONS]

GLOBAL OPTIONS:
  --cluster <devnet|localnet>   Target Solana cluster (default: devnet)
  --rpc-url <URL>               Custom RPC endpoint URL
  --keypair <PATH>              Wallet keypair path (default: ../.keys/id.json or ~/.config/solana/id.json)
  --mint <PUBKEY>               Base deposit token mint of the vault
  --secondary-mint <PUBKEY>     Secondary token mint of the Raydium CLMM pool
  --pool <PUBKEY>               Raydium CLMM PoolState address

COMMANDS:
  Week 8 (Vault Skeleton & PDAs):
    init                        Initialize VaultState, token_vault, and share_mint PDAs
    card                        Create personal UserCard PDA for the wallet
    show                        Show VaultState, UserCard, wallet balances, and Strategy state

  Week 9 (Deposit & Share Tokens):
    deposit --amount <UI_AMT>   Deposit tokens into vault and mint proportional share tokens

  Week 10 (CPI & Withdraw):
    withdraw --shares <UI_AMT>  Burn share tokens and withdraw proportional underlying tokens

  Week 11 (Raydium CLMM Pool & Initial Position):
    create-token [--decimals 6] [--supply 1000000]
                                Create a test SPL token and mint initial supply to wallet
    create-pool --mint <MINT_A> --secondary-mint <MINT_B> [--price 1.0]
                                Create a Raydium CLMM pool in Devnet
    open-position --pool <POOL> [--tick-lower -600] [--tick-upper 600] [--amount-0 10000] [--amount-1 10000]
                                Open a wallet LP position around current price via Token-2022 NFT
    pool --pool <POOL> [--position <PERSONAL_POS>]
                                Inspect Raydium CLMM PoolState and optional PersonalPositionState

  Week 12 (Vault Strategy & Swap via CPI):
    strategy --mint <MINT_A> --secondary-mint <MINT_B> --pool <POOL>
                                Initialize vault Strategy PDA and secondary_vault PDA
    vault-swap --amount <UI_AMT> [--a-to-b | --b-to-a] [--slippage-bps 100] [--reject]
                                Quote via simulation and swap vault tokens through Raydium CLMM CPI

  Week 13 (Vault CLMM Position, Fees & Rebalancing):
    vault-open-position [--tick-lower <T>] [--tick-upper <T>] [--amount-0 500] [--amount-1 500]
                                Open vault's own concentrated liquidity position via CPI
    trader-swap --pool <POOL> --amount <UI_AMT> [--zero-for-one | --one-for-zero]
                                Swap from external trader wallet to generate fees / move pool price
    vault-collect-fees          Collect accrued trading fees from vault's CLMM position via CPI
    vault-close-position [--slippage-bps 100]
                                Quote minimums via simulation, withdraw all liquidity, and close NFT
    vault-rebalance [--tick-lower <T>] [--tick-upper <T>] [--amount-0 500] [--amount-1 500]
                                Close out-of-range position and open a new one around current price"
    );
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || has_flag(&args, "--help") || has_flag(&args, "-h") {
        print_help();
        return Ok(());
    }

    let command = args[0].as_str();
    let cluster = parse_flag_value(&args, "--cluster").unwrap_or_else(|| "devnet".to_string());
    let rpc_url = parse_flag_value(&args, "--rpc-url");
    let keypair_path = parse_flag_value(&args, "--keypair");

    let rpc = RpcClient::new(&cluster, rpc_url.as_deref());
    let genesis = rpc.verify_cluster_genesis()?;
    let wallet = load_keypair(keypair_path.as_deref())?;
    let wallet_pk = Pubkey::new_from_array(wallet.pubkey().to_bytes());
    let sol_balance = rpc.get_balance(&wallet_pk)?;

    println!(
        "[RPC] cluster={} genesis={} wallet={} balance={:.4} SOL",
        rpc.cluster,
        genesis,
        wallet_pk,
        (sol_balance as f64) / 1_000_000_000.0
    );

    let resolve_deposit_mint = || -> Result<Pubkey> {
        let s = parse_flag_value(&args, "--mint")
            .unwrap_or_else(|| DEFAULT_DEPOSIT_MINT_DEVNET.to_string());
        Pubkey::from_str(&s).with_context(|| format!("Invalid --mint pubkey: {}", s))
    };

    let resolve_secondary_mint = || -> Result<Pubkey> {
        let s = parse_flag_value(&args, "--secondary-mint")
            .unwrap_or_else(|| DEFAULT_SECONDARY_MINT_DEVNET.to_string());
        Pubkey::from_str(&s).with_context(|| format!("Invalid --secondary-mint pubkey: {}", s))
    };

    let resolve_pool = || -> Result<Pubkey> {
        let s = parse_flag_value(&args, "--pool")
            .unwrap_or_else(|| DEFAULT_POOL_STATE_DEVNET.to_string());
        Pubkey::from_str(&s).with_context(|| format!("Invalid --pool pubkey: {}", s))
    };

    match command {
        "init" => {
            let mint = resolve_deposit_mint()?;
            vault::cmd_init_vault(&rpc, &wallet, &mint)?;
        }
        "card" => {
            let mint = resolve_deposit_mint()?;
            vault::cmd_create_card(&rpc, &wallet, &mint)?;
        }
        "show" => {
            let mint = resolve_deposit_mint()?;
            vault::cmd_show_vault(&rpc, &wallet, &mint)?;
        }
        "deposit" => {
            let mint = resolve_deposit_mint()?;
            let amount = parse_flag_value(&args, "--amount")
                .ok_or_else(|| anyhow!("Missing --amount <UI_AMOUNT> for deposit"))?;
            vault::cmd_deposit(&rpc, &wallet, &mint, &amount)?;
        }
        "withdraw" => {
            let mint = resolve_deposit_mint()?;
            let shares = parse_flag_value(&args, "--shares")
                .or_else(|| parse_flag_value(&args, "--amount"))
                .ok_or_else(|| anyhow!("Missing --shares <UI_SHARES> for withdraw"))?;
            vault::cmd_withdraw(&rpc, &wallet, &mint, &shares)?;
        }
        "create-token" => {
            let decimals: u8 = parse_flag_value(&args, "--decimals")
                .unwrap_or_else(|| "6".to_string())
                .parse()?;
            let supply =
                parse_flag_value(&args, "--supply").unwrap_or_else(|| "1000000".to_string());
            pool::cmd_create_token(&rpc, &wallet, decimals, &supply)?;
        }
        "create-pool" => {
            let mint_a = resolve_deposit_mint()?;
            let mint_b = resolve_secondary_mint()?;
            let price: f64 = parse_flag_value(&args, "--price")
                .unwrap_or_else(|| "1.0".to_string())
                .parse()?;
            let amm_config = match parse_flag_value(&args, "--amm-config") {
                Some(s) => Some(Pubkey::from_str(&s)?),
                None => None,
            };
            pool::cmd_create_pool(&rpc, &wallet, &mint_a, &mint_b, amm_config, price)?;
        }
        "open-position" => {
            let pool_addr = resolve_pool()?;
            let tick_lower = match parse_flag_value(&args, "--tick-lower") {
                Some(s) => Some(s.parse()?),
                None => None,
            };
            let tick_upper = match parse_flag_value(&args, "--tick-upper") {
                Some(s) => Some(s.parse()?),
                None => None,
            };
            let amount_0 =
                parse_flag_value(&args, "--amount-0").unwrap_or_else(|| "50000".to_string());
            let amount_1 =
                parse_flag_value(&args, "--amount-1").unwrap_or_else(|| "50000".to_string());
            pool::cmd_open_wallet_position(
                &rpc,
                &wallet,
                &pool_addr,
                tick_lower,
                tick_upper,
                &amount_0,
                &amount_1,
            )?;
        }
        "pool" => {
            let pool_addr = resolve_pool()?;
            let pos_addr = match parse_flag_value(&args, "--position") {
                Some(s) => Some(Pubkey::from_str(&s)?),
                None => None,
            };
            pool::cmd_show_pool(&rpc, &pool_addr, pos_addr.as_ref())?;
        }
        "strategy" => {
            let mint_a = resolve_deposit_mint()?;
            let mint_b = resolve_secondary_mint()?;
            let pool_addr = resolve_pool()?;
            strategy::cmd_init_strategy(&rpc, &wallet, &mint_a, &mint_b, &pool_addr)?;
        }
        "vault-swap" => {
            let mint_a = resolve_deposit_mint()?;
            let amount = parse_flag_value(&args, "--amount")
                .ok_or_else(|| anyhow!("Missing --amount <UI_AMOUNT> for vault-swap"))?;
            let a_to_b = !has_flag(&args, "--b-to-a");
            let slippage_bps: u64 = parse_flag_value(&args, "--slippage-bps")
                .unwrap_or_else(|| "100".to_string())
                .parse()?;
            let reject = has_flag(&args, "--reject");
            strategy::cmd_vault_swap(&rpc, &wallet, &mint_a, &amount, a_to_b, slippage_bps, reject)?;
        }
        "vault-open-position" => {
            let mint_a = resolve_deposit_mint()?;
            let tick_lower = match parse_flag_value(&args, "--tick-lower") {
                Some(s) => Some(s.parse()?),
                None => None,
            };
            let tick_upper = match parse_flag_value(&args, "--tick-upper") {
                Some(s) => Some(s.parse()?),
                None => None,
            };
            let amount_0 =
                parse_flag_value(&args, "--amount-0").unwrap_or_else(|| "200".to_string());
            let amount_1 =
                parse_flag_value(&args, "--amount-1").unwrap_or_else(|| "200".to_string());
            position::cmd_open_vault_position(
                &rpc,
                &wallet,
                &mint_a,
                tick_lower,
                tick_upper,
                &amount_0,
                &amount_1,
            )?;
        }
        "vault-collect-fees" => {
            let mint_a = resolve_deposit_mint()?;
            position::cmd_collect_vault_fees(&rpc, &wallet, &mint_a)?;
        }
        "vault-close-position" => {
            let mint_a = resolve_deposit_mint()?;
            let slippage_bps: u64 = parse_flag_value(&args, "--slippage-bps")
                .unwrap_or_else(|| "100".to_string())
                .parse()?;
            position::cmd_close_vault_position(&rpc, &wallet, &mint_a, slippage_bps)?;
        }
        "vault-rebalance" => {
            let mint_a = resolve_deposit_mint()?;
            let tick_lower = match parse_flag_value(&args, "--tick-lower") {
                Some(s) => Some(s.parse()?),
                None => None,
            };
            let tick_upper = match parse_flag_value(&args, "--tick-upper") {
                Some(s) => Some(s.parse()?),
                None => None,
            };
            let amount_0 =
                parse_flag_value(&args, "--amount-0").unwrap_or_else(|| "200".to_string());
            let amount_1 =
                parse_flag_value(&args, "--amount-1").unwrap_or_else(|| "200".to_string());
            let slippage_bps: u64 = parse_flag_value(&args, "--slippage-bps")
                .unwrap_or_else(|| "100".to_string())
                .parse()?;
            position::cmd_rebalance_vault_position(
                &rpc,
                &wallet,
                &mint_a,
                tick_lower,
                tick_upper,
                &amount_0,
                &amount_1,
                slippage_bps,
            )?;
        }
        "trader-swap" => {
            let pool_addr = resolve_pool()?;
            let amount = parse_flag_value(&args, "--amount")
                .ok_or_else(|| anyhow!("Missing --amount <UI_AMOUNT> for trader-swap"))?;
            let zero_for_one = !has_flag(&args, "--one-for-zero");
            position::cmd_trader_swap(&rpc, &wallet, &pool_addr, &amount, zero_for_one)?;
        }
        other => {
            bail!("Unknown command '{}'. Run with --help for usage.", other);
        }
    }

    Ok(())
}

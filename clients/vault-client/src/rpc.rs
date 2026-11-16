use std::{process::Command, str::FromStr, thread, time::Duration};

use anchor_lang::prelude::Pubkey;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};
use solana_message::Hash;
use solana_transaction::Transaction;

pub const DEVNET_RPC_URL: &str = "https://api.devnet.solana.com";
pub const LOCALNET_RPC_URL: &str = "http://127.0.0.1:8899";
pub const DEVNET_GENESIS_HASH: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";

#[derive(Clone, Debug)]
pub struct RpcClient {
    pub url: String,
    pub cluster: String,
}

#[derive(Clone, Debug)]
pub struct TxExecutionResult {
    pub signature: String,
    pub explorer_url: String,
}

impl RpcClient {
    pub fn new(cluster: &str, custom_url: Option<&str>) -> Self {
        let url = match custom_url {
            Some(u) => u.to_string(),
            None => match cluster {
                "localnet" | "localhost" => LOCALNET_RPC_URL.to_string(),
                _ => DEVNET_RPC_URL.to_string(),
            },
        };
        Self {
            url,
            cluster: cluster.to_string(),
        }
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let body = serde_json::to_string(&payload)?;
        let output = Command::new("curl")
            .args(["-sS", "--max-time", "30", "-X", "POST", "-H", "Content-Type: application/json", "-d", &body, &self.url])
            .output()
            .context("Failed to execute curl")?;
        if !output.status.success() {
            bail!("curl failed: {}", String::from_utf8_lossy(&output.stderr));
        }
        let parsed: Value = serde_json::from_slice(&output.stdout)?;
        if let Some(err) = parsed.get("error") {
            bail!("RPC error in {}: {}", method, err);
        }
        Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn verify_cluster_genesis(&self) -> Result<String> {
        let genesis = self.call("getGenesisHash", json!([]))?
            .as_str()
            .ok_or_else(|| anyhow!("Missing genesis hash"))?
            .to_string();
        if self.cluster == "devnet" && genesis != DEVNET_GENESIS_HASH {
            bail!("Genesis mismatch: expected {}, got {}", DEVNET_GENESIS_HASH, genesis);
        }
        Ok(genesis)
    }

    pub fn get_latest_blockhash(&self) -> Result<Hash> {
        let res = self.call("getLatestBlockhash", json!([{ "commitment": "confirmed" }]))?;
        let bh_str = res["value"]["blockhash"].as_str().ok_or_else(|| anyhow!("Missing blockhash"))?;
        let bytes = bs58::decode(bh_str).into_vec()?;
        let arr: [u8; 32] = bytes.try_into().map_err(|_| anyhow!("Invalid blockhash"))?;
        Ok(Hash::new_from_array(arr))
    }

    pub fn get_balance(&self, pubkey: &Pubkey) -> Result<u64> {
        let res = self.call("getBalance", json!([pubkey.to_string(), { "commitment": "confirmed" }]))?;
        res["value"].as_u64().ok_or_else(|| anyhow!("Missing balance"))
    }

    pub fn get_account_data(&self, pubkey: &Pubkey) -> Result<Option<Vec<u8>>> {
        let res = self.call(
            "getAccountInfo",
            json!([pubkey.to_string(), { "encoding": "base64", "commitment": "confirmed" }]),
        )?;
        let Some(val) = res.get("value").filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let b64 = val["data"][0].as_str().ok_or_else(|| anyhow!("Missing base64 data"))?;
        let _owner = Pubkey::from_str(val["owner"].as_str().unwrap_or_default())?;
        Ok(Some(BASE64.decode(b64)?))
    }

    pub fn send_and_confirm(&self, tx: &Transaction) -> Result<TxExecutionResult> {
        let tx_b64 = BASE64.encode(bincode::serialize(tx)?);
        let sig = self
            .call("sendTransaction", json!([tx_b64, { "encoding": "base64", "preflightCommitment": "confirmed" }]))?
            .as_str()
            .ok_or_else(|| anyhow!("Missing signature"))?
            .to_string();

        for _ in 0..45 {
            thread::sleep(Duration::from_millis(800));
            let st = self.call("getSignatureStatuses", json!([[sig], { "searchTransactionHistory": true }]))?;
            let item = &st["value"][0];
            if !item.is_null() {
                let conf = item["confirmationStatus"].as_str().unwrap_or("");
                if conf == "confirmed" || conf == "finalized" {
                    if let Some(e) = item.get("err").filter(|e| !e.is_null()) {
                        bail!("Transaction {} failed: {}", sig, e);
                    }
                    return Ok(TxExecutionResult {
                        explorer_url: self.explorer_tx_url(&sig),
                        signature: sig,
                    });
                }
            }
        }
        bail!("Timed out waiting for {}", sig)
    }

    pub fn explorer_tx_url(&self, signature: &str) -> String {
        format!("https://explorer.solana.com/tx/{}?cluster={}", signature, self.cluster)
    }

    pub fn explorer_address_url(&self, address: &Pubkey) -> String {
        format!("https://explorer.solana.com/address/{}?cluster={}", address, self.cluster)
    }
}

use std::{process::Command, str::FromStr, thread, time::Duration};

use anchor_lang::prelude::Pubkey;
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};
use solana_account::Account;
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
pub struct SimulationOutput {
    pub err: Option<Value>,
    pub logs: Vec<String>,
    pub units_consumed: u64,
    pub accounts: Vec<Option<Account>>,
}

#[derive(Clone, Debug)]
pub struct TxExecutionResult {
    pub signature: String,
    pub err: Option<Value>,
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
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let body = serde_json::to_string(&payload)?;

        let mut last_err = None;
        for attempt in 0..5 {
            if attempt > 0 {
                thread::sleep(Duration::from_millis(500 * attempt as u64));
            }
            let output = Command::new("curl")
                .args([
                    "-sS",
                    "--max-time",
                    "30",
                    "-X",
                    "POST",
                    "-H",
                    "Content-Type: application/json",
                    "-d",
                    &body,
                    &self.url,
                ])
                .output()
                .context("Failed to execute curl for JSON-RPC call")?;

            if !output.status.success() {
                last_err = Some(anyhow!(
                    "curl failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
                continue;
            }

            let resp_text = String::from_utf8_lossy(&output.stdout);
            let parsed: Value = match serde_json::from_str(&resp_text) {
                Ok(v) => v,
                Err(e) => {
                    last_err = Some(anyhow!("Invalid JSON from RPC ({}): {}", e, resp_text));
                    continue;
                }
            };

            if let Some(err) = parsed.get("error") {
                let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                if code == 429 || code == -32005 {
                    last_err = Some(anyhow!("RPC rate limit: {}", err));
                    continue;
                }
                bail!("RPC error in {}: {}", method, err);
            }

            return Ok(parsed.get("result").cloned().unwrap_or(Value::Null));
        }

        Err(last_err.unwrap_or_else(|| anyhow!("RPC request {} failed after retries", method)))
    }

    /// Проверка соответствия выбранной сети по `getGenesisHash`.
    pub fn verify_cluster_genesis(&self) -> Result<String> {
        let genesis = self
            .call("getGenesisHash", json!([]))?
            .as_str()
            .ok_or_else(|| anyhow!("Missing genesis hash string"))?
            .to_string();

        if self.cluster == "devnet" && genesis != DEVNET_GENESIS_HASH {
            bail!(
                "Genesis hash mismatch for devnet: expected {}, got {}",
                DEVNET_GENESIS_HASH,
                genesis
            );
        }
        if self.cluster == "localnet" && genesis == DEVNET_GENESIS_HASH {
            bail!("Refusing to treat Devnet RPC as localnet");
        }
        Ok(genesis)
    }

    pub fn get_latest_blockhash(&self) -> Result<Hash> {
        let res = self.call(
            "getLatestBlockhash",
            json!([{ "commitment": "confirmed" }]),
        )?;
        let bh_str = res
            .get("value")
            .and_then(|v| v.get("blockhash"))
            .and_then(|b| b.as_str())
            .ok_or_else(|| anyhow!("Malformed getLatestBlockhash response"))?;
        let bytes = bs58::decode(bh_str).into_vec()?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow!("Invalid blockhash length"))?;
        Ok(Hash::new_from_array(arr))
    }

    pub fn get_balance(&self, pubkey: &Pubkey) -> Result<u64> {
        let res = self.call(
            "getBalance",
            json!([pubkey.to_string(), { "commitment": "confirmed" }]),
        )?;
        res.get("value")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("Malformed getBalance response"))
    }

    pub fn get_account_data(&self, pubkey: &Pubkey) -> Result<Option<Vec<u8>>> {
        Ok(self.get_account(pubkey)?.map(|a| a.data))
    }

    pub fn get_account(&self, pubkey: &Pubkey) -> Result<Option<Account>> {
        let res = self.call(
            "getAccountInfo",
            json!([
                pubkey.to_string(),
                { "encoding": "base64", "commitment": "confirmed" }
            ]),
        )?;
        let val = match res.get("value") {
            Some(v) if !v.is_null() => v,
            _ => return Ok(None),
        };
        Self::decode_rpc_account(val).map(Some)
    }

    fn decode_rpc_account(val: &Value) -> Result<Account> {
        let lamports = val
            .get("lamports")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("Missing lamports"))?;
        let owner_str = val
            .get("owner")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing owner"))?;
        let executable = val
            .get("executable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let rent_epoch = val
            .get("rentEpoch")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let data_b64 = val
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
            .ok_or_else(|| anyhow!("Missing base64 data"))?;
        let data = BASE64.decode(data_b64)?;
        let owner = Pubkey::from_str(owner_str)?;

        Ok(Account {
            lamports,
            data,
            owner: owner.to_bytes().into(),
            executable,
            rent_epoch,
        })
    }

    pub fn simulate_transaction(&self, tx: &Transaction) -> Result<SimulationOutput> {
        self.simulate_with_accounts(tx, &[])
    }

    /// Симуляция транзакции с опциональным чтением состояния аккаунтов после исполнения (для котировок).
    pub fn simulate_with_accounts(
        &self,
        tx: &Transaction,
        inspect_keys: &[Pubkey],
    ) -> Result<SimulationOutput> {
        let raw_tx = bincode::serialize(tx)?;
        let tx_b64 = BASE64.encode(raw_tx);

        let mut cfg = json!({
            "encoding": "base64",
            "sigVerify": false,
            "replaceRecentBlockhash": true,
            "commitment": "confirmed"
        });

        if !inspect_keys.is_empty() {
            let addresses: Vec<String> = inspect_keys.iter().map(|k| k.to_string()).collect();
            cfg.as_object_mut().unwrap().insert(
                "accounts".to_string(),
                json!({
                    "encoding": "base64",
                    "addresses": addresses
                }),
            );
        }

        let res = self.call("simulateTransaction", json!([tx_b64, cfg]))?;
        let val = res
            .get("value")
            .ok_or_else(|| anyhow!("Malformed simulateTransaction response"))?;

        let err = val.get("err").and_then(|e| {
            if e.is_null() {
                None
            } else {
                Some(e.clone())
            }
        });
        let logs = val
            .get("logs")
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let units_consumed = val
            .get("unitsConsumed")
            .and_then(|u| u.as_u64())
            .unwrap_or(0);

        let mut accounts = Vec::new();
        if let Some(acc_arr) = val.get("accounts").and_then(|a| a.as_array()) {
            for item in acc_arr {
                if item.is_null() {
                    accounts.push(None);
                } else {
                    accounts.push(Some(Self::decode_rpc_account(item)?));
                }
            }
        }

        Ok(SimulationOutput {
            err,
            logs,
            units_consumed,
            accounts,
        })
    }

    /// Отправка транзакции и ожидание подтверждения (`skip_preflight = true` используется для записи отклонённого swap в блокчейн).
    pub fn send_and_confirm(
        &self,
        tx: &Transaction,
        skip_preflight: bool,
    ) -> Result<TxExecutionResult> {
        if !skip_preflight {
            let sim = self.simulate_transaction(tx)?;
            if let Some(err) = sim.err {
                bail!(
                    "Transaction simulation failed: {}\nLogs:\n{}",
                    err,
                    sim.logs.join("\n")
                );
            }
        }

        let raw_tx = bincode::serialize(tx)?;
        let tx_b64 = BASE64.encode(raw_tx);

        let sig = self
            .call(
                "sendTransaction",
                json!([
                    tx_b64,
                    {
                        "encoding": "base64",
                        "skipPreflight": skip_preflight,
                        "preflightCommitment": "confirmed",
                        "maxRetries": 5
                    }
                ]),
            )?
            .as_str()
            .ok_or_else(|| anyhow!("Missing signature from sendTransaction"))?
            .to_string();

        for _ in 0..45 {
            thread::sleep(Duration::from_millis(800));
            let statuses = self.call(
                "getSignatureStatuses",
                json!([[sig], { "searchTransactionHistory": true }]),
            )?;
            if let Some(status) = statuses
                .get("value")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
            {
                if !status.is_null() {
                    let conf = status
                        .get("confirmationStatus")
                        .and_then(|c| c.as_str())
                        .unwrap_or("");
                    if conf == "confirmed" || conf == "finalized" {
                        let err = status.get("err").and_then(|e| {
                            if e.is_null() {
                                None
                            } else {
                                Some(e.clone())
                            }
                        });
                        if !skip_preflight {
                            if let Some(ref e) = err {
                                bail!(
                                    "Transaction {} failed on-chain: {}\nExplorer: {}",
                                    sig,
                                    e,
                                    self.explorer_tx_url(&sig)
                                );
                            }
                        }
                        return Ok(TxExecutionResult {
                            explorer_url: self.explorer_tx_url(&sig),
                            signature: sig,
                            err,
                        });
                    }
                }
            }
        }

        bail!("Timed out waiting for confirmation of transaction {}", sig)
    }

    pub fn explorer_tx_url(&self, signature: &str) -> String {
        match self.cluster.as_str() {
            "localnet" | "localhost" => format!(
                "https://explorer.solana.com/tx/{}?cluster=custom&customUrl={}",
                signature, self.url
            ),
            _ => format!(
                "https://explorer.solana.com/tx/{}?cluster={}",
                signature, self.cluster
            ),
        }
    }

    pub fn explorer_address_url(&self, address: &Pubkey) -> String {
        match self.cluster.as_str() {
            "localnet" | "localhost" => format!(
                "https://explorer.solana.com/address/{}?cluster=custom&customUrl={}",
                address, self.url
            ),
            _ => format!(
                "https://explorer.solana.com/address/{}?cluster={}",
                address, self.cluster
            ),
        }
    }
}

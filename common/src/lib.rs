// common/src/lib.rs
use serde::Serialize;
use anyhow::Result;
use base64::{decode, encode};
use bincode::{deserialize, serialize};
use bs58;
use dotenv::dotenv;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize as SerdeSerialize;
use serde_json::json;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    signature::{read_keypair_file, Keypair, Signer},
    transaction::VersionedTransaction,
};
use std::env;
use solana_program::address_lookup_table::state::AddressLookupTable;
use solana_sdk::{
    address_lookup_table_account::AddressLookupTableAccount,
    instruction::Instruction,
    message::{v0::Message, VersionedMessage},
    pubkey::Pubkey,
};
use solana_sdk::{
    instruction::{AccountMeta},
    
};
use std::str::FromStr;
use solana_program::instruction::CompiledInstruction;

// ─────────────────── Configuration ───────────────────

pub struct Config {
    pub rpc_url: String,
    pub keypair_path: String,
}

pub fn load_config() -> Config {
    dotenv().ok();
    let rpc_url = env::var("RPC_URL").expect("RPC_URL must be set");
    let keypair_path = env::var("KEYPAIR_PATH").unwrap_or_default();
    Config { rpc_url, keypair_path }
}

pub fn rpc_client(cfg: &Config) -> RpcClient {
    RpcClient::new(cfg.rpc_url.clone())
}

pub fn http_client() -> Client {
    Client::builder().build().unwrap()
}

trait JupiterReqExt {
    fn with_jupiter_key(self) -> Self;
}
/// If API_KEY provided as `API_KEY` in env var, attach it as the `X-API-KEY`
impl JupiterReqExt for reqwest::RequestBuilder {
    fn with_jupiter_key(self) -> Self {
        match std::env::var("API_KEY") {
            Ok(key) if !key.is_empty() => self.header("X-API-KEY", key),
            _ => self,
        }
    }
}


/// Load Keypair from SECRET_KEY (base58) env var or fallback to KEYPAIR_PATH file
pub fn keypair(cfg: &Config) -> Keypair {
    if let Ok(secret_b58) = env::var("SECRET_KEY") {
        let bytes = bs58::decode(secret_b58)
            .into_vec()
            .expect("Invalid base58 in SECRET_KEY");
        Keypair::from_bytes(&bytes).expect("Failed to construct Keypair from SECRET_KEY")
    } else {
        read_keypair_file(&cfg.keypair_path).expect("Failed to read keypair from file")
    }
}



// Helper to sign a versioned transaction
fn sign_versioned_tx(tx: &mut VersionedTransaction, kp: &Keypair) {
    let message = tx.message.clone();
    let serialized = message.serialize();
    let signature = kp.try_sign_message(&serialized)
        .expect("Failed to sign transaction");
    tx.signatures = vec![signature];
}

// ────────── optional integrator-fee helper ──────────
fn integrator_fee() -> Option<(String, u64)> {
    let acc = std::env::var("FEE_ACCOUNT").ok().filter(|s| !s.is_empty());
    let bps = std::env::var("FEE_BPS").ok().and_then(|s| s.parse::<u64>().ok());
    match (acc, bps) {
        (Some(a), Some(b)) if b > 0 => Some((a, b)),   // both present & valid
        _ => None,                                     // fee disabled
    }
}


// ─────────────────── Swap Flow (/quote -> /swap -> send) ───────────────────

#[derive(Serialize, Deserialize, Debug)]
pub struct QuoteResponse {
    pub inputMint: String,
    pub inAmount: String,
    pub outputMint: String,
    pub outAmount: String,
    pub otherAmountThreshold: String,
    pub swapMode: String,
    pub slippageBps: u64,
    #[serde(default)] pub platformFee: Option<serde_json::Value>,
    pub priceImpactPct: String,
    pub routePlan: Vec<serde_json::Value>,
    pub contextSlot: u64,
    pub timeTaken: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SwapResponse {
    #[serde(rename = "swapTransaction")]
    pub swap_transaction: String,
    #[serde(rename = "lastValidBlockHeight")]
    pub last_valid_block_height: u64,
}

pub async fn swap_flow() -> Result<()> {
    let cfg = load_config();
    let http = http_client();
    let rpc  = rpc_client(&cfg);
    let mut kp = keypair(&cfg);
    let user_pubkey = kp.pubkey().to_string();

    // 1. Get quote for 0.05 SOL (50_000_000 lamports)
    let input_mint  = "So11111111111111111111111111111111111111112";
    let output_mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    let amount      = 50_000_000u64;
    let slippage    = 50;
    let fee_q = integrator_fee()
        .map(|(_, bps)| format!("&platformFeeBps={}", bps))
        .unwrap_or_default();
    let quote_url = format!(
        "https://lite-api.jup.ag/swap/v1/quote?inputMint={}&outputMint={}&amount={}&slippageBps={}{}",
        input_mint, output_mint, amount, slippage, fee_q
    );
    let quote: QuoteResponse = http.get(&quote_url).with_jupiter_key().send().await?.json().await?;

    // 2. Build swap transaction
     let mut swap_body = json!({
     "quoteResponse": quote,
     "userPublicKey": user_pubkey,
     "payer": user_pubkey, // Use same account for both user and payer
    });
    if let Some((acc, _)) = integrator_fee() {
        swap_body["feeAccount"] = acc.into();
    }
    let swap_resp: SwapResponse = http
        .post("https://lite-api.jup.ag/swap/v1/swap")
        .with_jupiter_key()
        .json(&swap_body)
        .send().await?
        .json().await?;

    // 3. Decode, sign, and send via RPC
    let mut tx: VersionedTransaction = deserialize(&decode(&swap_resp.swap_transaction)?)?;
    sign_versioned_tx(&mut tx, &kp);
    let signature = rpc.send_and_confirm_transaction(&tx)?;
    println!("Swap confirmed: {}", signature);

    Ok(())
}


// ────────── Swap Instructions Flow (/swap/v1/swap-instructions → build & send) ──────────



#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum Ci {
    Json(InstructionJson),
    B64(String), 
}

#[derive(Deserialize, Debug)]
struct InstructionJson {
    #[serde(rename = "programId")]
    program_id: String,
    accounts: Vec<AccountMetaJson>,
    data: String, // base-64
}

#[derive(Deserialize, Debug)]
struct AccountMetaJson {
    pubkey: String,
    isSigner: bool,
    isWritable: bool,
}

impl TryFrom<InstructionJson> for Instruction {
    type Error = anyhow::Error;

    fn try_from(j: InstructionJson) -> Result<Self, Self::Error> {
        use std::str::FromStr;

        let program_id = Pubkey::from_str(&j.program_id)?;
        let accounts = j
            .accounts
            .into_iter()
            .map(|a| {
                let pk = Pubkey::from_str(&a.pubkey)?;
                Ok(if a.isWritable {
                    AccountMeta::new(pk, a.isSigner)
                } else {
                    AccountMeta::new_readonly(pk, a.isSigner)
                })
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()?;

        Ok(Instruction {
            program_id,
            data: base64::decode(j.data)?,
            accounts,
        })
    }
}

impl Ci {
    fn into_instruction(self) -> Result<Instruction, anyhow::Error> {
        match self {
            Ci::Json(j) => j.try_into(),
            Ci::B64(_) => anyhow::bail!(
                "legacy CompiledInstruction returned – \
                 re-issue the API call with \"instructionFormat\":\"json\""
            ),
        }
    }
}

// ───────────────────────── response struct ─────────────────────────
#[derive(Deserialize, Debug)]
struct SwapInstructionResponse {
    #[serde(default, rename = "tokenLedgerInstruction")]
    token_ledger_instruction: Option<Ci>,

    #[serde(default, rename = "computeBudgetInstructions")]
    compute_budget_instructions: Option<Vec<Ci>>,

    #[serde(default, rename = "setupInstructions")]
    setup_instructions: Option<Vec<Ci>>,

    #[serde(default, rename = "swapInstruction")]
    swap_instruction: Option<Ci>,

    
    #[serde(default, rename = "cleanupInstruction")]
    cleanup_instruction: Option<Ci>,

    #[serde(default, rename = "addressLookupTableAddresses")]
    address_lookup_table_addresses: Option<Vec<String>>,
}

// ───────────────────────────────── flow ────────────────────────────
pub async fn swap_instruction_flow() -> Result<()> {
    let cfg  = load_config();
    let http = http_client();
    let rpc  = rpc_client(&cfg);
    let kp   = keypair(&cfg);

    // ─────────── /quote ─────────────────────────────────────────────
    let fee_q = integrator_fee()
        .map(|(_, bps)| format!("&platformFeeBps={}", bps))
        .unwrap_or_default();

    let quote_url = format!(
        concat!(
            "https://lite-api.jup.ag/swap/v1/quote",
            "?inputMint={}&outputMint={}",
            "&amount=1000000",
            "&slippageBps=50{}"
        ),
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",     // input mint
        "So11111111111111111111111111111111111111112",      // output mint
        fee_q
    );

    let quote: serde_json::Value = http
        .get(quote_url)
        .with_jupiter_key()
        .send()
        .await?
        .json()
        .await?;

    // ─────────── /swap-instructions ─────────────────────────────────
    let user_pubkey = kp.pubkey().to_string();
    let mut body = json!({
        "quoteResponse": quote,
        "userPublicKey": user_pubkey,
        "payer": user_pubkey, // Use same account for both user and payer
        "instructionFormat": "json",
    });
    if let Some((acc, _)) = integrator_fee() {
        body["feeAccount"] = acc.into();
    }

    let resp: SwapInstructionResponse = http
        .post("https://lite-api.jup.ag/swap/v1/swap-instructions")
        .with_jupiter_key()
        .json(&body)
        .send()
        .await?
        .json()
        .await?;


    // decode every Instruction ----------------------------------------------
    let mut ix: Vec<Instruction> = Vec::new();

    if let Some(ci)  = resp.token_ledger_instruction  { ix.push(ci.into_instruction()?); }
    if let Some(lst) = resp.compute_budget_instructions { for ci in lst { ix.push(ci.into_instruction()?); } }
    if let Some(lst) = resp.setup_instructions        { for ci in lst { ix.push(ci.into_instruction()?); } }
    if let Some(ci)  = resp.swap_instruction          { ix.push(ci.into_instruction()?); }
    if let Some(ci)  = resp.cleanup_instruction       { ix.push(ci.into_instruction()?); }

    if ix.is_empty() {
        anyhow::bail!("swap-instructions API returned no instructions – check amount/slippage");
    }

    // fetch & build ALT accounts --------------------------------------------
    let mut alts: Vec<AddressLookupTableAccount> = Vec::new();
    if let Some(addrs) = resp.address_lookup_table_addresses {
        for addr in addrs {
            let key = Pubkey::from_str(&addr)?;
            if let Ok(raw) = rpc.get_account(&key) {
                if let Ok(table) = AddressLookupTable::deserialize(&raw.data) {
                    alts.push(AddressLookupTableAccount {
                        key,
                        addresses: table.addresses.to_vec(),
                    });
                }
            }
        }
    }

    // compile message & send -------------------------------------------------
    let payer            = kp.pubkey();  // Use main account as transaction payer
    let recent_blockhash = rpc.get_latest_blockhash()?;
    let msg              = Message::try_compile(&payer, &ix, &alts, recent_blockhash)?;
    let versioned        = VersionedMessage::V0(msg);
    let tx               = VersionedTransaction::try_new(versioned, &[&kp])?;  // Sign with main keypair only

    let sig = rpc.send_and_confirm_transaction(&tx)?;
    println!("swap-instructions tx confirmed: {sig}");
    Ok(())
}







// ───────────────────────────────── Ultra Flow (/ultra/v1/order -> /ultra/v1/execute) ─────────────────────────────────

#[derive(Deserialize, Debug)]
pub struct UltraOrderResponse {
    pub requestId: String,
    pub transaction: String,
}

#[derive(Deserialize, Debug)]
pub struct UltraExecuteResponse {
    #[serde(default)] pub status: Option<String>,
    #[serde(default)] pub signature: Option<String>,
    /// slot as string from API
    #[serde(default)] pub slot: Option<String>,
    #[serde(flatten)] pub extra: serde_json::Value,
}

pub async fn ultra_flow() -> Result<()> {
    let cfg = load_config();
    let http = http_client();
    let rpc  = rpc_client(&cfg);
    let kp   = keypair(&cfg);
    let taker = kp.pubkey().to_string();
     

    let fee_part = integrator_fee()
        .map(|(acc, bps)| format!("&referralAccount={}&referralFee={}", acc, bps.max(50)))
        .unwrap_or_default();

    let order_url = format!(
        "https://lite-api.jup.ag/ultra/v1/order?inputMint={}&outputMint={}&amount={}&taker={}{}",
        "So11111111111111111111111111111111111111112",
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        10_000_000u64,
        taker,
        fee_part
    );
    let order: UltraOrderResponse = http.get(&order_url).with_jupiter_key().send().await?.json().await?;

    let mut tx: VersionedTransaction = deserialize(&decode(&order.transaction)?)?;
    sign_versioned_tx(&mut tx, &kp);

    let signed_bytes = bincode::serialize(&tx)?;   // Vec<u8>
    let signed       = base64::encode(&signed_bytes);

    let exec_body = json!({
        "signedTransaction": signed,
        "requestId": order.requestId,
    });
    let exec_resp: UltraExecuteResponse = http
        .post("https://lite-api.jup.ag/ultra/v1/execute")
        .with_jupiter_key()
        .json(&exec_body)
        .send().await?
        .json().await?;

    println!("Ultra execute: {:#?}", exec_resp);
    Ok(())
}


// ─────────────────────────────── Trigger Flow (/trigger/v1/createOrder -> /trigger/v1/execute) ──────────────────────────────


#[derive(Deserialize, Debug)]
pub struct CreateTriggerResponse {
    /// Base64-encoded unsigned transaction
    #[serde(default, rename = "transaction", alias = "tx", alias = "transactions")]
    pub transaction: Option<String>,

    /// Request ID for matching with execute
    #[serde(default, rename = "requestId", alias = "request_id")]
    pub request_id: Option<String>,

    /// Order public key string
    #[serde(default, rename = "order")]
    pub order: Option<String>,

    /// Any additional fields (e.g. code, error)
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

#[derive(Deserialize, Debug)]
pub struct ExecuteTriggerResponse {
    pub status: String,
    pub signature: String,
    #[serde(flatten)] pub extra: serde_json::Value,
}

pub async fn trigger_flow() -> Result<()> {
    let cfg  = load_config();
    let http = http_client();
    let rpc  = rpc_client(&cfg);
    let mut kp = keypair(&cfg);
    let user = kp.pubkey().to_string();

    // 1. Create order ---------------------------------------------------------
    let mut create_body = json!({
        "inputMint":  "So11111111111111111111111111111111111111112",
        "outputMint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        "maker":      user,
        "payer":      user,
        "params": {
            "makingAmount": "30000000",
            "takingAmount": "5000000"
        }
    });
    if let Some((_, bps)) = integrator_fee() {
        create_body["params"]["feeBps"] = bps.into();
    }

    let create_resp: CreateTriggerResponse = http
        .post("https://lite-api.jup.ag/trigger/v1/createOrder")
        .with_jupiter_key()
        .json(&create_body)
        .send().await?
        .json().await?;

    // 2. Decode, sign, execute -------------------------------------------------
    if create_resp.transaction.as_deref().unwrap_or("").is_empty() {
        eprintln!("Trigger createOrder failed: {:#?}", create_resp.extra);
        return Ok(());
    }
    let tx_b64 = create_resp.transaction.as_ref().unwrap();
    let mut tx: VersionedTransaction = deserialize(&decode(tx_b64)?)?;
    sign_versioned_tx(&mut tx, &kp);
    let signed = encode(&serialize(&tx)?);

    let exec_body = json!({
        "signedTransaction": signed,
        "requestId": create_resp.request_id.as_ref().unwrap_or(&String::new()),
    });
    let exec_resp: ExecuteTriggerResponse = http
        .post("https://lite-api.jup.ag/trigger/v1/execute")
        .with_jupiter_key()
        .json(&exec_body)
        .send().await?
        .json().await?;

    println!("Trigger execute: {:#?}", exec_resp);
    Ok(())
}







// ─────────────────────────── Recurring Flow (/recurring/v1/createOrder -> /recurring/v1/execute) ──────────────────────────


#[derive(Deserialize, Debug)]
pub struct CreateRecurringResponse {
    /// Base64-encoded unsigned transaction
    #[serde(default, rename = "transaction", alias = "tx", alias = "transactions")]
    pub transaction: Option<String>,

    /// Request ID for matching with execute
    #[serde(default, rename = "requestId", alias = "request_id")]
    pub request_id: Option<String>,

    /// Order public key string
    #[serde(default, rename = "order")]
    pub order: Option<String>,

    
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

#[derive(Deserialize, Debug)]
pub struct ExecuteRecurringResponse {
    pub signature: String,
    pub status: String,
    pub order: Option<String>,
    pub error: Option<String>,
}

pub async fn recurring_flow() -> Result<()> {
    let cfg = load_config();
    let http = http_client();
    let rpc  = rpc_client(&cfg);
    let mut kp = keypair(&cfg);
    let user = kp.pubkey().to_string();

    // 1. Create order
    let create_body = json!({
        "user":       user,
        "inputMint":  "So11111111111111111111111111111111111111112",
        "outputMint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        "params": { "time": { "inAmount": 50000000, "numberOfOrders": 2, "interval": 86400 } },
    });
    let create_resp: CreateRecurringResponse = http
        .post("https://lite-api.jup.ag/recurring/v1/createOrder")
        .with_jupiter_key()
        .json(&create_body)
        .send().await?
        .json().await?;

    // 2. Decode, sign, execute
    if create_resp.transaction.as_deref().unwrap_or("").is_empty() {
        eprintln!("Recurring createOrder failed: {:#?}", create_resp.extra);
        return Ok(());
    }
    let tx_b64 = create_resp.transaction.as_ref().unwrap();
    let mut tx: VersionedTransaction = deserialize(&decode(tx_b64)?)?;
    sign_versioned_tx(&mut tx, &kp);
    let signed = encode(&serialize(&tx)?);

    let exec_body = json!({
        "signedTransaction": signed,
        "requestId":         create_resp.request_id.as_ref().unwrap_or(&String::new()),
    });
    let exec_resp: ExecuteRecurringResponse = http
        .post("https://lite-api.jup.ag/recurring/v1/execute")
        .with_jupiter_key()
        .json(&exec_body)
        .send().await?
        .json().await?;

    println!("Recurring execute: {:#?}", exec_resp);
    
    Ok(())
}


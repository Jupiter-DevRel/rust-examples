use common::{load_config, trigger_flow};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // load .env (RPC_URL, KEYPAIR_PATH)
    let _cfg = load_config();
    // run the stub flow
    trigger_flow().await?;
    Ok(())
}

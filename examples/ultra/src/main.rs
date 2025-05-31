use common::{load_config, ultra_flow};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // load .env (RPC_URL, KEYPAIR_PATH)
    let _cfg = load_config();
    // run the stub flow
    ultra_flow().await?;
    Ok(())
}

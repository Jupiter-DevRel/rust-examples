// examples/swap/src/main.rs

use common::{load_config, swap_flow};
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env (RPC_URL, KEYPAIR_PATH or SECRET_KEY)
    let _cfg = load_config();

    // Execute the swap flow
    swap_flow().await?;

    Ok(())
}

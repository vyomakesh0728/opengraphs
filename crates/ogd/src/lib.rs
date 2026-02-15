use anyhow::Result;

pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";

pub async fn start(bind_addr: &str) -> Result<()> {
    tracing::info!(%bind_addr, "ogd placeholder started");
    println!("ogd listening on {bind_addr} (placeholder)");
    Ok(())
}

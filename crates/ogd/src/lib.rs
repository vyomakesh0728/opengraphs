use anyhow::Result;

pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";

pub async fn start(bind_addr: &str) -> Result<()> {
    tracing::info!(%bind_addr, "ogd placeholder started");
    println!("ogd listening on {bind_addr} (placeholder)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_BIND_ADDR, start};

    #[test]
    fn default_bind_addr_stays_on_localhost_port_8787() {
        let addr: std::net::SocketAddr = DEFAULT_BIND_ADDR
            .parse()
            .expect("default bind addr should parse as a socket address");

        assert!(addr.ip().is_loopback());
        assert_eq!(addr.port(), 8787);
    }

    #[tokio::test]
    async fn start_returns_ok_for_default_bind_addr() {
        start(DEFAULT_BIND_ADDR)
            .await
            .expect("placeholder daemon start should succeed");
    }

    #[tokio::test]
    async fn start_returns_ok_for_custom_bind_addr() {
        start("0.0.0.0:9999")
            .await
            .expect("placeholder daemon start should accept a custom bind address");
    }
}

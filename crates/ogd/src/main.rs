use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Ensure the trackio-rs crate is linked and ready for future integration.
    let _ = std::any::type_name::<trackio_rs::Client>();
    ogd::start(ogd::DEFAULT_BIND_ADDR).await
}

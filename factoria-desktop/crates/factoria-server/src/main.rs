//! Arranque del host HTTP local.
//!
//! ```text
//! factoria-server --port 0 --static dist
//! ```

use anyhow::Result;
use factoria_core::AppCore;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("FACTORIA_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut port: u16 = 0;
    let mut static_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => port = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--static" => static_dir = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                println!("factoria-server [--port <n>] [--static <dir>]");
                return Ok(());
            }
            other => anyhow::bail!("argumento desconocido: {other}"),
        }
    }

    if static_dir.is_none() {
        let default = PathBuf::from("dist");
        if default.is_dir() {
            static_dir = Some(default);
        }
    }

    let core = AppCore::bootstrap()?;
    let server = factoria_server::serve(core, port, static_dir).await?;

    // Una línea que el arranque de desarrollo y las pruebas pueden leer.
    println!("FACTORIA_READY {}", server.ui_url());
    tokio::signal::ctrl_c().await?;
    Ok(())
}

mod log;
mod persistent;
mod simple;

use clap::Parser;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

use log::{log, log_error};
use persistent::{handle_persist_connection, ClientRegistry};
use simple::handle_simple_connection;

#[derive(Parser)]
#[command(name = "stdio-to-ws", about = "Bridge stdio processes to WebSocket connections")]
struct Cli {
    /// The command to run (e.g. "python my-script.py")
    command: String,

    /// Port to listen on
    #[arg(short, long, default_value_t = 3000)]
    port: u16,

    /// Enable process persistence for reconnections
    #[arg(long, default_value_t = false)]
    persist: bool,

    /// Grace period in seconds before killing child process on disconnect
    /// (default: 30, -1 for infinite, requires --persist)
    #[arg(short, long, default_value_t = 30, allow_hyphen_values = true)]
    grace_period: i64,

    /// Suppress logging output
    #[arg(short, long, default_value_t = false)]
    quiet: bool,

    /// Message framing mode: line (default) or raw
    #[arg(short, long, default_value = "line", value_enum)]
    framing: FramingMode,

    /// Bind address
    #[arg(short, long, default_value = "0.0.0.0")]
    bind: String,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum FramingMode {
    /// Each stdout line becomes a WS message; newlines ensured on stdin
    Line,
    /// Raw chunks forwarded as-is; Content-Length headers stripped
    Raw,
}

pub fn strip_content_length(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix("Content-Length: ") {
        if let Some(pos) = rest.find("\r\n\r\n") {
            return &rest[pos + 4..];
        }
        if let Some(pos) = rest.find("\n\n") {
            return &rest[pos + 2..];
        }
    }
    s
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let command_parts = shell_words::split(&cli.command)?;
    if command_parts.is_empty() {
        eprintln!("No command provided.");
        std::process::exit(1);
    }

    let addr: SocketAddr = format!("{}:{}", cli.bind, cli.port).parse()?;
    let listener = TcpListener::bind(&addr).await?;

    let command = Arc::new(command_parts);
    let quiet = cli.quiet;
    let persist = cli.persist;
    let framing = cli.framing;
    let grace_period_ms: i64 = if cli.grace_period == -1 {
        -1
    } else {
        cli.grace_period * 1000
    };

    let clients: ClientRegistry = Arc::new(Mutex::new(HashMap::new()));

    if persist {
        let grace_display = if grace_period_ms == -1 {
            "infinite".to_string()
        } else {
            format!("{}s", grace_period_ms / 1000)
        };
        log(
            &format!(
                "WebSocket server listening on port {} (persistence enabled, grace period: {})",
                cli.port, grace_display
            ),
            quiet,
        );
    } else {
        log(&format!("WebSocket server listening on ws://{}", addr), quiet);
    }

    loop {
        let (stream, _peer) = listener.accept().await?;
        let command = Arc::clone(&command);
        let clients = Arc::clone(&clients);

        tokio::spawn(async move {
            // Extract X-Client-Id header during handshake (sync mutex for sync callback)
            let client_id_header: Arc<std::sync::Mutex<Option<String>>> =
                Arc::new(std::sync::Mutex::new(None));
            let header_ref = Arc::clone(&client_id_header);
            let callback = move |req: &Request, resp: Response| -> Result<Response, _> {
                if let Some(val) = req.headers().get("x-client-id") {
                    if let Ok(s) = val.to_str() {
                        *header_ref.lock().unwrap() = Some(s.to_string());
                    }
                }
                Ok(resp)
            };

            let ws_stream = match accept_hdr_async(stream, callback).await {
                Ok(ws) => ws,
                Err(e) => {
                    log_error(&format!("WebSocket handshake failed: {}", e), quiet);
                    return;
                }
            };

            let client_id_header = client_id_header.lock().unwrap().take();

            if persist {
                log(
                    &format!(
                        "New WebSocket connection {}",
                        client_id_header
                            .as_ref()
                            .map(|id| format!("(X-Client-Id: {})", id))
                            .unwrap_or_else(|| "(no X-Client-Id header)".to_string())
                    ),
                    quiet,
                );
                handle_persist_connection(
                    ws_stream,
                    &command,
                    &clients,
                    client_id_header,
                    grace_period_ms,
                    framing,
                    quiet,
                )
                .await;
            } else {
                handle_simple_connection(ws_stream, &command, framing, quiet).await;
            }
        });
    }
}

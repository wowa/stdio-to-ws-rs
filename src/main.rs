use clap::Parser;
use colored::Colorize;
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

#[derive(Clone, Copy, clap::ValueEnum)]
enum FramingMode {
    /// Each stdout line becomes a WS message; newlines ensured on stdin
    Line,
    /// Raw chunks forwarded as-is; Content-Length headers stripped
    Raw,
}

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

fn strip_content_length(s: &str) -> &str {
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

fn pretty_print(direction: &str, message: &str, quiet: bool) {
    if quiet {
        return;
    }
    let label = if direction.contains("Client") {
        format!("{}", direction.cyan())
    } else {
        format!("{}", direction.green())
    };
    match serde_json::from_str::<serde_json::Value>(message) {
        Ok(val) => {
            eprintln!("{}: {}", label, serde_json::to_string_pretty(&val).unwrap());
        }
        Err(_) => {
            eprintln!("{}: {}", label, message);
        }
    }
}

fn log(msg: &str, quiet: bool) {
    if !quiet {
        eprintln!("{} {}", "[stdio-to-ws]".bold(), msg);
    }
}

fn log_error(msg: &str, quiet: bool) {
    if !quiet {
        eprintln!("{} {}", "[stdio-to-ws]".red().bold(), msg);
    }
}

/// A persistent client: keeps the child process alive across WS reconnections.
struct PersistentClient {
    id: String,
    /// Send messages to the child's stdin
    stdin_tx: mpsc::Sender<String>,
    /// Receive stdout data from the child (each new WS conn gets a receiver)
    stdout_tx: tokio::sync::broadcast::Sender<String>,
    /// Buffered messages while no WS is connected
    buffer: Vec<String>,
    /// Cancel handle for grace period timer
    grace_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    /// Notify the currently connected WS to close (eviction)
    evict_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

type ClientRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<PersistentClient>>>>>;

async fn spawn_persistent_child(
    command: &[String],
    client_id: &str,
    framing: FramingMode,
    quiet: bool,
) -> Option<(
    mpsc::Sender<String>,
    tokio::sync::broadcast::Sender<String>,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
)> {
    let mut child = match Command::new(&command[0])
        .args(&command[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log_error(&format!("Failed to spawn child for {}: {}", client_id, e), quiet);
            return None;
        }
    };

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // stdin channel: WS -> child
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(256);
    let stdin_task = tokio::spawn(async move {
        while let Some(msg) = stdin_rx.recv().await {
            match framing {
                FramingMode::Line => {
                    let line = if msg.ends_with('\n') || msg.ends_with("\r\n") {
                        msg
                    } else {
                        format!("{}\n", msg)
                    };
                    if stdin.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                }
                FramingMode::Raw => {
                    if stdin.write_all(msg.as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
            let _ = stdin.flush().await;
        }
        drop(stdin);
        let _ = child.wait().await;
    });

    // stdout broadcast: child -> all connected WS
    let (stdout_tx, _) = tokio::sync::broadcast::channel::<String>(256);
    let stdout_tx2 = stdout_tx.clone();
    let stdout_task = tokio::spawn(async move {
        match framing {
            FramingMode::Line => {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            let trimmed = line.trim_end();
                            if trimmed.is_empty() {
                                continue;
                            }
                            pretty_print("[Server → Client]", trimmed, quiet);
                            let _ = stdout_tx2.send(trimmed.to_string());
                        }
                        Err(_) => break,
                    }
                }
            }
            FramingMode::Raw => {
                let mut reader = stdout;
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            let raw = String::from_utf8_lossy(&buf[..n]);
                            let content = strip_content_length(raw.trim_end()).to_string();
                            pretty_print("[Server → Client]", &content, quiet);
                            let _ = stdout_tx2.send(content);
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });

    // stderr logging
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => log_error(&format!("Child stderr: {}", line.trim_end()), quiet),
                Err(_) => break,
            }
        }
    });

    Some((stdin_tx, stdout_tx, stdin_task, stdout_task))
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

async fn handle_simple_connection(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    command: &[String],
    framing: FramingMode,
    quiet: bool,
) {
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    let mut child = match Command::new(&command[0])
        .args(&command[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log_error(&format!("Failed to spawn child: {}", e), quiet);
            let _ = ws_sender.close().await;
            return;
        }
    };

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let stdout_task = tokio::spawn(async move {
        match framing {
            FramingMode::Line => {
                let mut reader = BufReader::new(stdout);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            let trimmed = line.trim_end();
                            if trimmed.is_empty() {
                                continue;
                            }
                            pretty_print("[Server → Client]", trimmed, quiet);
                            if ws_sender
                                .send(Message::Text(trimmed.to_string().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(e) => {
                            log_error(&format!("stdout read error: {}", e), quiet);
                            break;
                        }
                    }
                }
            }
            FramingMode::Raw => {
                let mut reader = stdout;
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            let raw = String::from_utf8_lossy(&buf[..n]);
                            let content = strip_content_length(raw.trim_end());
                            pretty_print("[Server → Client]", content, quiet);
                            if ws_sender
                                .send(Message::Text(content.to_string().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(e) => {
                            log_error(&format!("stdout read error: {}", e), quiet);
                            break;
                        }
                    }
                }
            }
        }
        let _ = ws_sender.close().await;
    });

    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => log_error(&format!("Child stderr: {}", line.trim_end()), quiet),
                Err(_) => break,
            }
        }
    });

    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                match framing {
                    FramingMode::Line => {
                        pretty_print("[Client → Server]", &text, quiet);
                        let line = if text.ends_with('\n') || text.ends_with("\r\n") {
                            text.to_string()
                        } else {
                            format!("{}\n", text)
                        };
                        if stdin.write_all(line.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                    FramingMode::Raw => {
                        let content = strip_content_length(&text);
                        pretty_print("[Client → Server]", content, quiet);
                        if stdin.write_all(content.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                }
                if stdin.flush().await.is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            _ => {}
        }
    }

    drop(stdin);
    let _ = stdout_task.await;
    let _ = stderr_task.await;
    let _ = child.kill().await;
    log("Connection closed", quiet);
}

async fn handle_persist_connection(
    ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    command: &[String],
    clients: &ClientRegistry,
    requested_id: Option<String>,
    grace_period_ms: i64,
    framing: FramingMode,
    quiet: bool,
) {
    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    // Check if reconnecting to existing client
    let existing = if let Some(ref req_id) = requested_id {
        let registry = clients.lock().await;
        registry.get(req_id).cloned()
    } else {
        None
    };

    if let Some(client_arc) = existing {
        // Reconnect path: X-Client-Id matched an existing session
        let mut client = client_arc.lock().await;
        log(&format!("Reconnecting to existing client {}", client.id), quiet);

        // Evict old connection if still active
        if let Some(evict) = client.evict_tx.take() {
            log(&format!("Evicting previous connection for client {}", client.id), quiet);
            let _ = evict.send(());
        }

        // Cancel grace period timer
        if let Some(cancel) = client.grace_cancel.take() {
            let _ = cancel.send(());
        }

        // Send reconnect confirmation
        let reconnect_msg =
            serde_json::json!({"type": "reconnect", "clientId": client.id}).to_string();
        let _ = ws_sender
            .send(Message::Text(reconnect_msg.into()))
            .await;

        // Flush buffered messages
        for msg in client.buffer.drain(..) {
            let _ = ws_sender.send(Message::Text(msg.into())).await;
        }

        // Create eviction channel for this connection
        let (evict_tx, mut evict_rx) = tokio::sync::oneshot::channel::<()>();
        client.evict_tx = Some(evict_tx);

        let stdin_tx = client.stdin_tx.clone();
        let mut stdout_rx = client.stdout_tx.subscribe();
        let client_id = client.id.clone();
        drop(client);

        // stdout broadcast -> WS
        let client_id2 = client_id.clone();
        let clients2 = Arc::clone(clients);
        let ws_send_task = tokio::spawn(async move {
            while let Ok(content) = stdout_rx.recv().await {
                if ws_sender
                    .send(Message::Text(content.into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            // WS send failed or broadcast closed — buffer remaining
            let registry = clients2.lock().await;
            if let Some(c) = registry.get(&client_id2) {
                let mut cl = c.lock().await;
                while let Ok(content) = stdout_rx.try_recv() {
                    cl.buffer.push(content);
                }
            }
        });

        // WS -> stdin (with eviction support)
        let mut evicted = false;
        loop {
            tokio::select! {
                msg = ws_receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            let content = match framing {
                                FramingMode::Line => text.to_string(),
                                FramingMode::Raw => strip_content_length(&text).to_string(),
                            };
                            pretty_print("[Client → Server]", &content, quiet);
                            if stdin_tx.send(content).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                        _ => {}
                    }
                }
                _ = &mut evict_rx => {
                    log(&format!("Connection evicted for client {}", client_id), quiet);
                    evicted = true;
                    break;
                }
            }
        }

        ws_send_task.abort();
        if !evicted {
            start_grace_period(clients, &client_id, grace_period_ms, quiet).await;
        }
    } else {
        // New client path
        // If client provided X-Client-Id, use it; otherwise generate one and notify
        let send_connected_msg = requested_id.is_none();
        let client_id = requested_id.unwrap_or_else(|| Uuid::new_v4().to_string());

        let Some((stdin_tx, stdout_tx, _stdin_task, _stdout_task)) =
            spawn_persistent_child(command, &client_id, framing, quiet).await
        else {
            let _ = ws_sender.close().await;
            return;
        };

        log(&format!("Created new client {}", client_id), quiet);

        // Only send connected message when server generated the ID
        if send_connected_msg {
            let connected_msg =
                serde_json::json!({"type": "connected", "clientId": client_id}).to_string();
            let _ = ws_sender
                .send(Message::Text(connected_msg.into()))
                .await;
        }

        // Create eviction channel for this connection
        let (evict_tx, mut evict_rx) = tokio::sync::oneshot::channel::<()>();

        let client = PersistentClient {
            id: client_id.clone(),
            stdin_tx: stdin_tx.clone(),
            stdout_tx: stdout_tx.clone(),
            buffer: Vec::new(),
            grace_cancel: None,
            evict_tx: Some(evict_tx),
        };
        let client_arc = Arc::new(Mutex::new(client));
        {
            let mut registry = clients.lock().await;
            registry.insert(client_id.clone(), Arc::clone(&client_arc));
        }

        let mut stdout_rx = stdout_tx.subscribe();

        // stdout broadcast -> WS
        let client_id2 = client_id.clone();
        let clients2 = Arc::clone(clients);
        let ws_send_task = tokio::spawn(async move {
            while let Ok(content) = stdout_rx.recv().await {
                if ws_sender
                    .send(Message::Text(content.into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            let registry = clients2.lock().await;
            if let Some(c) = registry.get(&client_id2) {
                let mut cl = c.lock().await;
                while let Ok(content) = stdout_rx.try_recv() {
                    cl.buffer.push(content);
                }
            }
        });

        // WS -> stdin (with eviction support)
        let mut evicted = false;
        loop {
            tokio::select! {
                msg = ws_receiver.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            let content = match framing {
                                FramingMode::Line => text.to_string(),
                                FramingMode::Raw => strip_content_length(&text).to_string(),
                            };
                            pretty_print("[Client → Server]", &content, quiet);
                            if stdin_tx.send(content).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                        _ => {}
                    }
                }
                _ = &mut evict_rx => {
                    log(&format!("Connection evicted for client {}", client_id), quiet);
                    evicted = true;
                    break;
                }
            }
        }

        ws_send_task.abort();
        if !evicted {
            start_grace_period(clients, &client_id, grace_period_ms, quiet).await;
        }
    }
}

async fn start_grace_period(
    clients: &ClientRegistry,
    client_id: &str,
    grace_period_ms: i64,
    quiet: bool,
) {
    let is_infinite = grace_period_ms == -1;
    log(
        &format!(
            "WebSocket closed for client {}{}",
            client_id,
            if is_infinite {
                " (infinite persistence)"
            } else {
                ", starting grace period"
            }
        ),
        quiet,
    );

    if is_infinite {
        return;
    }

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

    // Store cancel handle
    {
        let registry = clients.lock().await;
        if let Some(c) = registry.get(client_id) {
            let mut cl = c.lock().await;
            cl.grace_cancel = Some(cancel_tx);
        }
    }

    let clients = Arc::clone(clients);
    let client_id = client_id.to_string();
    let duration = std::time::Duration::from_millis(grace_period_ms as u64);

    tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(duration) => {
                // Grace period expired — cleanup
                log(&format!("Cleaning up client {}", client_id), quiet);
                let mut registry = clients.lock().await;
                if let Some(client_arc) = registry.remove(&client_id) {
                    let client = client_arc.lock().await;
                    drop(client);
                }
            }
            _ = cancel_rx => {
                // Reconnected — cancel cleanup
            }
        }
    });
}

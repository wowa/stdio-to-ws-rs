use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_tungstenite::tungstenite::Message;

use crate::log::{log, log_error, pretty_print};
use crate::{strip_content_length, FramingMode};

pub async fn handle_simple_connection(
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

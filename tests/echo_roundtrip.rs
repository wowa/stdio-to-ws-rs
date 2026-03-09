mod common;

use common::{recv_timeout, send_recv, ws_connect, TestServer};
use std::time::Duration;

/// Goal: Verify plain text is echoed back verbatim
/// Feature: Echo roundtrip in raw mode
/// Setup: Server runs `cat` with `-q -f raw`
#[tokio::test]
async fn test_plain_text_echo() {
    let server = TestServer::new(&["cat", "-q", "-f", "raw"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response = send_recv(&mut sink, &mut stream, "hello world").await;
    assert_eq!(response, "hello world");
}

/// Goal: Verify a JSON object is echoed back unchanged
/// Feature: Echo roundtrip in raw mode
/// Setup: Server runs `cat` with `-q -f raw`
#[tokio::test]
async fn test_json_echo() {
    let server = TestServer::new(&["cat", "-q", "-f", "raw"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let msg = r#"{"jsonrpc":"2.0","id":1}"#;
    let response = send_recv(&mut sink, &mut stream, msg).await;
    assert_eq!(response, msg);
}

/// Goal: Verify nested JSON with arrays is echoed back unchanged
/// Feature: Echo roundtrip in raw mode
/// Setup: Server runs `cat` with `-q -f raw`
#[tokio::test]
async fn test_nested_json_echo() {
    let server = TestServer::new(&["cat", "-q", "-f", "raw"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let msg = r#"{"a":{"b":[1,2,3]}}"#;
    let response = send_recv(&mut sink, &mut stream, msg).await;
    assert_eq!(response, msg);
}

/// Goal: Verify that sending an empty string produces no response
/// Feature: Echo roundtrip in raw mode
/// Setup: Server runs `cat` with `-q -f raw`
#[tokio::test]
async fn test_empty_line_echo() {
    let server = TestServer::new(&["cat", "-q", "-f", "raw"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;

    sink.send(Message::Text("".to_string().into()))
        .await
        .expect("send failed");

    let response = recv_timeout(&mut stream, Duration::from_millis(500)).await;
    assert!(
        response.is_none(),
        "expected no response for empty string, got: {:?}",
        response
    );
}

/// Goal: Verify a long message (1000 chars) is echoed back completely
/// Feature: Echo roundtrip in raw mode
/// Setup: Server runs `cat` with `-q -f raw`
#[tokio::test]
async fn test_long_message_echo() {
    let server = TestServer::new(&["cat", "-q", "-f", "raw"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let msg = "a".repeat(1000);
    let response = send_recv(&mut sink, &mut stream, &msg).await;
    assert_eq!(response, msg);
}

/// Goal: Verify special characters are echoed back without escaping or mangling
/// Feature: Echo roundtrip in raw mode
/// Setup: Server runs `cat` with `-q -f raw`
#[tokio::test]
async fn test_special_chars_echo() {
    let server = TestServer::new(&["cat", "-q", "-f", "raw"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let msg = r#"hello "world" & <foo> bar"#;
    let response = send_recv(&mut sink, &mut stream, msg).await;
    assert_eq!(response, msg);
}

/// Goal: Verify unicode characters including emoji are echoed back correctly
/// Feature: Echo roundtrip in raw mode
/// Setup: Server runs `cat` with `-q -f raw`
#[tokio::test]
async fn test_unicode_echo() {
    let server = TestServer::new(&["cat", "-q", "-f", "raw"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let msg = "Hello 世界 🌍";
    let response = send_recv(&mut sink, &mut stream, msg).await;
    assert_eq!(response, msg);
}

/// Goal: Verify a command that produces output on its own (echo) delivers it to the client
/// Feature: Echo roundtrip in raw mode
/// Setup: Server runs `echo hello-from-echo` with `-q -f raw`
#[tokio::test]
async fn test_command_with_args() {
    let server = TestServer::new(&["echo hello-from-echo", "-q", "-f", "raw"]);
    let (_sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response = recv_timeout(&mut stream, Duration::from_secs(5)).await;
    let trimmed = response.as_deref().map(|s| s.trim_end());
    assert_eq!(
        trimmed,
        Some("hello-from-echo"),
        "expected echo output"
    );
}

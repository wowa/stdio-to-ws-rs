mod common;

use common::{recv_timeout, send_recv, ws_connect, ws_connect_with_header, TestServer};
use std::process::Stdio;
use std::time::Duration;

/// Goal: Verify server sends a connected message with a generated clientId
/// Feature: Persist mode auto-generated client ID
/// Setup: Server runs `cat` with `--persist -q`
#[tokio::test]
async fn test_connected_message() {
    let server = TestServer::new(&["cat", "--persist", "-q"]);
    let (_sink, mut stream) = ws_connect(&server.ws_url()).await;

    let msg = recv_timeout(&mut stream, Duration::from_secs(5))
        .await
        .expect("expected connected message");

    let json: serde_json::Value = serde_json::from_str(&msg).expect("not valid JSON");
    assert_eq!(json["type"], "connected");
    assert!(
        json["clientId"].as_str().map_or(false, |s| !s.is_empty()),
        "clientId should be a non-empty string, got: {:?}",
        json["clientId"]
    );
}

/// Goal: Verify echo works in persist mode after receiving the connected message
/// Feature: Persist mode basic echo
/// Setup: Server runs `cat` with `--persist -q`
#[tokio::test]
async fn test_persist_echo() {
    let server = TestServer::new(&["cat", "--persist", "-q"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    // Skip the connected message
    let _ = recv_timeout(&mut stream, Duration::from_secs(5)).await;

    let response = send_recv(&mut sink, &mut stream, "persist-test").await;
    assert_eq!(response, "persist-test");
}

/// Goal: Verify reconnection with the same client ID produces a reconnect message
/// Feature: Persist mode reconnection
/// Setup: Server runs `cat` with `--persist --grace-period 10 -q`
#[tokio::test]
async fn test_reconnect_with_client_id() {
    let server = TestServer::new(&["cat", "--persist", "--grace-period", "10", "-q"]);
    let url = server.ws_url();

    // First connection: get the client ID
    let (_sink, mut stream) = ws_connect(&url).await;
    let msg = recv_timeout(&mut stream, Duration::from_secs(5))
        .await
        .expect("expected connected message");
    let json: serde_json::Value = serde_json::from_str(&msg).expect("not valid JSON");
    let client_id = json["clientId"].as_str().unwrap().to_string();

    // Drop first connection
    drop(_sink);
    drop(stream);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Reconnect with X-Client-Id header
    let (_sink2, mut stream2) =
        ws_connect_with_header(&url, "X-Client-Id", &client_id).await;

    let reconnect_msg = recv_timeout(&mut stream2, Duration::from_secs(5))
        .await
        .expect("expected reconnect message");
    let reconnect_json: serde_json::Value =
        serde_json::from_str(&reconnect_msg).expect("not valid JSON");
    assert_eq!(reconnect_json["type"], "reconnect");
    assert_eq!(reconnect_json["clientId"], client_id);
}

/// Goal: Verify the child process stays alive during the grace period
/// Feature: Persist mode grace period keeping process alive
/// Setup: Server runs `cat` with `--persist --grace-period 5 -q`
#[tokio::test]
async fn test_grace_period_keeps_alive() {
    let server = TestServer::new(&["cat", "--persist", "--grace-period", "5", "-q"]);
    let url = server.ws_url();

    // Connect and get client ID
    let (_sink, mut stream) = ws_connect(&url).await;
    let msg = recv_timeout(&mut stream, Duration::from_secs(5))
        .await
        .expect("expected connected message");
    let json: serde_json::Value = serde_json::from_str(&msg).expect("not valid JSON");
    let client_id = json["clientId"].as_str().unwrap().to_string();

    // Disconnect
    drop(_sink);
    drop(stream);
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Reconnect within grace period
    let (_sink2, mut stream2) =
        ws_connect_with_header(&url, "X-Client-Id", &client_id).await;

    let reconnect_msg = recv_timeout(&mut stream2, Duration::from_secs(5))
        .await
        .expect("expected reconnect message");
    let reconnect_json: serde_json::Value =
        serde_json::from_str(&reconnect_msg).expect("not valid JSON");
    assert_eq!(reconnect_json["type"], "reconnect");
}

/// Goal: Verify infinite grace period (-1) keeps the child alive indefinitely
/// Feature: Persist mode infinite grace period
/// Setup: Server runs `cat` with `--persist --grace-period -1` and piped stderr
#[tokio::test]
async fn test_infinite_grace_period() {
    let mut server =
        TestServer::new_with_stderr(&["cat", "--persist", "--grace-period", "-1"], Stdio::piped());
    let url = server.ws_url();

    // Check stderr mentions "infinite"
    // Give the server a moment to print its startup message
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connect and get client ID
    let (_sink, mut stream) = ws_connect(&url).await;
    let msg = recv_timeout(&mut stream, Duration::from_secs(5))
        .await
        .expect("expected connected message");
    let json: serde_json::Value = serde_json::from_str(&msg).expect("not valid JSON");
    let client_id = json["clientId"].as_str().unwrap().to_string();

    // Disconnect
    drop(_sink);
    drop(stream);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Reconnect
    let (_sink2, mut stream2) =
        ws_connect_with_header(&url, "X-Client-Id", &client_id).await;

    let reconnect_msg = recv_timeout(&mut stream2, Duration::from_secs(5))
        .await
        .expect("expected reconnect message");
    let reconnect_json: serde_json::Value =
        serde_json::from_str(&reconnect_msg).expect("not valid JSON");
    assert_eq!(reconnect_json["type"], "reconnect");

    // Verify stderr contains "infinite"
    drop(_sink2);
    drop(stream2);
    let stderr = server.kill_and_get_stderr();
    let stderr_str = String::from_utf8_lossy(&stderr);
    assert!(
        stderr_str.to_lowercase().contains("infinite"),
        "expected stderr to mention 'infinite', got: {}",
        stderr_str
    );
}

/// Goal: Verify that without --persist, no connected message is sent
/// Feature: Non-persist mode baseline behavior
/// Setup: Server runs `cat` with `-q` (no --persist)
#[tokio::test]
async fn test_no_persist_baseline() {
    let server = TestServer::new(&["cat", "-q"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response = send_recv(&mut sink, &mut stream, "plain").await;
    assert_eq!(response, "plain", "first message should be the echo, not a connected message");
}

/// Goal: Verify the child process stays alive across disconnect/reconnect and can still echo
/// Feature: Persist mode process persistence
/// Setup: Server runs `cat` with `--persist --grace-period 10 -q`
#[tokio::test]
async fn test_process_survives_reconnect() {
    let server = TestServer::new(&["cat", "--persist", "--grace-period", "10", "-q"]);
    let url = server.ws_url();

    // First connection: get client ID, send data, verify echo
    let (mut sink, mut stream) = ws_connect(&url).await;
    let connected_msg = recv_timeout(&mut stream, Duration::from_secs(5))
        .await
        .expect("expected connected message");
    let json: serde_json::Value = serde_json::from_str(&connected_msg).expect("not valid JSON");
    let client_id = json["clientId"].as_str().unwrap().to_string();

    let echo = send_recv(&mut sink, &mut stream, "before-disconnect").await;
    assert_eq!(echo, "before-disconnect");

    // Disconnect
    drop(sink);
    drop(stream);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Reconnect and verify process is still alive by sending new data
    let (mut sink2, mut stream2) =
        ws_connect_with_header(&url, "X-Client-Id", &client_id).await;

    let reconnect_msg = recv_timeout(&mut stream2, Duration::from_secs(5))
        .await
        .expect("expected reconnect message");
    let reconnect_json: serde_json::Value =
        serde_json::from_str(&reconnect_msg).expect("not valid JSON");
    assert_eq!(reconnect_json["type"], "reconnect");
    assert_eq!(reconnect_json["clientId"], client_id);

    // Child process should still be alive — cat can echo new input
    let echo2 = send_recv(&mut sink2, &mut stream2, "after-reconnect").await;
    assert_eq!(echo2, "after-reconnect");
}

/// Goal: Verify client-provided X-Client-Id skips the connected message
/// Feature: Persist mode client-provided ID
/// Setup: Server runs `cat` with `--persist -q`
#[tokio::test]
async fn test_client_provided_id() {
    let server = TestServer::new(&["cat", "--persist", "-q"]);

    let (mut sink, mut stream) =
        ws_connect_with_header(&server.ws_url(), "X-Client-Id", "my-client-42").await;

    // Should NOT get a connected message — echo works directly
    let response = send_recv(&mut sink, &mut stream, "hello").await;
    assert_eq!(
        response, "hello",
        "first message should be echo, not a connected message"
    );
}

/// Goal: Verify that a new connection with the same client ID evicts the old one
/// Feature: Persist mode eviction
/// Setup: Server runs `cat` with `--persist --grace-period 30 -q`
#[tokio::test]
async fn test_eviction() {
    let server = TestServer::new(&["cat", "--persist", "--grace-period", "30", "-q"]);
    let url = server.ws_url();

    // ws1 connects with a known client ID
    let (mut sink1, mut stream1) =
        ws_connect_with_header(&url, "X-Client-Id", "evict-me").await;

    // ws1 echo works
    let resp1 = send_recv(&mut sink1, &mut stream1, "from-ws1").await;
    assert_eq!(resp1, "from-ws1");

    // ws2 connects with the same client ID — should evict ws1
    let (mut sink2, mut stream2) =
        ws_connect_with_header(&url, "X-Client-Id", "evict-me").await;

    // ws2 gets reconnect message
    let reconnect_msg = recv_timeout(&mut stream2, Duration::from_secs(5))
        .await
        .expect("expected reconnect message on ws2");
    let reconnect_json: serde_json::Value =
        serde_json::from_str(&reconnect_msg).expect("not valid JSON");
    assert_eq!(reconnect_json["type"], "reconnect");

    // ws1 should be closed
    let ws1_msg = recv_timeout(&mut stream1, Duration::from_secs(2)).await;
    assert!(
        ws1_msg.is_none(),
        "expected ws1 to be closed after eviction, got: {:?}",
        ws1_msg
    );

    // ws2 echo works
    let resp2 = send_recv(&mut sink2, &mut stream2, "from-ws2").await;
    assert_eq!(resp2, "from-ws2");
}

/// Goal: Verify persist mode works with line framing
/// Feature: Persist mode with line framing
/// Setup: Server runs `cat` with `--persist -q -f line`
#[tokio::test]
async fn test_persist_with_line_framing() {
    let server = TestServer::new(&["cat", "--persist", "-q", "-f", "line"]);

    let (mut sink, mut stream) =
        ws_connect_with_header(&server.ws_url(), "X-Client-Id", "line-persist").await;

    let response = send_recv(&mut sink, &mut stream, "persist-line-test").await;
    assert_eq!(response, "persist-line-test");
}

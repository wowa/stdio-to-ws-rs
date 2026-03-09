mod common;

use common::{recv_timeout, send_recv, ws_connect, TestServer};
use futures_util::SinkExt;
use std::time::Duration;

/// Goal: Verify single message echo works in line framing mode
/// Feature: Line framing mode
/// Setup: Server runs `cat` with `-q` (quiet), default line framing
#[tokio::test]
async fn test_line_mode_single_echo() {
    let server = TestServer::new(&["cat", "-q"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response = send_recv(&mut sink, &mut stream, "hello").await;
    assert_eq!(response, "hello");
}

/// Goal: Verify line mode appends \n to stdin so cat can echo it back
/// Feature: Line framing mode
/// Setup: Server runs `cat` with `-q`, input has no trailing newline
#[tokio::test]
async fn test_line_mode_appends_newline() {
    let server = TestServer::new(&["cat", "-q"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response = send_recv(&mut sink, &mut stream, "no-newline").await;
    assert_eq!(response, "no-newline");
}

/// Goal: Verify line mode skips empty lines in stdout
/// Feature: Line framing mode
/// Setup: Server runs python3 that prints "a", an empty line, then "b"
#[tokio::test]
async fn test_line_mode_skips_empty_lines() {
    let server = TestServer::new(&[
        r#"python3 -c "print('a'); print(''); print('b')""#,
        "-q",
    ]);
    let (mut _sink, mut stream) = ws_connect(&server.ws_url()).await;

    let msg1 = recv_timeout(&mut stream, Duration::from_secs(5))
        .await
        .expect("expected first message 'a'");
    assert_eq!(msg1, "a");

    let msg2 = recv_timeout(&mut stream, Duration::from_secs(5))
        .await
        .expect("expected second message 'b'");
    assert_eq!(msg2, "b");
}

/// Goal: Verify line mode delivers each stdout line as a separate WS message
/// Feature: Line framing mode
/// Setup: Server runs python3 that prints line0, line1, line2
#[tokio::test]
async fn test_line_mode_multi_line_output() {
    let server = TestServer::new(&[
        r#"python3 -c "for i in range(3): print(f'line{i}')""#,
        "-q",
    ]);
    let (mut _sink, mut stream) = ws_connect(&server.ws_url()).await;

    for i in 0..3 {
        let msg = recv_timeout(&mut stream, Duration::from_secs(5))
            .await
            .unwrap_or_else(|| panic!("expected message 'line{}'", i));
        assert_eq!(msg, format!("line{}", i));
    }
}

/// Goal: Verify raw mode forwards data without line splitting
/// Feature: Raw framing mode
/// Setup: Server runs `cat` with `-q -f raw`
#[tokio::test]
async fn test_raw_mode_no_line_split() {
    let server = TestServer::new(&["cat", "-q", "-f", "raw"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    sink.send(tokio_tungstenite::tungstenite::Message::Text(
        "raw-test".to_string().into(),
    ))
    .await
    .expect("send failed");

    let response = recv_timeout(&mut stream, Duration::from_secs(5))
        .await
        .expect("expected response containing 'raw-test'");
    assert!(
        response.contains("raw-test"),
        "expected response to contain 'raw-test', got: {response}"
    );
}

/// Goal: Verify that line framing is the default when no -f flag is provided
/// Feature: Line framing mode
/// Setup: Server runs `cat` with `-q` only (no explicit -f flag)
#[tokio::test]
async fn test_default_framing_is_line() {
    let server = TestServer::new(&["cat", "-q"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response = send_recv(&mut sink, &mut stream, "default-framing").await;
    assert_eq!(response, "default-framing");
}

mod common;

use common::{send_recv, ws_connect, TestServer};
use std::process::Stdio;
use std::time::Duration;

/// Goal: Verify that quiet mode suppresses all stderr output
/// Feature: Quiet mode / logging
/// Setup: Server runs `cat` with `-q` flag and piped stderr
#[tokio::test]
async fn test_quiet_suppresses_stderr() {
    let mut server = TestServer::new_with_stderr(&["cat", "-q"], Stdio::piped());
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response = send_recv(&mut sink, &mut stream, "quiet-test").await;
    assert_eq!(response, "quiet-test");

    let stderr_bytes = server.kill_and_get_stderr();
    assert!(
        stderr_bytes.is_empty(),
        "expected no stderr output in quiet mode, got: {:?}",
        String::from_utf8_lossy(&stderr_bytes)
    );
}

/// Goal: Verify that without quiet mode the server logs to stderr
/// Feature: Quiet mode / logging
/// Setup: Server runs `cat` without `-q` flag and piped stderr
#[tokio::test]
async fn test_non_quiet_logs_to_stderr() {
    let mut server = TestServer::new_with_stderr(&["cat"], Stdio::piped());
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response = send_recv(&mut sink, &mut stream, "noisy-test").await;
    assert_eq!(response, "noisy-test");

    // Allow time for log output to flush
    tokio::time::sleep(Duration::from_millis(300)).await;

    let stderr_bytes = server.kill_and_get_stderr();
    let stderr_str = String::from_utf8_lossy(&stderr_bytes);
    assert!(
        stderr_str.contains("stdio-to-ws"),
        "expected stderr to contain 'stdio-to-ws', got: {:?}",
        stderr_str
    );
}

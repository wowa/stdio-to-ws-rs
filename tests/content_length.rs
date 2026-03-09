mod common;

use common::{recv_timeout, send_recv, ws_connect, TestServer};
use std::time::Duration;

/// Goal: Verify Content-Length prefix is stripped on the WS→stdin path (and stdout→WS path)
/// Feature: Content-Length stripping
/// Setup: Server runs `cat` with `-q -f raw`
#[tokio::test]
async fn test_strip_cl_ws_to_stdin() {
    let server = TestServer::new(&["cat", "-q", "-f", "raw"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response = send_recv(&mut sink, &mut stream, "Content-Length: 8\r\n\r\n{\"id\":1}").await;
    assert_eq!(
        response, "{\"id\":1}",
        "Expected Content-Length prefix to be stripped; got: {response}"
    );
}

/// Goal: Verify Content-Length prefix is stripped on the stdout→WS path
/// Feature: Content-Length stripping
/// Setup: Server runs `python3 -c` printing a CL-prefixed message, with `-q -f raw`
#[tokio::test]
async fn test_strip_cl_stdout_to_ws() {
    let server = TestServer::new(&[
        "python3 -c \"print('Content-Length: 5\\r\\n\\r\\nhello')\"",
        "-q",
        "-f",
        "raw",
    ]);
    let (_, mut stream) = ws_connect(&server.ws_url()).await;

    let response = recv_timeout(&mut stream, Duration::from_secs(5))
        .await
        .expect("expected a message from stdout");
    assert_eq!(
        response, "hello",
        "Expected Content-Length prefix to be stripped from stdout; got: {response}"
    );
}

/// Goal: Verify Content-Length prefix is NOT stripped in line mode
/// Feature: Content-Length stripping
/// Setup: Server runs `cat` with `-q -f line`
#[tokio::test]
async fn test_no_strip_in_line_mode() {
    let server = TestServer::new(&["cat", "-q", "-f", "line"]);
    let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;

    let response =
        send_recv(&mut sink, &mut stream, "Content-Length: 8\r\n\r\n{\"id\":1}").await;
    assert!(
        response.contains("Content-Length"),
        "In line mode, Content-Length should be preserved; got: {response}"
    );
}

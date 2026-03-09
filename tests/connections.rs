mod common;

use common::{send_recv, ws_connect, TestServer};

/// Goal: Verify the server handles sequential connect/send/recv/close cycles correctly
/// Feature: Connection handling
/// Setup: Server runs `cat` with `-q`
#[tokio::test]
async fn test_sequential_connections() {
    let server = TestServer::new(&["cat", "-q"]);

    for (i, msg) in ["seq1", "seq2", "seq3"].iter().enumerate() {
        let (mut sink, mut stream) = ws_connect(&server.ws_url()).await;
        let response = send_recv(&mut sink, &mut stream, msg).await;
        assert_eq!(
            response, *msg,
            "sequential connection {} echoed wrong value",
            i + 1
        );
        drop(sink);
        drop(stream);
    }
}

/// Goal: Verify multiple simultaneous connections each get their own isolated child process
/// Feature: Connection handling
/// Setup: Server runs `cat` with `-q`
#[tokio::test]
async fn test_concurrent_connections() {
    let server = TestServer::new(&["cat", "-q"]);

    let url = server.ws_url();
    let (mut sink1, mut stream1) = ws_connect(&url).await;
    // Small delay to let the server finish processing the first handshake
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let (mut sink2, mut stream2) = ws_connect(&url).await;

    let (resp1, resp2) = tokio::join!(
        send_recv(&mut sink1, &mut stream1, "client-one"),
        send_recv(&mut sink2, &mut stream2, "client-two"),
    );

    assert_eq!(resp1, "client-one");
    assert_eq!(resp2, "client-two");
}

/// Goal: Verify the server accepts connections when bound to 0.0.0.0
/// Feature: Connection handling
/// Setup: Server runs `cat` with `-q -b 0.0.0.0`
#[tokio::test]
async fn test_custom_bind_address() {
    let server = TestServer::new(&["cat", "-q", "-b", "0.0.0.0"]);

    let url = format!("ws://127.0.0.1:{}", server.port);
    let (mut sink, mut stream) = ws_connect(&url).await;

    let response = send_recv(&mut sink, &mut stream, "bound-test").await;
    assert_eq!(response, "bound-test");
}

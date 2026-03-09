// Shared test infrastructure: TestServer, port allocation, WS client helpers.
#![allow(dead_code)]

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::net::TcpStream as TokioTcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TokioTcpStream>>, Message>;
pub type WsStream = SplitStream<WebSocketStream<MaybeTlsStream<TokioTcpStream>>>;

/// Manages a stdio-to-ws server process for testing.
/// Automatically kills the child on drop.
pub struct TestServer {
    child: Child,
    pub port: u16,
}

impl TestServer {
    /// Spawn the binary with the given arguments.
    /// Waits until the TCP port is accepting connections.
    pub fn new(args: &[&str]) -> Self {
        Self::new_with_stderr(args, Stdio::null())
    }

    /// Spawn with explicit stderr handling (for quiet/logging tests).
    pub fn new_with_stderr(args: &[&str], stderr: Stdio) -> Self {
        let port = portpicker::pick_unused_port().expect("no free port");
        let bin = env!("CARGO_BIN_EXE_stdio-to-ws");

        // Build argument list, injecting -p <port>
        let mut cmd_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        cmd_args.push("-p".to_string());
        cmd_args.push(port.to_string());

        let child = Command::new(bin)
            .args(&cmd_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .expect("failed to spawn stdio-to-ws");

        // Wait for server to be ready (poll TCP connect)
        let addr = format!("127.0.0.1:{}", port);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if TcpStream::connect(&addr).is_ok() {
                return TestServer { child, port };
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("server did not become ready on port {}", port);
    }

    /// WebSocket URL for this server.
    pub fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    /// Kill the server and return captured stderr bytes (if stderr was piped).
    pub fn kill_and_get_stderr(&mut self) -> Vec<u8> {
        use std::io::Read;
        let _ = self.child.kill();
        let mut stderr_bytes = Vec::new();
        if let Some(mut stderr) = self.child.stderr.take() {
            let _ = stderr.read_to_end(&mut stderr_bytes);
        }
        let _ = self.child.wait();
        stderr_bytes
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Connect to a WebSocket URL. Returns (sink, stream).
pub async fn ws_connect(url: &str) -> (WsSink, WsStream) {
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect failed");
    ws.split()
}

/// Connect with a custom header (e.g. X-Client-Id).
pub async fn ws_connect_with_header(
    url: &str,
    header_name: &str,
    header_value: &str,
) -> (WsSink, WsStream) {
    let uri: http::Uri = url.parse().expect("bad uri");
    let host = uri.host().unwrap_or("127.0.0.1");

    let request = http::Request::builder()
        .uri(url)
        .header("Host", host)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        )
        .header(header_name, header_value)
        .body(())
        .expect("failed to build request");

    let (ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("ws connect with header failed");
    ws.split()
}

/// Send a text message and receive one text response.
pub async fn send_recv(sink: &mut WsSink, stream: &mut WsStream, msg: &str) -> String {
    sink.send(Message::Text(msg.to_string().into()))
        .await
        .expect("send failed");
    recv_timeout(stream, Duration::from_secs(5))
        .await
        .expect("no response received")
}

/// Receive a single text message with timeout. Returns None on timeout or close.
pub async fn recv_timeout(stream: &mut WsStream, duration: Duration) -> Option<String> {
    match tokio::time::timeout(duration, stream.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => Some(text.to_string()),
        _ => None,
    }
}

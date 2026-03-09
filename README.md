# stdio-to-ws

Rust port of [marimo-team/stdio-to-ws](https://github.com/marimo-team/stdio-to-ws), merging features from the [rebornix/stdio-to-ws](https://github.com/rebornix/stdio-to-ws) fork (persist mode, reconnection). Bridges a stdio subprocess to WebSocket connections.

Each WebSocket client gets its own child process. Messages sent over the WebSocket are forwarded to the child's stdin, and stdout is streamed back. JSON messages are pretty-printed in the server log.

Use cases include bridging LSP language servers, exposing ACP-compatible agents (e.g. `stdio-to-ws "npx @google/gemini-cli --experimental-acp"`), and any stdio-based service that needs a WebSocket interface.

## Install

```bash
cargo install --path .
```

## Usage

```bash
stdio-to-ws <command> [options]
```

### Examples

```bash
stdio-to-ws "cat" -p 3000
stdio-to-ws "python my-script.py" -p 8080
stdio-to-ws "my-command" -f raw                          # raw chunk framing (for LSP)
stdio-to-ws --persist --grace-period 60 "your-command"   # persist with 60s grace period
stdio-to-ws --persist --grace-period -1 "your-command"   # infinite persistence
stdio-to-ws -q "your-command"                            # quiet mode
```

## Options

| Flag | Description | Default |
|------|-------------|---------|
| `-p, --port <port>` | Port to listen on | `3000` |
| `-b, --bind <addr>` | Address to bind the server to | `0.0.0.0` |
| `-f, --framing <mode>` | Message framing mode: `line` or `raw` | `line` |
| `--persist` | Keep child process alive across WebSocket disconnections | off |
| `-g, --grace-period <secs>` | Seconds to wait before killing a disconnected child process. Use `-1` for infinite (never kill). Requires `--persist` | `30` |
| `-q, --quiet` | Suppress all logging output | off |

## Framing modes

### `line` (default)

Each line of stdout becomes a separate WebSocket message. Empty lines are skipped. Incoming WebSocket messages are written to stdin with a trailing newline appended if not already present.

Best for line-delimited protocols like NDJSON.

### `raw`

Stdout data is forwarded as raw chunks. LSP-style `Content-Length: N\r\n\r\n` headers are stripped from messages in both directions (WS to stdin and stdout to WS).

Best for LSP servers and similar binary-framed protocols.

## Persist mode

By default, each WebSocket connection spawns a new child process, and closing the connection kills the process.

With `--persist`, child processes survive WebSocket disconnections. Two client identification modes are supported:

### Server-assigned ID

As defined by the [rebornix/stdio-to-ws](https://github.com/rebornix/stdio-to-ws) fork. If no `X-Client-Id` header is sent, the server generates a UUID and sends `{"type": "connected", "clientId": "<uuid>"}` as the first message. The client saves this ID and includes it as `X-Client-Id` on subsequent connections to reconnect.

### Client-chosen ID

The client sends an `X-Client-Id` header with the initial WebSocket upgrade request. The server uses this ID to register the session directly — no protocol messages are injected into the data stream, keeping the wire clean for application data.

- **First connection** with a new ID: the server spawns a child process and registers the session.
- **Reconnection** with an existing ID: the server reattaches to the existing child process, sends `{"type": "reconnect", "clientId": "<id>"}`, and replays any buffered messages.
- **Eviction**: if a new connection arrives with the same ID as an active connection, the old connection is closed and the new one takes over.

### Grace period

When a client disconnects, a grace period timer starts. If no reconnection occurs within the grace period, the child process is killed. Use `--grace-period -1` to keep child processes alive indefinitely.

## License

[Apache-2.0](LICENSE)

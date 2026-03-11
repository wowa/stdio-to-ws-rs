use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::log::log_error;

pub async fn serve_static(mut stream: TcpStream, serve_dir: &str, quiet: bool) {
    let mut buf = vec![0u8; 8192];
    let n = match stream.read(&mut buf).await {
        Ok(0) => return,
        Ok(n) => n,
        Err(_) => return,
    };

    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let path = path.split('?').next().unwrap_or("/");

    let file_path = if path == "/" {
        format!("{}/index.html", serve_dir)
    } else {
        format!("{}{}", serve_dir, path)
    };

    // Security: prevent directory traversal
    let serve_canonical = match std::fs::canonicalize(serve_dir) {
        Ok(p) => p,
        Err(e) => {
            log_error(&format!("Cannot resolve serve dir: {}", e), quiet);
            send_response(&mut stream, 500, "text/plain", b"Internal Server Error").await;
            return;
        }
    };

    let canonical = match std::fs::canonicalize(&file_path) {
        Ok(p) => p,
        Err(_) => {
            send_response(&mut stream, 404, "text/plain", b"Not Found").await;
            return;
        }
    };

    if !canonical.starts_with(&serve_canonical) {
        send_response(&mut stream, 403, "text/plain", b"Forbidden").await;
        return;
    }

    match tokio::fs::read(&canonical).await {
        Ok(contents) => {
            let content_type = guess_content_type(canonical.to_str().unwrap_or(""));
            send_response(&mut stream, 200, content_type, &contents).await;
        }
        Err(_) => {
            send_response(&mut stream, 404, "text/plain", b"Not Found").await;
        }
    }
}

async fn send_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let status_text = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status, status_text, content_type,
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(body).await;
}

fn guess_content_type(path: &str) -> &str {
    if path.ends_with(".html") || path.ends_with(".htm") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".gif") {
        "image/gif"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else if path.ends_with(".woff") {
        "font/woff"
    } else if path.ends_with(".wasm") {
        "application/wasm"
    } else if path.ends_with(".txt") {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

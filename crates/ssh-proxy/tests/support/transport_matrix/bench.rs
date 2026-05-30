use std::{
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

use super::command::TcpMeasurement;

const BENCH_TIMEOUT: Duration = Duration::from_secs(180);
const CHUNK_SIZE: usize = 64 * 1024;

pub(super) const DEFAULT_PAYLOAD_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const DEFAULT_CONCURRENT_PAYLOAD_BYTES: u64 = 256 * 1024;

pub(super) const BENCH_SERVER_SCRIPT: &str = r#"#!/usr/bin/env python3
import socket
import sys
import threading
import time

CHUNK = (b"0123456789abcdef" * 4096)

def recv_exact(conn, count):
    remaining = count
    while remaining > 0:
        data = conn.recv(min(len(CHUNK), remaining))
        if not data:
            raise RuntimeError("client closed before payload completed")
        remaining -= len(data)

def handle(conn):
    with conn:
        line = b""
        while not line.endswith(b"\n") and len(line) < 128:
            data = conn.recv(1)
            if not data:
                return
            line += data
        parts = line.decode("ascii", "replace").strip().split()
        if not parts:
            return
        command = parts[0].upper()
        if command == "GET":
            remaining = int(parts[1])
            while remaining > 0:
                chunk = CHUNK[:min(len(CHUNK), remaining)]
                conn.sendall(chunk)
                remaining -= len(chunk)
        elif command == "PUT":
            count = int(parts[1])
            recv_exact(conn, count)
            conn.sendall(("OK %d\n" % count).encode("ascii"))
        elif command == "STREAM":
            seconds = max(1.0, float(parts[1]))
            chunk_size = int(parts[2]) if len(parts) > 2 else 4096
            chunk = CHUNK[:max(1, min(len(CHUNK), chunk_size))]
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                conn.sendall(chunk)
                time.sleep(0.1)
        else:
            conn.sendall(b"ERR unknown command\n")

def main():
    host = "127.0.0.1"
    port = int(sys.argv[1])
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
        server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.bind((host, port))
        server.listen(128)
        while True:
            conn, _ = server.accept()
            thread = threading.Thread(target=handle, args=(conn,), daemon=True)
            thread.start()

if __name__ == "__main__":
    main()
"#;

pub(super) fn bench_download_via_tcp(
    proxy: SocketAddr,
    expected_bytes: u64,
) -> Result<TcpMeasurement, String> {
    let started = Instant::now();
    let mut stream = connect(proxy)?;
    stream
        .write_all(format!("GET {expected_bytes}\n").as_bytes())
        .map_err(|err| format!("write bench download request: {err}"))?;

    let mut bytes = 0_u64;
    let mut first_byte_ms = None;
    let mut buf = vec![0_u8; CHUNK_SIZE];
    while bytes < expected_bytes {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if first_byte_ms.is_none() {
                    first_byte_ms = Some(started.elapsed().as_millis());
                }
                bytes += n as u64;
            }
            Err(err) if matches!(err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                return Err(format!("read bench download response: {err}"));
            }
            Err(err) => return Err(format!("read bench download response: {err}")),
        }
    }
    if bytes != expected_bytes {
        return Err(format!(
            "bench download byte mismatch: expected={expected_bytes} actual={bytes}"
        ));
    }
    Ok(measurement(
        format!("bench_download:{bytes}"),
        bytes,
        started.elapsed().as_millis(),
        first_byte_ms.unwrap_or_else(|| started.elapsed().as_millis()),
    ))
}

pub(super) fn bench_upload_via_tcp(
    proxy: SocketAddr,
    expected_bytes: u64,
) -> Result<TcpMeasurement, String> {
    let started = Instant::now();
    let mut stream = connect(proxy)?;
    stream
        .write_all(format!("PUT {expected_bytes}\n").as_bytes())
        .map_err(|err| format!("write bench upload request: {err}"))?;
    write_payload(&mut stream, expected_bytes)?;

    let mut response = Vec::new();
    let mut first_byte_ms = None;
    let mut buf = [0_u8; 128];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if first_byte_ms.is_none() {
                    first_byte_ms = Some(started.elapsed().as_millis());
                }
                response.extend_from_slice(&buf[..n]);
                if response.ends_with(b"\n") {
                    break;
                }
            }
            Err(err) if matches!(err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                return Err(format!("read bench upload ack: {err}"));
            }
            Err(err) => return Err(format!("read bench upload ack: {err}")),
        }
    }
    let response = String::from_utf8(response).map_err(|err| format!("utf8 upload ack: {err}"))?;
    let expected = format!("OK {expected_bytes}");
    if response.trim() != expected {
        return Err(format!(
            "bench upload ack mismatch: expected={expected:?} actual={:?}",
            response.trim()
        ));
    }
    Ok(measurement(
        response.trim().to_string(),
        expected_bytes,
        started.elapsed().as_millis(),
        first_byte_ms.unwrap_or_else(|| started.elapsed().as_millis()),
    ))
}

pub(super) fn bench_stream_via_tcp(
    proxy: SocketAddr,
    seconds: u64,
) -> Result<TcpMeasurement, String> {
    let started = Instant::now();
    let mut stream = connect(proxy)?;
    stream
        .write_all(format!("STREAM {} 4096\n", seconds.max(1)).as_bytes())
        .map_err(|err| format!("write bench stream request: {err}"))?;

    let mut bytes = 0_u64;
    let mut first_byte_ms = None;
    let mut buf = [0_u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if first_byte_ms.is_none() {
                    first_byte_ms = Some(started.elapsed().as_millis());
                }
                bytes += n as u64;
            }
            Err(err) if matches!(err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                if bytes == 0 {
                    return Err(format!("read bench stream response: {err}"));
                }
                break;
            }
            Err(err) => return Err(format!("read bench stream response: {err}")),
        }
    }
    if bytes == 0 {
        return Err("bench stream returned no bytes".to_string());
    }
    Ok(measurement(
        format!("bench_stream:{bytes}"),
        bytes,
        started.elapsed().as_millis(),
        first_byte_ms.unwrap_or_else(|| started.elapsed().as_millis()),
    ))
}

fn connect(proxy: SocketAddr) -> Result<TcpStream, String> {
    let stream = TcpStream::connect(proxy).map_err(|err| format!("connect proxy: {err}"))?;
    stream
        .set_read_timeout(Some(BENCH_TIMEOUT))
        .map_err(|err| format!("set read timeout: {err}"))?;
    stream
        .set_write_timeout(Some(BENCH_TIMEOUT))
        .map_err(|err| format!("set write timeout: {err}"))?;
    Ok(stream)
}

fn write_payload(stream: &mut TcpStream, expected_bytes: u64) -> Result<(), String> {
    let chunk = vec![b'x'; CHUNK_SIZE];
    let mut remaining = expected_bytes;
    while remaining > 0 {
        let n = remaining.min(chunk.len() as u64) as usize;
        stream
            .write_all(&chunk[..n])
            .map_err(|err| format!("write bench upload payload: {err}"))?;
        remaining -= n as u64;
    }
    Ok(())
}

fn measurement(
    response: String,
    bytes: u64,
    duration_ms: u128,
    first_byte_ms: u128,
) -> TcpMeasurement {
    TcpMeasurement {
        response,
        bytes,
        duration_ms: duration_ms.max(1),
        first_byte_ms,
        proxy_stderr: None,
    }
}

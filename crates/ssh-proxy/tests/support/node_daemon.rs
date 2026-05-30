#![allow(dead_code)]

use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral port");
    listener.local_addr().expect("read local addr")
}

pub fn ssh_proxy_command() -> Command {
    let home = temp_dir("cmd-home");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"));
    command.env("SSH_PROXY_HOME", home);
    command
}

pub fn run_control(endpoint: &str, args: &[&str]) -> std::process::Output {
    let mut command = ssh_proxy_command();
    command.args(["--log", "warn", "node", "control", "--endpoint", endpoint]);
    command.args(args);
    command.output().expect("run node control")
}

pub fn run_control_with_token(endpoint: &str, token: &str, args: &[&str]) -> std::process::Output {
    let mut command = ssh_proxy_command();
    command.args([
        "--log",
        "warn",
        "node",
        "control",
        "--endpoint",
        endpoint,
        "--token",
        token,
    ]);
    command.args(args);
    command.output().expect("run node control")
}

pub struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    pub fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn shutdown_with_control(mut self, endpoint: &str, token: Option<&str>) {
        self.request_shutdown(endpoint, token);
        self.wait_or_kill(Duration::from_secs(2));
    }

    fn request_shutdown(&self, endpoint: &str, token: Option<&str>) {
        let _ = match token {
            Some(token) => run_control_with_token(endpoint, token, &["shutdown"]),
            None => run_control(endpoint, &["shutdown"]),
        };
    }

    fn wait_or_kill(&mut self, grace: Duration) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl std::ops::Deref for ChildGuard {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        self.child.as_ref().expect("child guard still owns child")
    }
}

impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.child.as_mut().expect("child guard still owns child")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.wait_or_kill(Duration::from_millis(0));
        }
    }
}

pub fn wait_for_status(endpoint: &str) {
    for _ in 0..40 {
        let output = run_control(endpoint, &["status"]);
        if output.status.success()
            && serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .ok()
                .and_then(|value| value["ok"].as_bool())
                == Some(true)
        {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("node daemon did not become ready");
}

pub fn wait_for_status_with_token(endpoint: &str, token: &str) {
    for _ in 0..40 {
        let output = run_control_with_token(endpoint, token, &["status"]);
        if output.status.success()
            && serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .ok()
                .and_then(|value| value["ok"].as_bool())
                == Some(true)
        {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("node daemon did not become ready");
}

pub fn wait_tcp(addr: SocketAddr) {
    for _ in 0..40 {
        if std::net::TcpStream::connect(addr).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("{addr} did not become ready");
}

pub fn start_daemon(endpoint: &str, transport: SocketAddr, routes_path: &PathBuf) -> ChildGuard {
    let home = temp_dir("daemon-home");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"));
    command
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            endpoint,
            "--transport",
            &transport.to_string(),
            "--routes-path",
            &routes_path.display().to_string(),
        ])
        .env("SSH_PROXY_HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(ChildGuard::new)
        .expect("start node daemon")
}

pub fn start_daemon_with_home(
    endpoint: &str,
    transport: SocketAddr,
    routes_path: &PathBuf,
    home: &PathBuf,
) -> ChildGuard {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"));
    command
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            endpoint,
            "--transport",
            &transport.to_string(),
            "--routes-path",
            &routes_path.display().to_string(),
        ])
        .env("SSH_PROXY_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(ChildGuard::new)
        .expect("start node daemon")
}

pub fn start_daemon_with_token(
    endpoint: &str,
    transport: SocketAddr,
    routes_path: &PathBuf,
    token: &str,
    home: &PathBuf,
) -> ChildGuard {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssh_proxy"));
    command
        .args([
            "--log",
            "warn",
            "node",
            "daemon",
            "--control",
            endpoint,
            "--transport",
            &transport.to_string(),
            "--token",
            token,
            "--routes-path",
            &routes_path.display().to_string(),
        ])
        .env("SSH_PROXY_HOME", home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(ChildGuard::new)
        .expect("start node daemon")
}

pub fn stop_child(child: ChildGuard, endpoint: &str) {
    child.shutdown_with_control(endpoint, None);
}

pub fn stop_child_with_token(child: ChildGuard, endpoint: &str, token: &str) {
    child.shutdown_with_control(endpoint, Some(token));
}

pub fn route_store_path(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("ssh_proxy-{name}-{nanos}.json"))
}

pub fn temp_path(name: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("ssh_proxy-{name}-{nanos}.{ext}"))
}

pub fn temp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("ssh_proxy-{name}-{nanos}"));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

pub fn raw_control_request(addr: SocketAddr, request: &str) -> serde_json::Value {
    let mut stream = std::net::TcpStream::connect(addr).expect("connect raw control");
    stream
        .write_all(format!("{request}\n").as_bytes())
        .expect("write raw control");
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("shutdown write");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read raw control");
    serde_json::from_str(&response).expect("json response")
}

pub fn start_http_server(body: &'static str) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind http");
    let addr = listener.local_addr().expect("http addr");
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (addr, handle)
}

pub fn start_http_server_close_delimited_keep_alive(
    body: &'static str,
) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind http");
    let addr = listener.local_addr().expect("http addr");
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set http read timeout");
            let mut request = Vec::new();
            let mut buf = [0_u8; 256];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                    Err(_) => break,
                }
            }
            let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
            let proxy_header_forwarded = request.contains("\r\nproxy-connection:");
            let origin_close_requested = request.contains("\r\nconnection: close\r\n");
            let response = format!("HTTP/1.1 200 OK\r\nConnection: keep-alive\r\n\r\n{body}");
            let _ = stream.write_all(response.as_bytes());
            assert!(!proxy_header_forwarded, "proxy header reached origin");
            assert!(origin_close_requested, "origin close was not requested");
            thread::sleep(Duration::from_secs(3));
        }
    });
    (addr, handle)
}

pub fn start_http_server_many(
    body: &'static str,
    max_requests: usize,
) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind http");
    listener
        .set_nonblocking(true)
        .expect("set http listener nonblocking");
    let addr = listener.local_addr().expect("http addr");
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut served = 0;
        while served < max_requests && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buf = [0_u8; 1024];
                    let _ = stream.read(&mut buf);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    served += 1;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if served > 0 {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    (addr, handle)
}

pub fn socks_get(socks: SocketAddr, target: SocketAddr) -> String {
    let mut stream = std::net::TcpStream::connect(socks).expect("connect socks");
    stream.write_all(&[5, 1, 0]).expect("socks hello");
    let mut hello = [0_u8; 2];
    stream.read_exact(&mut hello).expect("socks hello response");
    assert_eq!(hello, [5, 0]);
    let mut request = vec![5, 1, 0, 1];
    match target.ip() {
        std::net::IpAddr::V4(ip) => request.extend_from_slice(&ip.octets()),
        std::net::IpAddr::V6(_) => panic!("test uses ipv4"),
    }
    request.extend_from_slice(&target.port().to_be_bytes());
    stream.write_all(&request).expect("socks connect");
    let mut reply = [0_u8; 10];
    stream.read_exact(&mut reply).expect("socks reply");
    assert_eq!(reply[1], 0, "socks connect failed: {reply:?}");
    stream
        .write_all(b"GET / HTTP/1.1\r\nhost: 127.0.0.1\r\nconnection: close\r\n\r\n")
        .expect("http request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("http response");
    response
}

pub fn http_connect_get(proxy: SocketAddr, target: SocketAddr) -> String {
    let mut stream = std::net::TcpStream::connect(proxy).expect("connect proxy");
    let connect =
        format!("CONNECT {target} HTTP/1.1\r\nhost: {target}\r\nconnection: keep-alive\r\n\r\n");
    stream.write_all(connect.as_bytes()).expect("http connect");
    let mut response = Vec::new();
    let mut one = [0_u8; 1];
    while !response.windows(4).any(|window| window == b"\r\n\r\n") {
        stream.read_exact(&mut one).expect("connect response");
        response.push(one[0]);
    }
    let response_text = String::from_utf8(response).expect("connect response utf8");
    assert!(response_text.starts_with("HTTP/1.1 200"), "{response_text}");
    stream
        .write_all(b"GET / HTTP/1.1\r\nhost: 127.0.0.1\r\nconnection: close\r\n\r\n")
        .expect("http request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("http response");
    response
}

pub fn http_absolute_get(proxy: SocketAddr, target: SocketAddr) -> String {
    let mut stream = std::net::TcpStream::connect(proxy).expect("connect proxy");
    let request = format!(
        "GET http://{target}/via-http-proxy?x=1 HTTP/1.1\r\nhost: {target}\r\nconnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .expect("absolute http request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("http response");
    response
}

pub fn http_absolute_keep_alive_get(proxy: SocketAddr, target: SocketAddr) -> String {
    let mut stream = std::net::TcpStream::connect(proxy).expect("connect proxy");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set proxy read timeout");
    let request = format!(
        "GET http://{target}/close-delimited HTTP/1.1\r\nhost: {target}\r\nproxy-connection: Keep-Alive\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .expect("absolute http request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("http response");
    response
}

pub fn fixed_tcp_http_get(proxy: SocketAddr) -> String {
    let mut stream = std::net::TcpStream::connect(proxy).expect("connect fixed tcp proxy");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set fixed tcp read timeout");
    stream
        .write_all(b"GET /fixed HTTP/1.1\r\nhost: fixed-target\r\nconnection: close\r\n\r\n")
        .expect("fixed target http request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("http response");
    response
}

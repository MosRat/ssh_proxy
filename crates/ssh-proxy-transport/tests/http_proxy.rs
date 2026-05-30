use ssh_proxy_transport::proxy::http::{
    HttpRequest, HttpRequestKind, HttpResponseBodyMode, rewrite_response_head_for_proxy_close,
};
use tokio::{io::AsyncWriteExt, net::TcpListener};

#[tokio::test]
async fn absolute_form_requests_are_rewritten_for_origin_close() {
    let request = parse_request(
        b"GET http://example.test/path?q=1 HTTP/1.1\r\n\
          Host: example.test\r\n\
          Proxy-Connection: Keep-Alive\r\n\
          Connection: keep-alive, X-Hop\r\n\
          X-Hop: remove-me\r\n\
          User-Agent: test-client\r\n\
          \r\n",
    )
    .await;

    let HttpRequestKind::Forward {
        host,
        port,
        request,
    } = request.kind
    else {
        panic!("expected forward request");
    };
    let request = String::from_utf8(request).expect("forward request utf8");

    assert_eq!(host, "example.test");
    assert_eq!(port, 80);
    assert!(request.starts_with("GET /path?q=1 HTTP/1.1\r\n"));
    assert!(request.contains("\r\nHost: example.test\r\n"));
    assert!(request.contains("\r\nUser-Agent: test-client\r\n"));
    assert!(request.contains("\r\nconnection: close\r\n"));
    assert!(!request.to_ascii_lowercase().contains("proxy-connection"));
    assert!(
        !request
            .to_ascii_lowercase()
            .contains("\r\nconnection: keep-alive")
    );
    assert!(!request.contains("X-Hop: remove-me"));
}

#[tokio::test]
async fn connect_requests_keep_tunnel_semantics() {
    let request = parse_request(
        b"CONNECT example.test:8443 HTTP/1.1\r\n\
          Host: example.test:8443\r\n\
          Proxy-Connection: Keep-Alive\r\n\
          \r\n",
    )
    .await;

    let HttpRequestKind::Connect { host, port } = request.kind else {
        panic!("expected connect request");
    };

    assert_eq!(host, "example.test");
    assert_eq!(port, 8443);
}

#[test]
fn response_rewrite_closes_close_delimited_keep_alive() {
    let (rewritten, mode) = rewrite_response_head_for_proxy_close(
        b"HTTP/1.1 200 OK\r\n\
          Cache-Control: no-cache\r\n\
          Connection: keep-alive, X-Hop\r\n\
          Proxy-Connection: keep-alive\r\n\
          Keep-Alive: timeout=4\r\n\
          X-Hop: remove-me\r\n\
          Server: test\r\n\
          \r\n",
    )
    .expect("rewrite response");
    let rewritten = String::from_utf8(rewritten).expect("response utf8");

    assert_eq!(mode, HttpResponseBodyMode::CloseDelimited);
    assert!(rewritten.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(rewritten.contains("\r\nCache-Control: no-cache\r\n"));
    assert!(rewritten.contains("\r\nServer: test\r\n"));
    assert!(rewritten.contains("\r\nconnection: close\r\n"));
    assert!(!rewritten.to_ascii_lowercase().contains("proxy-connection"));
    assert!(!rewritten.to_ascii_lowercase().contains("keep-alive"));
    assert!(!rewritten.contains("X-Hop: remove-me"));
}

#[test]
fn response_rewrite_preserves_content_length_body_mode() {
    let (rewritten, mode) = rewrite_response_head_for_proxy_close(
        b"HTTP/1.1 200 OK\r\n\
          Content-Length: 2381\r\n\
          Connection: keep-alive\r\n\
          \r\n",
    )
    .expect("rewrite response");
    let rewritten = String::from_utf8(rewritten).expect("response utf8");

    assert_eq!(mode, HttpResponseBodyMode::ContentLength(2381));
    assert!(rewritten.contains("\r\nContent-Length: 2381\r\n"));
    assert!(rewritten.contains("\r\nconnection: close\r\n"));
    assert!(!rewritten.contains("\r\nConnection: keep-alive\r\n"));
}

async fn parse_request(bytes: &'static [u8]) -> HttpRequest {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind parser listener");
    let addr = listener.local_addr().expect("parser listener addr");
    let writer = tokio::spawn(async move {
        let mut stream = tokio::net::TcpStream::connect(addr)
            .await
            .expect("connect parser listener");
        stream.write_all(bytes).await.expect("write request");
        stream.shutdown().await.expect("shutdown request");
    });
    let (mut stream, _) = listener.accept().await.expect("accept parser client");
    let request = HttpRequest::read_from(&mut stream)
        .await
        .expect("parse request");
    writer.await.expect("writer task");
    request
}

//! 端到端执行引擎测试：本地 TCP mock server + 真实 reqwest 请求。
//! 覆盖脚本、digest、oauth2、awsv4、multipart/file 请求体。

use bruno_core::runner::SendRequestInput;
use bruno_core::execute_request;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Recorded {
    fn header(&self, name: &str) -> String {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
}

struct MockServer {
    port: u16,
    seen: Arc<Mutex<Vec<Recorded>>>,
}

fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_headers_end(&buf) {
            let head = String::from_utf8_lossy(&buf[..pos]).to_string();
            let mut lines = head.split("\r\n");
            let request_line = lines.next().unwrap_or("");
            let mut parts = request_line.split(' ');
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let headers: Vec<(String, String)> = lines
                .filter_map(|l| l.split_once(':'))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .collect();
            let content_length = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.parse::<usize>().ok())
                .unwrap_or(0);
            let body_start = pos + 4;
            while buf.len() < body_start + content_length {
                let n = stream.read(&mut chunk).ok()?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body = buf[body_start..std::cmp::min(buf.len(), body_start + content_length)].to_vec();
            return Some(Recorded { method, path, headers, body });
        }
    }
    None
}

fn find_headers_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

impl MockServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = seen.clone();
        let counter = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Some(req) = read_request(&mut stream) else { continue };
                let index = counter.fetch_add(1, Ordering::SeqCst);
                seen_clone.lock().unwrap().push(req.clone());

                let (status, extra_headers, body) = if req.path.starts_with("/token") {
                    (
                        "200 OK",
                        vec![("Content-Type", "application/json")],
                        r#"{"access_token":"tok123","token_type":"Bearer","expires_in":3600}"#.to_string(),
                    )
                } else if req.path.starts_with("/digest") {
                    if req.header("authorization").starts_with("Digest ") {
                        ("200 OK", vec![("Content-Type", "application/json")], r#"{"ok":true}"#.into())
                    } else {
                        (
                            "401 Unauthorized",
                            vec![(
                                "WWW-Authenticate",
                                r#"Digest realm="bruno", qop="auth", nonce="n0nce", opaque="op""#,
                            )],
                            r#"{"error":"auth required"}"#.into(),
                        )
                    }
                } else {
                    // 回显：收到的 header 与 body 长度，便于断言
                    let body_text = String::from_utf8_lossy(&req.body).to_string();
                    let echo = json!({
                        "ok": true,
                        "index": index,
                        "method": req.method,
                        "contentType": req.header("content-type"),
                        "authorization": req.header("authorization"),
                        "bodyLength": req.body.len(),
                        "body": body_text,
                    });
                    ("200 OK", vec![("Content-Type", "application/json")], echo.to_string())
                };

                let mut response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (k, v) in extra_headers {
                    response.push_str(&format!("{k}: {v}\r\n"));
                }
                response.push_str("\r\n");
                response.push_str(&body);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        Self { port, seen }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    fn requests(&self) -> Vec<Recorded> {
        self.seen.lock().unwrap().clone()
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bruno-rs-it-{name}"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn input(item: Value, base: &str) -> SendRequestInput {
    SendRequestInput {
        item,
        collection: json!({ "pathname": base.replace('\\', "/") }),
        environment: None,
        variables: None,
        options: None,
    }
}

#[tokio::test]
async fn script_mutates_request_and_tests_report() {
    let server = MockServer::start();
    let item = json!({
        "name": "scripted",
        "type": "http-request",
        "request": {
            "method": "GET",
            "url": server.url("/echo"),
            "headers": [],
            "body": { "mode": "none" },
            "script": { "req": "req.setHeader('x-from-script', 'yes'); bru.setVar('tag', 'v1');" },
            "tests": "test('status 200', function(){ expect(res.getStatus()).to.equal(200); }); test('echo tag', function(){ expect(res.getBody().body).to.include(''); });"
        }
    });
    let response = execute_request(input(item, &temp_dir("script").to_string_lossy())).await.unwrap();
    assert_eq!(response.status, 200);
    assert!(server.requests()[0].header("x-from-script") == "yes");
    assert_eq!(response.test_results.len(), 2);
    assert!(response.test_results.iter().all(|t| t.status == "pass"), "{:?}", response.test_results);
    assert_eq!(response.runtime_variables.get("tag").map(String::as_str), Some("v1"));
}

#[tokio::test]
async fn digest_auth_retries_with_challenge() {
    let server = MockServer::start();
    let item = json!({
        "name": "digest",
        "type": "http-request",
        "request": {
            "method": "GET",
            "url": server.url("/digest"),
            "headers": [],
            "body": { "mode": "none" },
            "auth": { "mode": "digest", "digest": { "username": "alice", "password": "secret" } }
        }
    });
    let response = execute_request(input(item, &temp_dir("digest").to_string_lossy())).await.unwrap();
    assert_eq!(response.status, 200, "第二次请求应通过 digest 认证");
    let requests = server.requests();
    assert_eq!(requests.len(), 2, "应发生 401 后重试");
    let auth = requests[1].header("authorization");
    assert!(auth.starts_with("Digest "), "{auth}");
    assert!(auth.contains("username=\"alice\""), "{auth}");
    assert!(auth.contains("qop=auth"), "{auth}");
    assert!(auth.contains("uri=\"/digest\""), "{auth}");
}

#[tokio::test]
async fn oauth2_client_credentials_attaches_bearer() {
    let server = MockServer::start();
    let item = json!({
        "name": "oauth",
        "type": "http-request",
        "request": {
            "method": "GET",
            "url": server.url("/echo"),
            "headers": [],
            "body": { "mode": "none" },
            "auth": { "mode": "oauth2", "oauth2": {
                "grantType": "client_credentials",
                "accessTokenUrl": server.url("/token"),
                "clientId": "cid",
                "clientSecret": "csecret",
                "tokenPlacement": "header"
            }}
        }
    });
    let response = execute_request(input(item, &temp_dir("oauth").to_string_lossy())).await.unwrap();
    assert_eq!(response.status, 200);
    let requests = server.requests();
    assert_eq!(requests[0].path, "/token");
    assert_eq!(requests[1].header("authorization"), "Bearer tok123");
}

#[tokio::test]
async fn awsv4_signs_request() {
    let server = MockServer::start();
    let item = json!({
        "name": "aws",
        "type": "http-request",
        "request": {
            "method": "GET",
            "url": server.url("/echo"),
            "headers": [],
            "body": { "mode": "none" },
            "auth": { "mode": "awsv4", "awsv4": {
                "accessKeyId": "AKIDEXAMPLE",
                "secretAccessKey": "SECRET",
                "service": "s3",
                "region": "us-east-1"
            }}
        }
    });
    let response = execute_request(input(item, &temp_dir("aws").to_string_lossy())).await.unwrap();
    assert_eq!(response.status, 200);
    let auth = server.requests()[0].header("authorization");
    assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/"), "{auth}");
    assert!(auth.contains("SignedHeaders="), "{auth}");
    assert!(server.requests()[0].header("x-amz-date").ends_with('Z'));
}

#[tokio::test]
async fn multipart_and_file_bodies_are_sent() {
    let dir = temp_dir("multipart");
    let upload = dir.join("upload.txt");
    std::fs::write(&upload, b"hello-upload").unwrap();
    let server = MockServer::start();

    let item = json!({
        "name": "upload",
        "type": "http-request",
        "request": {
            "method": "POST",
            "url": server.url("/echo"),
            "headers": [],
            "body": { "mode": "multipartForm", "multipartForm": [
                { "name": "field", "type": "text", "value": "v1", "enabled": true },
                { "name": "file", "type": "file", "value": [upload.to_string_lossy()], "enabled": true }
            ]}
        }
    });
    let response = execute_request(input(item, &dir.to_string_lossy())).await.unwrap();
    assert_eq!(response.status, 200);
    let requests = server.requests();
    let content_type = requests[0].header("content-type");
    assert!(content_type.starts_with("multipart/form-data; boundary="), "{content_type}");
    let body = String::from_utf8_lossy(&requests[0].body).to_string();
    assert!(body.contains("name=\"field\"") && body.contains("v1"), "{body}");
    assert!(body.contains("hello-upload"), "{body}");

    // file / binary body：相对路径基于集合目录解析
    let item = json!({
        "name": "binary",
        "type": "http-request",
        "request": {
            "method": "POST",
            "url": server.url("/echo"),
            "headers": [],
            "body": { "mode": "file", "file": [
                { "filePath": "upload.txt", "contentType": "text/plain", "selected": true }
            ]}
        }
    });
    let response = execute_request(input(item, &dir.to_string_lossy())).await.unwrap();
    assert_eq!(response.status, 200);
    let last = server.requests().last().cloned().unwrap();
    assert_eq!(last.header("content-type"), "text/plain");
    assert_eq!(String::from_utf8_lossy(&last.body), "hello-upload");
}

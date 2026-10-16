//! Bounds remote JSON before buffering it. Errors never contain upstream bytes or URLs.
use crate::{Error, Result};
use futures_util::StreamExt;
use serde_json::Value;

pub(crate) const JSON_LIMIT: usize = 8 * 1024 * 1024;
pub(crate) const STYLE_LIMIT: usize = 2 * 1024 * 1024;

pub(crate) async fn json(response: reqwest::Response, limit: usize) -> Result<Value> {
    let too_large = || Error::new(502, "Resposta JSON excedeu o limite de leitura.");
    if let Some(length) = response.headers().get(reqwest::header::CONTENT_LENGTH) {
        let length = length
            .to_str()
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| Error::new(502, "Content-Length upstream inválido."))?;
        if length > limit as u64 {
            return Err(too_large());
        }
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::new(502, "Leitura JSON upstream interrompida."))?;
        let len = bytes.len().checked_add(chunk.len()).ok_or_else(too_large)?;
        if len > limit {
            return Err(too_large());
        }
        if len > bytes.capacity() {
            // Geometric growth without requesting capacity beyond the body ceiling.
            let capacity = bytes.capacity().saturating_mul(2).max(len).min(limit);
            bytes.reserve_exact(capacity - bytes.len());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::new(502, "JSON upstream inválido."))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub(crate) struct Fixture {
        pub url: String,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    // One isolated connection; no environment, provider, subprocess, or persistent service.
    pub(crate) async fn serve(headers: &str, body: Vec<u8>) -> Fixture {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let mut response = format!("HTTP/1.1 200 OK\r\n{headers}\r\n").into_bytes();
        response.extend(body);
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 8192];
            // Consume the request, including its body, before responding (avoid TCP resets).
            loop {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    return;
                }
                request.extend_from_slice(&chunk[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                assert!(
                    request.len() < 1024 * 1024,
                    "fixture request exceeded its test budget"
                );
            }
            let _ = socket.write_all(&response).await;
            // Oversized Content-Length cases send no body. The reader must reject
            // headers without waiting for this connection to close.
            let _ = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut chunk)).await;
        });
        Fixture { url, task }
    }
    pub(crate) fn chunked_oversize(limit: usize) -> Vec<u8> {
        let mut body = Vec::new();
        let chunk = vec![b' '; 64 * 1024];
        let mut left = limit + 1;
        while left > 0 {
            let len = left.min(chunk.len());
            body.extend_from_slice(format!("{len:X}\r\n").as_bytes());
            body.extend_from_slice(&chunk[..len]);
            body.extend_from_slice(b"\r\n");
            left -= len;
        }
        body.extend_from_slice(b"0\r\n\r\n");
        body
    }
    #[tokio::test]
    async fn accepts_json_at_the_ceiling_and_sanitizes_invalid_json() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for body in [b"{\"ok\":true}".to_vec(), b"SECRET_UPSTREAM_BODY".to_vec()] {
            let fixture = serve(&format!("Content-Length: {}\r\n", body.len()), body.clone()).await;
            let response = client
                .get(&fixture.url)
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .unwrap();
            let result = json(response, body.len()).await;
            if body[0] == b'{' {
                assert_eq!(result.unwrap()["ok"], true);
            } else {
                assert!(!result.unwrap_err().message.contains("SECRET"));
            }
        }
    }
    #[tokio::test]
    async fn rejects_declared_size_before_waiting_and_counts_chunked_bytes() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for (headers, body) in [
            ("Content-Length: 1025\r\n", vec![]),
            ("Transfer-Encoding: chunked\r\n", chunked_oversize(1024)),
        ] {
            let fixture = serve(headers, body).await;
            let response = client
                .get(&fixture.url)
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .unwrap();
            let error = tokio::time::timeout(Duration::from_millis(500), json(response, 1024))
                .await
                .expect("must reject without waiting for EOF")
                .unwrap_err();
            assert_eq!(error.status, 502);
            assert!(error.message.contains("limite"));
        }
    }
}

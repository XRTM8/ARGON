use tokio::io::{AsyncRead, AsyncReadExt};

// A TCP read is not an HTTP message. Read the complete bounded body before JSON parsing.
pub async fn read_request(stream: &mut (impl AsyncRead + Unpin)) -> Result<String, String> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                if header_end > 16384 {
                    return Err("Headers too large".into());
                }
                let head = std::str::from_utf8(&request[..header_end])
                    .map_err(|_| "Invalid HTTP headers")?;
                if head
                    .lines()
                    .any(|line| line.to_ascii_lowercase().starts_with("transfer-encoding:"))
                {
                    return Err("Chunked requests unsupported".into());
                }
                let lengths: Vec<_> = head
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    .collect();
                if lengths.len() > 1 {
                    return Err("Duplicate Content-Length".into());
                }
                let length = match lengths.first() {
                    Some((_, value)) => value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| "Invalid Content-Length")?,
                    None => 0,
                };
                if length > 65536 {
                    return Err("Request body too large".into());
                }
                let total = header_end + 4 + length;
                if request.len() >= total {
                    return String::from_utf8(request[..total].to_vec())
                        .map_err(|_| "Invalid UTF-8 request".into());
                }
            } else if request.len() > 16384 {
                return Err("Headers too large".into());
            }
            let n = stream.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("Incomplete HTTP request".into());
            }
            request.extend_from_slice(&buffer[..n]);
        }
    })
    .await
    .map_err(|_| "HTTP request timed out".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test]
    async fn reads_fragmented_headers_and_unicode_body() {
        let (mut writer, mut reader) = tokio::io::duplex(4096);
        let body = r#"{"url":"https://example.com/ملف.zip"}"#;
        let head = format!(
            "POST /add HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let expected = format!("{head}{body}");
        let write = tokio::spawn(async move {
            for part in [
                head.as_bytes(),
                &body.as_bytes()[..12],
                &body.as_bytes()[12..],
            ] {
                writer.write_all(part).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        assert_eq!(read_request(&mut reader).await.unwrap(), expected);
        write.await.unwrap();
    }
    #[tokio::test]
    async fn rejects_incomplete_or_oversized_bodies() {
        for raw in [
            "POST /add HTTP/1.1\r\nContent-Length: 999999\r\n\r\n",
            "POST /add HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc",
        ] {
            let mut bytes = raw.as_bytes();
            assert!(read_request(&mut bytes).await.is_err());
        }
    }
}

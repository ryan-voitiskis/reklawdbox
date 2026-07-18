//! Shared provider HTTP helpers.

const MAX_ERROR_BODY_BYTES: usize = 8_192;
const ERROR_BODY_OBSERVATION_BYTES: usize = MAX_ERROR_BODY_BYTES + 1;
const MAX_LOSSY_ERROR_BODY_BYTES: usize = MAX_ERROR_BODY_BYTES * 3;
pub(crate) const ERROR_BODY_READ_FAILED: &str = "[body read failed]";

/// Read a remote error response into a bounded, single-line diagnostic.
///
/// Remote prose must never become an unbounded or control-character-bearing
/// user-facing error. Observe at most one byte beyond the retained raw-byte
/// boundary so truncation is deterministic without buffering the whole
/// response. Lossy UTF-8 expansion remains bounded to three times the retained
/// bytes and does not itself imply that the remote body was truncated.
pub(crate) async fn read_bounded_error_body(mut response: reqwest::Response) -> String {
    let mut retained = Vec::with_capacity(MAX_ERROR_BODY_BYTES);
    let mut observed = 0usize;
    let mut raw_overflow = false;

    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let remaining_observation = ERROR_BODY_OBSERVATION_BYTES - observed;
                let observed_from_chunk = chunk.len().min(remaining_observation);
                observed += observed_from_chunk;

                let remaining_retained = MAX_ERROR_BODY_BYTES - retained.len();
                retained.extend_from_slice(&chunk[..chunk.len().min(remaining_retained)]);

                if observed > MAX_ERROR_BODY_BYTES || chunk.len() > observed_from_chunk {
                    raw_overflow = true;
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => return ERROR_BODY_READ_FAILED.to_string(),
        }
    }

    let decoded = String::from_utf8_lossy(&retained);
    debug_assert!(decoded.len() <= MAX_LOSSY_ERROR_BODY_BYTES);
    let mut sanitized = String::with_capacity(decoded.len());
    for character in decoded.chars() {
        let replacement = if character.is_ascii_control() {
            ' '
        } else {
            character
        };
        sanitized.push(replacement);
    }

    if raw_overflow {
        if !sanitized.is_empty() {
            sanitized.push(' ');
        }
        sanitized.push_str("[truncated]");
    }
    sanitized
}

pub(crate) fn urlencoding(s: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    const SET: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    utf8_percent_encode(s, SET).to_string()
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    async fn error_response(body: Vec<u8>, declared_length: Option<usize>) -> reqwest::Response {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1_024];
            let _ = socket.read(&mut request).await;
            let length = declared_length.unwrap_or(body.len());
            let headers = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        });

        reqwest::get(format!("http://{address}"))
            .await
            .expect("fixture response")
    }

    #[tokio::test]
    async fn discogs_error_body_boundary_preserves_exact_limit_without_marker() {
        let body = vec![b'x'; MAX_ERROR_BODY_BYTES];
        let response = error_response(body, None).await;
        let diagnostic = read_bounded_error_body(response).await;
        assert_eq!(diagnostic, "x".repeat(MAX_ERROR_BODY_BYTES));
        assert!(!diagnostic.contains("[truncated]"));
    }

    #[tokio::test]
    async fn discogs_error_body_boundary_marks_overflow_once() {
        let body = vec![b'x'; ERROR_BODY_OBSERVATION_BYTES + 500];
        let response = error_response(body, None).await;
        let diagnostic = read_bounded_error_body(response).await;
        assert_eq!(
            diagnostic,
            format!("{} [truncated]", "x".repeat(MAX_ERROR_BODY_BYTES))
        );
        assert_eq!(diagnostic.matches("[truncated]").count(), 1);
    }

    #[tokio::test]
    async fn discogs_error_body_boundary_decodes_lossily_and_replaces_controls() {
        let response = error_response(vec![b'a', b'\n', b'\r', 0x1b, 0x7f, 0xff, b'z'], None).await;
        let diagnostic = read_bounded_error_body(response).await;
        assert_eq!(diagnostic, "a    �z");
        assert!(
            !diagnostic
                .chars()
                .any(|character| character.is_ascii_control())
        );
    }

    #[tokio::test]
    async fn discogs_error_body_boundary_lossy_expansion_does_not_imply_raw_overflow() {
        let mut body = vec![b'x'; MAX_ERROR_BODY_BYTES - 2];
        body.push(0xff);
        assert_eq!(body.len(), MAX_ERROR_BODY_BYTES - 1);

        let response = error_response(body, None).await;
        let diagnostic = read_bounded_error_body(response).await;
        assert_eq!(
            diagnostic,
            format!("{}�", "x".repeat(MAX_ERROR_BODY_BYTES - 2))
        );
        assert_eq!(diagnostic.len(), MAX_ERROR_BODY_BYTES + 1);
        assert!(diagnostic.len() <= MAX_LOSSY_ERROR_BODY_BYTES);
        assert!(!diagnostic.contains("[truncated]"));
    }

    #[tokio::test]
    async fn discogs_error_body_boundary_returns_stable_read_failure() {
        let response = error_response(b"partial".to_vec(), Some(100)).await;
        let diagnostic = read_bounded_error_body(response).await;
        assert_eq!(diagnostic, ERROR_BODY_READ_FAILED);
    }
}

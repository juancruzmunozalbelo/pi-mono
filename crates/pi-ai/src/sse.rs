//! Generic SSE (Server-Sent Events) stream parser.
//!
//! Handles both OpenAI (`data: [DONE]`) and Anthropic (`event:` typed) formats.

use bytes::Bytes;
use futures::Stream;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use thiserror::Error;

/// An SSE event with an optional typed name and a data payload.
#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    /// Value of the `event:` field, if present.
    pub event_type: Option<String>,
    /// Concatenated `data:` lines (joined with `\n`).
    pub data: String,
}

/// Errors that can occur while parsing the SSE stream.
#[derive(Debug, Error)]
pub enum SseError {
    #[error("HTTP stream error: {0}")]
    Stream(#[from] reqwest::Error),
}

pin_project! {
    pub struct SseStream<S> {
        #[pin]
        inner: S,
        buffer: String,
        current_event_type: Option<String>,
        current_data: Vec<String>,
        done: bool,
    }
}

impl<S> SseStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>>,
{
    /// Wrap an existing byte stream.
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            buffer: String::new(),
            current_event_type: None,
            current_data: Vec::new(),
            done: false,
        }
    }

    /// Process one complete SSE line and, if an event is ready, return it.
    ///
    /// Returns `Some(event)` when an empty line delimits a complete event,
    /// returns `None` (without closing the stream) for all other lines,
    /// and signals stream-end via the `done` flag for `[DONE]`.
    fn process_line(
        event_type: &mut Option<String>,
        data_lines: &mut Vec<String>,
        done: &mut bool,
        line: &str,
    ) -> Option<SseEvent> {
        if line.is_empty() {
            // Empty line: dispatch accumulated event.
            if data_lines.is_empty() {
                return None;
            }
            let event = SseEvent {
                event_type: event_type.take(),
                data: data_lines.join("\n"),
            };
            data_lines.clear();
            return Some(event);
        }

        if line.starts_with(": ") || line == ":" {
            // Comment / heartbeat — skip.
            return None;
        }

        if line == "data: [DONE]" || line == "data:[DONE]" {
            *done = true;
            return None;
        }

        if let Some(payload) = line.strip_prefix("data: ") {
            data_lines.push(payload.to_owned());
        } else if let Some(payload) = line.strip_prefix("data:") {
            data_lines.push(payload.to_owned());
        } else if let Some(payload) = line.strip_prefix("event: ") {
            *event_type = Some(payload.to_owned());
        } else if let Some(payload) = line.strip_prefix("event:") {
            *event_type = Some(payload.to_owned());
        }
        // Other field types (id:, retry:, …) are silently ignored.

        None
    }
}

impl<S> Stream for SseStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>>,
{
    type Item = Result<SseEvent, SseError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            // First, try to drain any complete lines already sitting in the buffer.
            if let Some(newline_pos) = this.buffer.find('\n') {
                let line: String = this.buffer[..newline_pos].trim_end_matches('\r').to_owned();
                *this.buffer = this.buffer[newline_pos + 1..].to_owned();

                if let Some(event) =
                    Self::process_line(this.current_event_type, this.current_data, this.done, &line)
                {
                    return Poll::Ready(Some(Ok(event)));
                }

                // [DONE] was set — end the stream.
                if *this.done {
                    return Poll::Ready(None);
                }

                // Keep draining.
                continue;
            }

            // Buffer has no complete line; pull more bytes from the inner stream.
            match this.inner.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    // Inner stream ended. Process any remaining buffer content
                    // that wasn't terminated by \n.
                    if !this.buffer.is_empty() {
                        let remaining = std::mem::take(this.buffer);
                        let line = remaining.trim_end_matches('\r');
                        Self::process_line(
                            this.current_event_type,
                            this.current_data,
                            this.done,
                            line,
                        );
                    }
                    // Emit any accumulated event.
                    if !this.current_data.is_empty() {
                        let event = SseEvent {
                            event_type: this.current_event_type.take(),
                            data: this.current_data.join("\n"),
                        };
                        this.current_data.clear();
                        return Poll::Ready(Some(Ok(event)));
                    }
                    return Poll::Ready(None);
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(SseError::Stream(e))));
                }
                Poll::Ready(Some(Ok(bytes))) => {
                    // Append new bytes and loop back to drain lines.
                    match std::str::from_utf8(&bytes) {
                        Ok(s) => this.buffer.push_str(s),
                        Err(_) => {
                            // Best-effort: use lossy conversion so we never panic.
                            this.buffer.push_str(&String::from_utf8_lossy(&bytes));
                        }
                    }
                }
            }
        }
    }
}

// ─── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use tokio_stream::iter as stream_iter;

    /// Convert a `&str` slice into the stream type our parser expects.
    fn make_stream(chunks: Vec<&'static str>) -> impl Stream<Item = Result<Bytes, reqwest::Error>> {
        stream_iter(
            chunks
                .into_iter()
                .map(|s| Ok::<Bytes, reqwest::Error>(Bytes::from(s))),
        )
    }

    #[tokio::test]
    async fn test_basic_text_event() {
        let s = make_stream(vec!["data: {\"text\":\"hello\"}\n\n"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(event.event_type, None);
        assert_eq!(event.data, r#"{"text":"hello"}"#);
        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_multiple_events() {
        let s = make_stream(vec!["data: first\n\ndata: second\n\n"]);
        let mut parser = SseStream::new(s);

        let e1 = parser.next().await.unwrap().unwrap();
        assert_eq!(e1.data, "first");

        let e2 = parser.next().await.unwrap().unwrap();
        assert_eq!(e2.data, "second");

        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_typed_event_anthropic() {
        let s = make_stream(vec![
            "event: content_block_delta\ndata: {\"type\":\"text_delta\"}\n\n",
        ]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(event.event_type, Some("content_block_delta".to_owned()));
        assert_eq!(event.data, r#"{"type":"text_delta"}"#);
    }

    #[tokio::test]
    async fn test_comment_lines_skipped() {
        let s = make_stream(vec![": heartbeat\ndata: hi\n\n"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(event.data, "hi");
        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_done_sentinel_ends_stream() {
        let s = make_stream(vec!["data: [DONE]\n\n"]);
        let mut parser = SseStream::new(s);
        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_partial_chunks() {
        // Event split across two byte chunks.
        let s = make_stream(vec!["data: hel", "lo\n\n"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(event.data, "hello");
        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_empty_data_line() {
        let s = make_stream(vec!["data:\n\n"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(event.data, "");
    }

    #[tokio::test]
    async fn test_multi_line_data() {
        let s = make_stream(vec!["data: line1\ndata: line2\n\n"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(event.data, "line1\nline2");
    }

    // ── Edge cases ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_crlf_line_endings() {
        // Windows-style \r\n line endings should work the same as \n
        let s = make_stream(vec!["data: hello\r\n\r\n"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(
            event.data, "hello",
            "CRLF line endings should be stripped and event parsed correctly"
        );
        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_no_trailing_newline_at_stream_end() {
        // Stream ends without final \n\n — should still emit the buffered event
        let s = make_stream(vec!["data: incomplete"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(
            event.data, "incomplete",
            "Event with no trailing newline at stream end should be emitted"
        );
        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_multiple_consecutive_empty_lines_no_empty_events() {
        // Multiple empty lines should not produce multiple/empty events
        let s = make_stream(vec!["data: hello\n\n\n\n\n"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(event.data, "hello");

        // Should NOT produce empty events for the extra blank lines
        assert!(
            parser.next().await.is_none(),
            "Extra empty lines should not produce empty events"
        );
    }

    #[tokio::test]
    async fn test_very_large_data_payload() {
        // 100KB+ payload in a single event
        let large_payload = "x".repeat(100 * 1024);
        let sse_str = format!("data: {large_payload}\n\n");
        // We need an owned string for the stream; use Bytes directly
        let bytes = bytes::Bytes::from(sse_str);
        let stream = tokio_stream::iter(vec![Ok::<bytes::Bytes, reqwest::Error>(bytes)]);
        let mut parser = SseStream::new(stream);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(
            event.data.len(),
            100 * 1024,
            "Large payload should be preserved without truncation"
        );
        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_done_sentinel_preceded_by_data_event_in_same_chunk() {
        // data event followed by [DONE] in the same chunk
        let s = make_stream(vec!["data: {\"text\":\"hi\"}\n\ndata: [DONE]\n\n"]);
        let mut parser = SseStream::new(s);

        // First event should be the data event
        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(
            event.data, r#"{"text":"hi"}"#,
            "Data event before [DONE] should be emitted"
        );

        // Stream should end after [DONE]
        assert!(
            parser.next().await.is_none(),
            "Stream should end after [DONE] sentinel"
        );
    }

    #[tokio::test]
    async fn test_comment_only_stream() {
        // Only comment lines — no events should be emitted
        let s = make_stream(vec![": heartbeat\n: another comment\n: and another\n"]);
        let mut parser = SseStream::new(s);

        assert!(
            parser.next().await.is_none(),
            "Comment-only stream should emit no events"
        );
    }

    #[tokio::test]
    async fn test_event_field_with_no_data_field() {
        // event: type with no subsequent data: field — should emit no event
        let s = make_stream(vec!["event: content_block_start\n\n"]);
        let mut parser = SseStream::new(s);

        assert!(
            parser.next().await.is_none(),
            "event: field with no data: should not emit an event"
        );
    }

    #[tokio::test]
    async fn test_chunk_splits_data_prefix_mid_word() {
        // "da" in one chunk, "ta: hello\n\n" in next
        let s = make_stream(vec!["da", "ta: hello\n\n"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(
            event.data, "hello",
            "Chunk split mid-prefix should be reassembled correctly"
        );
        assert!(parser.next().await.is_none());
    }

    #[tokio::test]
    async fn test_data_no_space_after_colon() {
        // Per SSE spec, "data:value" (no space) should work
        let s = make_stream(vec!["data:nospace\n\n"]);
        let mut parser = SseStream::new(s);

        let event = parser.next().await.unwrap().unwrap();
        assert_eq!(
            event.data, "nospace",
            "data: without space after colon should be parsed per SSE spec"
        );
    }
}

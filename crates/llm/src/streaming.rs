//! Bounded byte decoding and shared delta-stream lifecycle.

use crate::{ProviderError, StreamChunk, UsageAcc, clock_now};
use bytes::Bytes;
use futures::{Stream, StreamExt, stream};
use openwebide_core::{ChatRequest, StopReason};
use std::{collections::VecDeque, pin::Pin};

/// What one line of a provider stream means.
///
/// Step 39's `chat_tools_stream` uses the same parsers and terminal rule.
pub(crate) enum StreamLine {
    /// A content delta to emit.
    Delta(String),
    /// The model finished; may carry a last delta. More metadata lines may
    /// follow; EOF is now acceptable.
    Finished(Option<String>),
    /// The stream finished successfully.
    Done,
    /// A keep-alive or metadata line to ignore.
    Skip,
}

/// The default cap on a single stream line's byte length. Ollama puts a
/// whole tool call, including a full `write_file` body, on one NDJSON
/// line, so this is generous.
const MAX_LINE_BYTES: usize = 16 << 20;

/// Splits a byte-chunk stream into newline-delimited lines.
///
/// Handles chunks that split a line (or a multi-byte UTF-8 character)
/// mid-way, `\r\n` line endings, and a final line without a trailing
/// newline. The buffer is bounded by `max_line`: a line longer than that
/// yields an error and fuses the stream.
pub(crate) struct LineStream<S> {
    inner: S,
    buffer: Vec<u8>,
    scan_from: usize,
    max_line: usize,
    done: bool,
}

impl<S> LineStream<S> {
    pub(crate) fn new(inner: S) -> Self {
        Self::with_max_line(inner, MAX_LINE_BYTES)
    }

    pub(crate) fn with_max_line(inner: S, max_line: usize) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            scan_from: 0,
            max_line,
            done: false,
        }
    }
}

impl<S> Stream for LineStream<S>
where
    S: Stream<Item = Result<Bytes, ProviderError>> + Unpin,
{
    type Item = Result<String, ProviderError>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::task::Poll;

        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        loop {
            if let Some(pos) = this.buffer[this.scan_from..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|p| p + this.scan_from)
            {
                if pos + 1 > this.max_line {
                    this.done = true;
                    let max_line = this.max_line;
                    return Poll::Ready(Some(Err(ProviderError::Parse(format!(
                        "stream line exceeds {max_line} bytes"
                    )))));
                }
                let mut end = pos;
                if end > 0 && this.buffer[end - 1] == b'\r' {
                    end -= 1;
                }
                let line_bytes: Vec<u8> = this.buffer[..end].to_vec();
                this.buffer.drain(..=pos);
                this.scan_from = 0;
                if line_bytes.is_empty() {
                    continue;
                }
                return match String::from_utf8(line_bytes) {
                    Ok(line) => Poll::Ready(Some(Ok(line))),
                    Err(_) => {
                        this.done = true;
                        Poll::Ready(Some(Err(ProviderError::Parse(
                            "stream line is not valid UTF-8".into(),
                        ))))
                    }
                };
            }
            this.scan_from = this.buffer.len();
            if this.buffer.len() > this.max_line {
                this.done = true;
                let max_line = this.max_line;
                return Poll::Ready(Some(Err(ProviderError::Parse(format!(
                    "stream line exceeds {max_line} bytes"
                )))));
            }
            match Stream::poll_next(std::pin::Pin::new(&mut this.inner), cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    this.buffer.extend_from_slice(&chunk);
                }
                Poll::Ready(Some(Err(e))) => {
                    this.done = true;
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(None) => {
                    this.done = true;
                    let line_bytes = std::mem::take(&mut this.buffer);
                    if line_bytes.is_empty() {
                        return Poll::Ready(None);
                    }
                    return match String::from_utf8(line_bytes) {
                        Ok(line) => Poll::Ready(Some(Ok(line))),
                        Err(_) => Poll::Ready(Some(Err(ProviderError::Parse(
                            "stream line is not valid UTF-8".into(),
                        )))),
                    };
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

pub(crate) struct ParsedLine {
    pub event: StreamLine,
    pub reasoning: Option<String>,
    pub stop: Option<StopReason>,
}

/// Providers parse their own metadata; terminal handling, estimation and errors are shared.
type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, ProviderError>> + Send>>;

pub(crate) fn chat_stream(
    lines: LineStream<ByteStream>,
    request: &ChatRequest,
    parse: fn(&str, &mut UsageAcc) -> Result<ParsedLine, ProviderError>,
) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
    Box::pin(stream::unfold(
        (
            lines,
            UsageAcc::new(request),
            VecDeque::new(),
            false,
            false,
            StopReason::Complete,
        ),
        move |(mut lines, mut acc, mut pending, mut complete, mut ended, mut stop_reason)| async move {
            loop {
                if let Some(chunk) = pending.pop_front() {
                    return Some((
                        Ok(chunk),
                        (lines, acc, pending, complete, ended, stop_reason),
                    ));
                }
                if ended {
                    return None;
                }
                let result = match lines.next().await {
                    None if !complete => Err(ProviderError::Incomplete),
                    Some(Err(e)) => Err(e),
                    line => (|| {
                        let mut done = line.is_none();
                        if let Some(Ok(line)) = line {
                            let ParsedLine {
                                event: parsed,
                                reasoning,
                                stop,
                            } = parse(&line, &mut acc)?;
                            if let Some(reason) = stop {
                                stop_reason = reason;
                            }
                            if let Some(reasoning) = reasoning.filter(|s| !s.is_empty()) {
                                if acc.started.is_none() {
                                    acc.started = clock_now();
                                }
                                acc.text.push_str(&reasoning);
                                pending.push_back(StreamChunk::Reasoning(reasoning));
                            }
                            let delta = match parsed {
                                StreamLine::Delta(delta) => Some(delta),
                                StreamLine::Finished(delta) => {
                                    complete = true;
                                    delta
                                }
                                StreamLine::Done => {
                                    done = true;
                                    None
                                }
                                StreamLine::Skip => None,
                            };
                            if let Some(delta) = delta {
                                if acc.started.is_none() {
                                    acc.started = clock_now();
                                }
                                acc.text.push_str(&delta);
                                pending.push_back(StreamChunk::Delta(delta));
                            }
                        }
                        if done {
                            acc.ended = clock_now();
                            pending.push_back(StreamChunk::Stop(stop_reason));
                            pending.push_back(StreamChunk::Usage(acc.finish()));
                            ended = true;
                        }
                        Ok(())
                    })(),
                };
                if let Err(e) = result {
                    pending.clear();
                    return Some((Err(e), (lines, acc, pending, complete, true, stop_reason)));
                }
            }
        },
    ))
}

pub(crate) struct ToolLine {
    pub parsed: ParsedLine,
    pub end: bool,
    pub tool_activity: bool,
}
pub(crate) trait ToolParser: Send {
    fn parse(&mut self, line: &str, usage: &mut UsageAcc) -> Result<ToolLine, ProviderError>;
    fn finish(&mut self) -> Result<Vec<openwebide_core::ToolCall>, ProviderError>;
}
struct ToolState<P> {
    lines: LineStream<ByteStream>,
    parser: P,
    usage: UsageAcc,
    content: String,
    reason: StopReason,
    pending: VecDeque<crate::ToolStreamChunk>,
    complete: bool,
    ended: bool,
}

/// One terminal/error/telemetry policy for streamed tools; adapters only decode protocol fields.
pub(crate) fn tool_stream<P: ToolParser + 'static>(
    lines: LineStream<ByteStream>,
    request: &ChatRequest,
    parser: P,
) -> Pin<Box<dyn Stream<Item = Result<crate::ToolStreamChunk, ProviderError>> + Send>> {
    use crate::ToolStreamChunk;
    let state = ToolState {
        lines,
        parser,
        usage: UsageAcc::new(request),
        content: String::new(),
        reason: StopReason::Complete,
        pending: VecDeque::new(),
        complete: false,
        ended: false,
    };
    Box::pin(stream::unfold(state, |mut state| async move {
        loop {
            if let Some(chunk) = state.pending.pop_front() {
                return Some((Ok(chunk), state));
            }
            if state.ended {
                return None;
            }
            let result = match state.lines.next().await {
                Some(Err(error)) => Err(error),
                None if !state.complete => Err(ProviderError::Incomplete),
                line => (|| {
                    let mut done = line.is_none();
                    if let Some(Ok(line)) = line {
                        let ToolLine {
                            parsed,
                            end,
                            tool_activity,
                        } = state.parser.parse(&line, &mut state.usage)?;
                        if tool_activity && state.usage.started.is_none() {
                            state.usage.started = clock_now();
                        }
                        if let Some(reason) = parsed.stop {
                            state.reason = reason;
                        }
                        if let Some(reasoning) = parsed.reasoning.filter(|text| !text.is_empty()) {
                            if state.usage.started.is_none() {
                                state.usage.started = clock_now();
                            }
                            state.usage.text.push_str(&reasoning);
                            state
                                .pending
                                .push_back(ToolStreamChunk::Reasoning(reasoning));
                        }
                        let delta = match parsed.event {
                            StreamLine::Delta(delta) => Some(delta),
                            StreamLine::Finished(delta) => {
                                state.complete = true;
                                delta
                            }
                            StreamLine::Done => {
                                done = true;
                                None
                            }
                            StreamLine::Skip => None,
                        };
                        if let Some(delta) = delta {
                            if state.usage.started.is_none() {
                                state.usage.started = clock_now();
                            }
                            state.content.push_str(&delta);
                            state.usage.text.push_str(&delta);
                            state.pending.push_back(ToolStreamChunk::Delta(delta));
                        }
                        done |= end;
                    }
                    if done {
                        let calls = state.parser.finish()?;
                        for call in &calls {
                            state.usage.text.push_str(&call.name);
                            state.usage.text.push_str(&call.arguments);
                        }
                        state.usage.ended = clock_now();
                        state.pending.push_back(ToolStreamChunk::Stop(state.reason));
                        state
                            .pending
                            .push_back(ToolStreamChunk::Usage(state.usage.finish()));
                        let response = if calls.is_empty() {
                            openwebide_core::ChatResponse::Text(std::mem::take(&mut state.content))
                        } else {
                            openwebide_core::ChatResponse::ToolCalls(calls)
                        };
                        state.pending.push_back(ToolStreamChunk::Response(response));
                        state.ended = true;
                    }
                    Ok(())
                })(),
            };
            if let Err(error) = result {
                state.pending.clear();
                state.ended = true;
                return Some((Err(error), state));
            }
        }
    }))
}

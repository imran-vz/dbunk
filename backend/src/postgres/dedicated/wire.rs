//! Passive observation of the PostgreSQL backend protocol on a dedicated
//! socket. The server reports its transaction status in every ReadyForQuery
//! and its cancel identity in BackendKeyData; tokio-postgres consumes both
//! without exposing them. Reading them off the wire is authoritative behind
//! any connection pooler and sends no SQL on the observed session.
//!
//! The raw socket layer parses messages until TLS is negotiated; the TLS
//! layer then parses the decrypted stream. Anything unexpected makes the
//! observer inert and the status unknown, never guessed.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::tls::{ChannelBinding, TlsConnect, TlsStream};

/// `SSLRequest`: length 8, code 80877103.
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f];
/// Protocol versions 3.0 and 3.2 in a StartupMessage.
const STARTUP_VERSIONS: [[u8; 4]; 2] = [[0, 3, 0, 0], [0, 3, 0, 2]];
const UNKNOWN: u8 = 0;

/// Server transaction status from the latest ReadyForQuery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireTransaction {
    Idle,
    InTransaction,
    Failed,
}

#[derive(Default)]
pub(crate) struct WireState {
    status: AtomicU8,
    /// Process ID announced in BackendKeyData; 0 until seen.
    key_pid: AtomicI32,
}

impl WireState {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// `None` until a ReadyForQuery is parsed, or once the stream could not
    /// be followed. Callers that need the status after a request whose
    /// response may still be draining must first complete a later request.
    pub(crate) fn transaction(&self) -> Option<WireTransaction> {
        match self.status.load(Ordering::SeqCst) {
            b'I' => Some(WireTransaction::Idle),
            b'T' => Some(WireTransaction::InTransaction),
            b'E' => Some(WireTransaction::Failed),
            _ => None,
        }
    }

    pub(crate) fn key_pid(&self) -> Option<i32> {
        Some(self.key_pid.load(Ordering::SeqCst)).filter(|pid| *pid != 0)
    }

    fn lost(&self) {
        self.status.store(UNKNOWN, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The raw layer has not yet seen what the client sent first.
    Pending,
    /// The next inbound byte answers an SSLRequest.
    SslAnswer,
    Framed,
    /// TLS took over; the decrypted layer observes from here.
    HandedOff,
    /// The stream could not be followed.
    Lost,
}

/// Incremental backend-message framing across arbitrary read boundaries.
struct Parser {
    phase: Phase,
    header: [u8; 5],
    header_len: usize,
    /// Body bytes still to skip or capture for the current message.
    remaining: usize,
    captured: [u8; 4],
    captured_len: usize,
}

impl Parser {
    fn new(phase: Phase) -> Self {
        Self {
            phase,
            header: [0; 5],
            header_len: 0,
            remaining: 0,
            captured: [0; 4],
            captured_len: 0,
        }
    }

    fn lose(&mut self, state: &WireState) {
        self.phase = Phase::Lost;
        state.lost();
    }

    fn feed(&mut self, mut bytes: &[u8], state: &WireState) {
        while !bytes.is_empty() {
            match self.phase {
                Phase::HandedOff | Phase::Lost => return,
                Phase::Pending => return self.lose(state),
                Phase::SslAnswer => {
                    self.phase = match bytes[0] {
                        b'S' => Phase::HandedOff,
                        b'N' => Phase::Framed,
                        _ => return self.lose(state),
                    };
                    bytes = &bytes[1..];
                }
                Phase::Framed if self.header_len < 5 => {
                    let take = (5 - self.header_len).min(bytes.len());
                    self.header[self.header_len..self.header_len + take]
                        .copy_from_slice(&bytes[..take]);
                    self.header_len += take;
                    bytes = &bytes[take..];
                    if self.header_len == 5 {
                        let length = u32::from_be_bytes(self.header[1..5].try_into().unwrap());
                        if length < 4 {
                            return self.lose(state);
                        }
                        self.remaining = length as usize - 4;
                        self.captured_len = 0;
                        self.finish_if_complete(state);
                    }
                }
                Phase::Framed => {
                    let take = self.remaining.min(bytes.len());
                    let wanted = (4 - self.captured_len).min(take);
                    self.captured[self.captured_len..self.captured_len + wanted]
                        .copy_from_slice(&bytes[..wanted]);
                    self.captured_len += wanted;
                    self.remaining -= take;
                    bytes = &bytes[take..];
                    self.finish_if_complete(state);
                }
            }
        }
    }

    fn finish_if_complete(&mut self, state: &WireState) {
        if self.remaining != 0 {
            return;
        }
        match self.header[0] {
            b'Z' if self.captured_len >= 1 => match self.captured[0] {
                status @ (b'I' | b'T' | b'E') => state.status.store(status, Ordering::SeqCst),
                _ => return self.lose(state),
            },
            b'Z' => return self.lose(state),
            b'K' if self.captured_len == 4 => {
                state
                    .key_pid
                    .store(i32::from_be_bytes(self.captured), Ordering::SeqCst);
            }
            _ => {}
        }
        self.header_len = 0;
    }
}

/// The first eight bytes the client writes decide how the raw layer reads.
struct Opening {
    bytes: [u8; 8],
    len: usize,
}

/// A stream whose inbound backend messages are observed.
pub(crate) struct Observed<S> {
    inner: S,
    state: Arc<WireState>,
    parser: Parser,
    opening: Option<Opening>,
}

impl<S> Observed<S> {
    /// The plain socket, before any TLS negotiation.
    pub(crate) fn socket(inner: S, state: Arc<WireState>) -> Self {
        Self {
            inner,
            state,
            parser: Parser::new(Phase::Pending),
            opening: Some(Opening {
                bytes: [0; 8],
                len: 0,
            }),
        }
    }

    /// The decrypted stream: it carries only framed backend messages.
    fn decrypted(inner: S, state: Arc<WireState>) -> Self {
        Self {
            inner,
            state,
            parser: Parser::new(Phase::Framed),
            opening: None,
        }
    }

    fn observe_written(&mut self, written: &[u8]) {
        let Some(opening) = &mut self.opening else {
            return;
        };
        let take = (8 - opening.len).min(written.len());
        opening.bytes[opening.len..opening.len + take].copy_from_slice(&written[..take]);
        opening.len += take;
        if opening.len < 8 {
            return;
        }
        let bytes = opening.bytes;
        self.opening = None;
        self.parser.phase = if bytes == SSL_REQUEST {
            Phase::SslAnswer
        } else if STARTUP_VERSIONS
            .iter()
            .any(|version| bytes[4..8] == *version)
        {
            Phase::Framed
        } else {
            // Direct TLS or an unknown opening: the decrypted layer, if any,
            // observes instead of this one.
            Phase::HandedOff
        };
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Observed<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let polled = Pin::new(&mut this.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = polled {
            this.parser.feed(&buf.filled()[before..], &this.state);
        }
        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Observed<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.inner).poll_write(cx, data);
        if let Poll::Ready(Ok(written)) = polled {
            this.observe_written(&data[..written]);
        }
        polled
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl<S: TlsStream + Unpin> TlsStream for Observed<S> {
    fn channel_binding(&self) -> ChannelBinding {
        self.inner.channel_binding()
    }
}

/// Wraps a TLS connector so the decrypted stream is observed.
pub(crate) struct ObservedTls<T> {
    inner: T,
    state: Arc<WireState>,
}

impl<T> ObservedTls<T> {
    pub(crate) fn new(inner: T, state: Arc<WireState>) -> Self {
        Self { inner, state }
    }
}

impl<S, T> TlsConnect<S> for ObservedTls<T>
where
    T: TlsConnect<S>,
    T::Future: Send + 'static,
    T::Stream: Send + 'static,
{
    type Stream = Observed<T::Stream>;
    type Error = T::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Stream, T::Error>> + Send>>;

    fn connect(self, stream: S) -> Self::Future {
        let state = self.state;
        let connecting = self.inner.connect(stream);
        Box::pin(async move {
            let stream = connecting.await?;
            Ok(Observed::decrypted(stream, state))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(tag: u8, body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![tag];
        bytes.extend_from_slice(&(body.len() as u32 + 4).to_be_bytes());
        bytes.extend_from_slice(body);
        bytes
    }

    fn startup() -> Vec<u8> {
        let mut bytes = vec![0, 0, 0, 9, 0, 3, 0, 0, 0];
        bytes[3] = bytes.len() as u8;
        bytes
    }

    /// A server conversation: auth ok, key data, a parameter, then ready.
    fn handshake(status: u8) -> Vec<u8> {
        let mut bytes = message(b'R', &0u32.to_be_bytes());
        bytes.extend(message(b'K', &[0, 0, 0x30, 0x39, 1, 2, 3, 4]));
        bytes.extend(message(b'S', b"TimeZone\0UTC\0"));
        bytes.extend(message(b'Z', &[status]));
        bytes
    }

    fn observed(opening: &[u8]) -> Observed<()> {
        let mut observed = Observed::socket((), WireState::new());
        observed.observe_written(opening);
        observed
    }

    fn feed_in_pieces(observed: &mut Observed<()>, bytes: &[u8], piece: usize) {
        for chunk in bytes.chunks(piece) {
            observed.parser.feed(chunk, &observed.state);
        }
    }

    #[test]
    fn follows_status_and_key_across_every_read_boundary() {
        for piece in 1..=8 {
            let mut observed = observed(&startup());
            let mut bytes = handshake(b'I');
            bytes.extend(message(b'T', &[0, 0]));
            bytes.extend(message(b'C', b"BEGIN\0"));
            bytes.extend(message(b'Z', b"T"));
            feed_in_pieces(&mut observed, &bytes, piece);
            assert_eq!(
                observed.state.transaction(),
                Some(WireTransaction::InTransaction),
                "piece {piece}"
            );
            assert_eq!(observed.state.key_pid(), Some(12345));
            feed_in_pieces(&mut observed, &message(b'Z', b"E"), piece);
            assert_eq!(observed.state.transaction(), Some(WireTransaction::Failed));
        }
    }

    #[test]
    fn declined_ssl_request_continues_on_the_plain_socket() {
        let mut observed = observed(&SSL_REQUEST);
        let mut bytes = vec![b'N'];
        bytes.extend(handshake(b'I'));
        feed_in_pieces(&mut observed, &bytes, 3);
        assert_eq!(observed.state.transaction(), Some(WireTransaction::Idle));
    }

    #[test]
    fn accepted_ssl_request_hands_off_to_the_decrypted_layer() {
        let state = WireState::new();
        let mut socket = Observed::socket((), state.clone());
        socket.observe_written(&SSL_REQUEST);
        // Ciphertext after 'S' must never be read as messages.
        socket
            .parser
            .feed(b"S\x16\x03\x03\x00\x05Z\x00\x00\x00\x05T", &state);
        assert_eq!(state.transaction(), None);
        let mut decrypted = Observed::decrypted((), state.clone());
        decrypted.parser.feed(&handshake(b'I'), &state);
        assert_eq!(state.transaction(), Some(WireTransaction::Idle));
        socket.parser.feed(&message(b'Z', b"T"), &state);
        assert_eq!(state.transaction(), Some(WireTransaction::Idle));
    }

    #[test]
    fn unexpected_input_is_unknown_rather_than_guessed() {
        // A server that speaks before the client.
        let mut observed = Observed::socket((), WireState::new());
        observed.parser.feed(&handshake(b'I'), &observed.state);
        assert_eq!(observed.state.transaction(), None);

        let mut observed = observed_after_ready();
        observed.parser.feed(&[b'Z', 0, 0, 0, 2], &observed.state);
        assert_eq!(observed.state.transaction(), None);
        // Once lost, later valid frames are not trusted either.
        observed.parser.feed(&message(b'Z', b"I"), &observed.state);
        assert_eq!(observed.state.transaction(), None);

        let mut observed = observed_after_ready();
        observed.parser.feed(&message(b'Z', b"X"), &observed.state);
        assert_eq!(observed.state.transaction(), None);

        // A direct-TLS ClientHello is not a startup message.
        let mut observed = self::observed(&[0x16, 0x03, 0x01, 0x02, 0x00, 0x01, 0x00, 0x01]);
        observed.parser.feed(&handshake(b'I'), &observed.state);
        assert_eq!(observed.state.transaction(), None);
    }

    fn observed_after_ready() -> Observed<()> {
        let mut observed = observed(&startup());
        observed.parser.feed(&handshake(b'I'), &observed.state);
        assert_eq!(observed.state.transaction(), Some(WireTransaction::Idle));
        observed
    }

    #[tokio::test]
    async fn reads_and_writes_pass_through_unchanged() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (client, mut server) = tokio::io::duplex(64);
        let state = WireState::new();
        let mut observed = Observed::socket(client, state.clone());
        observed.write_all(&startup()).await.unwrap();
        let mut written = vec![0; startup().len()];
        server.read_exact(&mut written).await.unwrap();
        assert_eq!(written, startup());
        let reply = handshake(b'T');
        server.write_all(&reply).await.unwrap();
        let mut read = vec![0; reply.len()];
        observed.read_exact(&mut read).await.unwrap();
        assert_eq!(read, reply);
        assert_eq!(state.transaction(), Some(WireTransaction::InTransaction));
    }
}

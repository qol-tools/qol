use std::io::{BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use mio::{Events, Interest, Poll, Token};
use pulseaudio::protocol::{self, Command, ProtocolError, PulseError};

use crate::AudioError;

use super::environment::{self, COOKIE_LENGTH};

pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MINIMUM_SLICE: Duration = Duration::from_millis(1);
const RETRY_SLICE: Duration = Duration::from_millis(10);
const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const POLL_TOKEN: Token = Token(0);

#[derive(Debug)]
pub(super) struct Connection {
    reader: BufReader<DeadlineStream>,
    version: u16,
    request_timeout: Duration,
    seq: u32,
}

impl Connection {
    pub(super) fn connect() -> Result<Self, AudioError> {
        let settings = environment::load()?;
        Self::connect_at(&settings.socket, CONNECT_TIMEOUT, &settings.cookie)
    }

    pub(super) fn connect_at(
        path: &Path,
        timeout: Duration,
        cookie: &[u8; COOKIE_LENGTH],
    ) -> Result<Self, AudioError> {
        let deadline = Instant::now() + timeout;
        let stream = connect_stream(path, deadline)?;
        let mut connection = Self {
            reader: BufReader::new(DeadlineStream::new(stream, deadline)),
            version: protocol::MAX_VERSION,
            request_timeout: timeout,
            seq: 0,
        };
        connection.handshake(cookie)?;
        Ok(connection)
    }

    pub(super) fn version(&self) -> u16 {
        self.version
    }

    pub(super) fn request<T: protocol::CommandReply>(
        &mut self,
        command: &Command,
    ) -> Result<T, AudioError> {
        self.begin_request();
        let seq = self.next_seq();
        self.write(seq, command)?;
        self.read_reply(seq)
    }

    pub(super) fn request_ack(&mut self, command: &Command) -> Result<(), AudioError> {
        self.begin_request();
        let seq = self.next_seq();
        self.write(seq, command)?;
        self.reader.get_mut().begin_message();
        let reply = protocol::read_ack_message(&mut self.reader).map_err(from_protocol)?;
        if reply != seq {
            return Err(sequence_mismatch(reply, seq));
        }
        Ok(())
    }

    fn handshake(&mut self, cookie: &[u8; COOKIE_LENGTH]) -> Result<(), AudioError> {
        let auth = protocol::AuthParams {
            version: protocol::MAX_VERSION,
            supports_shm: false,
            supports_memfd: false,
            cookie: cookie.to_vec(),
        };
        let seq = self.next_seq();
        self.write(seq, &Command::Auth(auth))?;
        let reply: protocol::AuthReply = self.read_reply(seq)?;
        self.version = reply.version.min(protocol::MAX_VERSION);

        let mut props = protocol::Props::new();
        props.set(protocol::Prop::ApplicationName, c"qol-audio");
        let seq = self.next_seq();
        self.write(seq, &Command::SetClientName(props))?;
        let _: protocol::SetClientNameReply = self.read_reply(seq)?;
        Ok(())
    }

    fn begin_request(&mut self) {
        self.reader
            .get_mut()
            .set_deadline(Instant::now() + self.request_timeout);
    }

    fn next_seq(&mut self) -> u32 {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        seq
    }

    fn write(&mut self, seq: u32, command: &Command) -> Result<(), AudioError> {
        protocol::write_command_message(self.reader.get_mut(), seq, command, self.version)
            .map_err(from_protocol)
    }

    fn read_reply<T: protocol::CommandReply>(&mut self, expected: u32) -> Result<T, AudioError> {
        self.reader.get_mut().begin_message();
        let (seq, reply) = protocol::read_reply_message::<T>(&mut self.reader, self.version)
            .map_err(from_protocol)?;
        if seq != expected {
            return Err(sequence_mismatch(seq, expected));
        }
        Ok(reply)
    }
}

#[derive(Debug)]
pub(super) struct DeadlineStream {
    stream: UnixStream,
    deadline: Option<Instant>,
    message_bytes: usize,
}

impl DeadlineStream {
    pub(super) fn new(stream: UnixStream, deadline: Instant) -> Self {
        Self {
            stream,
            deadline: Some(deadline),
            message_bytes: 0,
        }
    }

    pub(super) fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = Some(deadline);
    }

    pub(super) fn begin_message(&mut self) {
        self.message_bytes = 0;
    }

    fn remaining(&self) -> std::io::Result<Option<Duration>> {
        let Some(deadline) = self.deadline else {
            return Ok(None);
        };
        remaining_until(deadline)
            .map(Some)
            .ok_or_else(|| std::io::Error::new(ErrorKind::TimedOut, "the audio deadline elapsed"))
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(remaining) = self.remaining()? {
            self.stream.set_read_timeout(Some(remaining))?;
        }
        let read = self.stream.read(buf)?;
        self.message_bytes += read;
        if self.message_bytes > MAX_MESSAGE_BYTES {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                format!(
                    "the audio server sent a message larger than the {MAX_MESSAGE_BYTES} byte limit"
                ),
            ));
        }
        Ok(read)
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(remaining) = self.remaining()? {
            self.stream.set_write_timeout(Some(remaining))?;
        }
        self.stream.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

pub(super) fn connect_stream(path: &Path, deadline: Instant) -> Result<UnixStream, AudioError> {
    let mut poll = Poll::new().map_err(|error| {
        AudioError::ServerUnavailable(format!("cannot create the audio poller: {error}"))
    })?;
    let mut events = Events::with_capacity(1);

    loop {
        let remaining = remaining_until(deadline).ok_or(AudioError::Timeout)?;
        match mio::net::UnixStream::connect(path) {
            Ok(stream) => return finish_connect(&mut poll, &mut events, stream, deadline),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                poll.poll(&mut events, Some(remaining.min(RETRY_SLICE)))
                    .map_err(|error| {
                        AudioError::ServerUnavailable(format!("audio connect wait failed: {error}"))
                    })?;
            }
            Err(error) => {
                return Err(AudioError::ServerUnavailable(format!(
                    "cannot connect to the audio socket: {error}"
                )));
            }
        }
    }
}

fn finish_connect(
    poll: &mut Poll,
    events: &mut Events,
    mut stream: mio::net::UnixStream,
    deadline: Instant,
) -> Result<UnixStream, AudioError> {
    poll.registry()
        .register(&mut stream, POLL_TOKEN, Interest::WRITABLE)
        .map_err(|error| {
            AudioError::ServerUnavailable(format!("cannot watch the audio socket: {error}"))
        })?;
    let remaining = remaining_until(deadline).ok_or(AudioError::Timeout)?;
    poll.poll(events, Some(remaining)).map_err(|error| {
        AudioError::ServerUnavailable(format!("audio connect wait failed: {error}"))
    })?;
    if events.is_empty() {
        return Err(AudioError::Timeout);
    }
    if let Some(error) = stream.take_error().map_err(map_io)? {
        return Err(map_io(error));
    }
    let stream = UnixStream::from(stream);
    stream.set_nonblocking(false).map_err(map_io)?;
    Ok(stream)
}

fn remaining_until(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| *remaining >= MINIMUM_SLICE)
}

pub(super) fn from_protocol(error: ProtocolError) -> AudioError {
    match error {
        ProtocolError::Io(error) => map_io(error),
        ProtocolError::Timeout => AudioError::Timeout,
        ProtocolError::ServerError(PulseError::AuthKey) => {
            AudioError::Authentication("the authentication key was rejected".to_owned())
        }
        ProtocolError::ServerError(PulseError::AccessDenied) => {
            AudioError::Authentication("the server denied access".to_owned())
        }
        ProtocolError::ServerError(error) => {
            AudioError::Operation(format!("the audio server rejected the request: {error}"))
        }
        ProtocolError::UnsupportedVersion(version) => {
            AudioError::Protocol(format!("unsupported protocol version {version}"))
        }
        ProtocolError::UnexpectedCommand(tag) => {
            AudioError::Protocol(format!("unexpected server command {tag:?}"))
        }
        ProtocolError::Invalid(reason) => AudioError::Protocol(reason),
        ProtocolError::Unimplemented(_, tag) => {
            AudioError::Protocol(format!("unimplemented command {tag:?}"))
        }
    }
}

pub(super) fn map_io(error: std::io::Error) -> AudioError {
    match error.kind() {
        ErrorKind::WouldBlock | ErrorKind::TimedOut => AudioError::Timeout,
        _ => AudioError::ServerUnavailable(format!("audio socket failure: {error}")),
    }
}

fn sequence_mismatch(received: u32, expected: u32) -> AudioError {
    AudioError::Protocol(format!(
        "received reply sequence {received} for request {expected}"
    ))
}

#[cfg(test)]
mod tests {
    use std::io::{ErrorKind, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    use super::{DeadlineStream, MAX_MESSAGE_BYTES};

    const CHUNK: usize = 1024;
    const TEST_DEADLINE: Duration = Duration::from_secs(30);

    fn pair() -> (UnixStream, DeadlineStream) {
        let (writer, reader) = UnixStream::pair().expect("in-memory audio pair");
        (
            writer,
            DeadlineStream::new(reader, Instant::now() + TEST_DEADLINE),
        )
    }

    fn read_message_bytes(stream: &mut DeadlineStream, writer: &mut UnixStream, bytes: usize) {
        let chunk = [0u8; CHUNK];
        let mut sink = [0u8; CHUNK];
        let mut remaining = bytes;
        while remaining > 0 {
            let step = remaining.min(CHUNK);
            writer.write_all(&chunk[..step]).expect("write");
            let mut filled = 0;
            while filled < step {
                let read = stream.read(&mut sink[..step - filled]).expect("read");
                assert!(read > 0, "the in-memory pair ended early");
                filled += read;
            }
            remaining -= step;
        }
    }

    #[test]
    fn a_message_at_the_byte_budget_is_accepted() {
        let (mut writer, mut stream) = pair();
        stream.begin_message();
        read_message_bytes(&mut stream, &mut writer, MAX_MESSAGE_BYTES);
    }

    #[test]
    fn a_message_over_the_byte_budget_is_refused() {
        let (mut writer, mut stream) = pair();
        stream.begin_message();
        read_message_bytes(&mut stream, &mut writer, MAX_MESSAGE_BYTES);
        writer.write_all(&[0u8]).expect("write");
        let mut byte = [0u8; 1];
        let error = stream.read(&mut byte).expect_err("the byte budget");
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.to_string().contains(&MAX_MESSAGE_BYTES.to_string()));
    }

    #[test]
    fn a_new_message_restarts_the_byte_budget() {
        let (mut writer, mut stream) = pair();
        stream.begin_message();
        read_message_bytes(&mut stream, &mut writer, MAX_MESSAGE_BYTES);
        stream.begin_message();
        read_message_bytes(&mut stream, &mut writer, MAX_MESSAGE_BYTES);
    }
}

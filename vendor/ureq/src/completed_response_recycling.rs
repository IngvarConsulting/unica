//! Fault controls at the two fallible reset operations, with the real response
//! parser, length-delimited reader, owned stream and connection pool.

use std::cell::Cell;
use std::io::{self, BufRead, BufReader, Cursor, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::body::Payload;
use crate::pool::{PoolKey, PoolReturner};
use crate::stream::{ReadWrite, Stream};
use crate::unit::Unit;
use crate::{Agent, AgentBuilder, Response};

struct ServerJoin(Option<std::thread::JoinHandle<Vec<(String, Vec<u8>)>>>);

impl ServerJoin {
    fn finish(mut self) -> Vec<(String, Vec<u8>)> {
        self.0.take().unwrap().join().unwrap()
    }
}

impl Drop for ServerJoin {
    fn drop(&mut self) {
        if let Some(thread) = self.0.take() {
            let _ = thread.join();
        }
    }
}

fn accept_bounded(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                return stream;
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            result => panic!("bounded server accept failed: {:?}", result),
        }
    }
}

fn request(reader: &mut impl BufRead) -> (String, Vec<u8>) {
    let mut request = String::new();
    assert_ne!(
        reader.read_line(&mut request).unwrap(),
        0,
        "request closed early"
    );
    let mut length = 0;
    loop {
        let mut line = String::new();
        assert_ne!(
            reader.read_line(&mut line).unwrap(),
            0,
            "request headers closed early"
        );
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap();
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    (request.trim().to_owned(), body)
}

thread_local! {
    static FAULT: Cell<Option<bool>> = const { Cell::new(None) };
    static FAULTS_SEEN: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn reset_fault(read: bool) -> io::Result<()> {
    if FAULT.with(|fault| fault.get() == Some(read)) {
        FAULTS_SEEN.with(|seen| seen.set(seen.get() + 1));
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "injected reset failure",
        ))
    } else {
        Ok(())
    }
}

struct Fault;

impl Fault {
    fn arm(read: bool) -> Self {
        FAULT.with(|fault| assert_eq!(fault.replace(Some(read)), None));
        FAULTS_SEEN.with(|seen| seen.set(0));
        Self
    }

    fn seen(&self) -> usize {
        FAULTS_SEEN.with(Cell::get)
    }
}

impl Drop for Fault {
    fn drop(&mut self) {
        FAULT.with(|fault| fault.set(None));
    }
}

/// The bytes and read failures are controlled, while a held real socket makes
/// the production reset path perform its ordinary socket operations.
#[derive(Debug)]
struct SocketReader {
    socket: TcpStream,
    bytes: Cursor<Vec<u8>>,
    max_read: usize,
    read_error_at_end: bool,
    dropped: Arc<AtomicBool>,
}

impl Read for SocketReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.read_error_at_end && self.bytes.position() as usize == self.bytes.get_ref().len() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "injected body read failure",
            ));
        }
        let amount = buf.len().min(self.max_read);
        self.bytes.read(&mut buf[..amount])
    }
}

impl Write for SocketReader {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ReadWrite for SocketReader {
    fn socket(&self) -> Option<&TcpStream> {
        Some(&self.socket)
    }
}

impl Drop for SocketReader {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn parsed_response(
    agent: &Agent,
    bytes: &[u8],
    max_read: usize,
    read_error_at_end: bool,
    deadline: Option<Instant>,
) -> (Response, Arc<AtomicBool>, TcpStream) {
    let (stream, dropped, peer) = socket_stream(agent, bytes, max_read, read_error_at_end);
    let unit = Unit::new(
        agent,
        "GET",
        &"http://example.test/".parse().unwrap(),
        vec![],
        &Payload::Empty.into_read(),
        deadline,
    );
    (
        Response::do_from_stream(stream, unit).unwrap(),
        dropped,
        peer,
    )
}

fn socket_stream(
    agent: &Agent,
    bytes: &[u8],
    max_read: usize,
    read_error_at_end: bool,
) -> (Stream, Arc<AtomicBool>, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (peer, address) = listener.accept().unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let stream = Stream::new(
        SocketReader {
            socket,
            bytes: Cursor::new(bytes.to_vec()),
            max_read,
            read_error_at_end,
            dropped: Arc::clone(&dropped),
        },
        address,
        PoolReturner::new(agent, PoolKey::from_parts("http", "example.test", 80)),
    );
    (stream, dropped, peer)
}

fn complete_response_survives_reset(read: bool, body: &str) {
    let fault = Fault::arm(read);
    let agent = Agent::new();
    let wire = format!(
        "HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let (response, dropped, _peer) =
        parsed_response(&agent, wire.as_bytes(), usize::MAX, false, None);
    assert_eq!(response.status(), 302);
    assert_eq!(response.header("Location"), Some("/next"));
    assert_eq!(response.into_string().unwrap(), body);
    assert_eq!(
        fault.seen(),
        1,
        "actual read/write reset boundary must be reached"
    );
    assert!(dropped.load(Ordering::SeqCst), "failed stream is closed");
    assert_eq!(agent.state.pool.len(), 0, "failed stream cannot be pooled");
}

#[test]
fn complete_empty_response_survives_read_reset_failure() {
    complete_response_survives_reset(true, "");
}
#[test]
fn complete_empty_response_survives_write_reset_failure() {
    complete_response_survives_reset(false, "");
}
#[test]
fn complete_buffered_response_survives_read_reset_failure() {
    complete_response_survives_reset(true, "complete body");
}
#[test]
fn complete_buffered_response_survives_write_reset_failure() {
    complete_response_survives_reset(false, "complete body");
}

#[test]
fn streaming_completion_preserves_every_final_byte_when_reset_fails() {
    for read in [true, false] {
        let fault = Fault::arm(read);
        let agent = Agent::new();
        let wire = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nabcde";
        let (response, dropped, _peer) = parsed_response(&agent, wire, 1, false, None);
        assert_eq!(fault.seen(), 0, "body is not fully buffered");
        let mut reader = response.into_reader();
        let mut body = Vec::new();
        reader.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"abcde");
        assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
        assert_eq!(fault.seen(), 1);
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(agent.state.pool.len(), 0);
    }
}

#[test]
fn incomplete_bodies_keep_read_and_eof_failures_and_never_enter_pool() {
    for fail_read in [false, true] {
        let fault = Fault::arm(true);
        let agent = Agent::new();
        let wire = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nab";
        let (response, dropped, _peer) = parsed_response(&agent, wire, 1, fail_read, None);
        let mut body = Vec::new();
        let mut reader = response.into_reader();
        let error = reader.read_to_end(&mut body).unwrap_err();
        assert_eq!(
            error.kind(),
            if fail_read {
                io::ErrorKind::ConnectionReset
            } else {
                io::ErrorKind::UnexpectedEof
            }
        );
        assert_eq!(body, b"ab");
        assert_eq!(fault.seen(), 0, "incomplete response cannot attempt reuse");
        assert_eq!(agent.state.pool.len(), 0);
        drop(reader);
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn a_deadline_before_body_completion_remains_an_error() {
    let fault = Fault::arm(true);
    let agent = Agent::new();
    let (stream, dropped, _peer) = socket_stream(&agent, b"abcde", 1, false);
    let deadline =
        crate::stream::DeadlineStream::new(stream, Some(Instant::now() - Duration::from_secs(1)));
    let mut reader =
        crate::response::LimitedRead::new(deadline, std::num::NonZeroUsize::new(5).unwrap());
    assert_eq!(
        reader.read(&mut [0; 1]).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    drop(reader);
    assert_eq!(fault.seen(), 0);
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(agent.state.pool.len(), 0);
}

#[test]
fn healthy_completed_streams_still_enter_the_pool() {
    for max_read in [1, usize::MAX] {
        let agent = Agent::new();
        let (response, dropped, _peer) = parsed_response(
            &agent,
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
            max_read,
            false,
            None,
        );
        assert_eq!(response.into_string().unwrap(), "ok");
        assert_eq!(agent.state.pool.len(), 1);
        assert!(!dropped.load(Ordering::SeqCst));
        drop(agent);
        assert!(dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn a_real_post_is_delivered_once_and_a_following_request_uses_a_fresh_connection() {
    for read in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = ServerJoin(Some(std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in [
                "HTTP/1.1 302 Found\r\nLocation: /never\r\nContent-Length: 0\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
            ] {
                let mut socket = accept_bounded(&listener);
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                requests.push(request(&mut reader));
                socket.write_all(response.as_bytes()).unwrap();
            }
            requests
        })));
        let agent = AgentBuilder::new()
            .redirects(0)
            .try_proxy_from_env(false)
            .timeout(Duration::from_secs(5))
            .build();
        let fault = Fault::arm(read);
        let response = agent
            .post(&format!("http://{address}/start"))
            .send_string("{}")
            .unwrap();
        assert_eq!(response.status(), 302);
        assert_eq!(response.header("Location"), Some("/never"));
        assert_eq!(response.into_string().unwrap(), "");
        assert_eq!(fault.seen(), 1);
        assert_eq!(agent.state.pool.len(), 0);
        drop(fault);
        assert_eq!(
            agent
                .get(&format!("http://{address}/next"))
                .call()
                .unwrap()
                .into_string()
                .unwrap(),
            "ok"
        );
        assert_eq!(
            server.finish(),
            vec![
                ("POST /start HTTP/1.1".to_owned(), b"{}".to_vec()),
                ("GET /next HTTP/1.1".to_owned(), Vec::new())
            ]
        );
    }
}

#[test]
fn a_healthy_connection_is_reused_by_the_next_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = ServerJoin(Some(std::thread::spawn(move || {
        let mut socket = accept_bounded(&listener);
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut seen = Vec::new();
        for response in [
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone",
        ] {
            seen.push(request(&mut reader));
            socket.write_all(response.as_bytes()).unwrap();
        }
        seen
    })));
    let agent = AgentBuilder::new()
        .try_proxy_from_env(false)
        .timeout(Duration::from_secs(5))
        .build();
    assert_eq!(
        agent
            .get(&format!("http://{address}/first"))
            .call()
            .unwrap()
            .into_string()
            .unwrap(),
        "ok"
    );
    assert_eq!(agent.state.pool.len(), 1);
    assert_eq!(
        agent
            .get(&format!("http://{address}/second"))
            .call()
            .unwrap()
            .into_string()
            .unwrap(),
        "done"
    );
    assert_eq!(
        server.finish(),
        vec![
            ("GET /first HTTP/1.1".to_owned(), Vec::new()),
            ("GET /second HTTP/1.1".to_owned(), Vec::new())
        ]
    );
}

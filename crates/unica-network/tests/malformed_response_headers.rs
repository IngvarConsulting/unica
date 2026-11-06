use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use unica_network::NetworkClient;

/// Serves raw response bytes, including lines a normal HTTP server would reject.
/// Drop wakes accept and joins the thread even when the client panics.
struct RawResponseStand {
    address: SocketAddr,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl RawResponseStand {
    fn start(response: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind raw HTTP stand");
        let address = listener.local_addr().expect("stand address");
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let worker = thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("bounded request read");
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("bounded response write");
                let mut reader = BufReader::new(&mut stream);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => return,
                        Ok(_) if line == "\r\n" => break,
                        Ok(_) => {}
                    }
                }
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            address,
            stopped,
            worker: Some(worker),
        }
    }

    fn get(&self) -> ureq::Response {
        NetworkClient::from_environment(&|_| None)
            .expect("client without ambient proxy settings")
            .get(
                &format!("http://{}/start", self.address),
                &[],
                Some(Duration::from_secs(5)),
            )
            .expect("malformed header line does not fail the response")
    }
}

impl Drop for RawResponseStand {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(2));
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.expect("raw HTTP stand finished");
            }
        }
    }
}

#[test]
fn a_colonless_location_is_ignored_by_the_common_network_client() {
    let stand = RawResponseStand::start(
        "HTTP/1.1 302 Found\r\nLocation\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    );
    let response = stand.get();
    assert_eq!(response.status(), 302);
    assert_eq!(response.header("Location"), None);
    assert_eq!(response.into_string().expect("empty response body"), "");
}

#[test]
fn another_colonless_header_is_ignored_while_empty_values_and_body_survive() {
    let stand = RawResponseStand::start(
        "HTTP/1.1 200 OK\r\nX-Colonless\r\nLocation:\r\nX-Valid: retained\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone",
    );
    let response = stand.get();
    assert_eq!(response.status(), 200);
    assert_eq!(response.header("X-Colonless"), None);
    assert_eq!(response.header("Location"), Some(""));
    assert_eq!(response.header("X-Valid"), Some("retained"));
    assert_eq!(response.into_string().expect("framed body"), "done");
}

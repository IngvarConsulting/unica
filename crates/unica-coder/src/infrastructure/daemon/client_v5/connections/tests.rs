use super::*;
use std::io::BufRead;
use std::net::{Ipv4Addr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

struct Endpoint {
    record: V5EndpointRecord,
    accepted: Arc<Mutex<Vec<TcpStream>>>,
    arrived: mpsc::Receiver<()>,
    completed: mpsc::Receiver<()>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Endpoint {
    fn start(malformed: bool, first_ready: Option<mpsc::Receiver<()>>) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = V5EndpointRecord::new(
            CoreIdentity::production_v5(),
            listener.local_addr().unwrap().port(),
        )
        .unwrap();
        let response = if malformed {
            b"{malformed}\n".to_vec()
        } else {
            let mut bytes = serde_json::to_vec(&V5HandshakeServerResponse::ready(&record)).unwrap();
            bytes.push(b'\n');
            bytes
        };
        let stop = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(Mutex::new(Vec::new()));
        let server_stop = Arc::clone(&stop);
        let server_accepted = Arc::clone(&accepted);
        let (arrived_tx, arrived) = mpsc::channel();
        let (completed_tx, completed) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut first_ready = first_ready;
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                if server_stop.load(Ordering::SeqCst) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut hello = Vec::new();
                if BufReader::new(&stream)
                    .read_until(b'\n', &mut hello)
                    .is_err()
                {
                    continue;
                }
                if arrived_tx.send(()).is_err() {
                    break;
                }
                if let Some(gate) = first_ready.take() {
                    if gate.recv_timeout(Duration::from_secs(2)).is_err() {
                        break;
                    }
                }
                if stream.write_all(&response).is_ok() {
                    server_accepted.lock().unwrap().push(stream);
                    let _ = completed_tx.send(());
                }
            }
        });
        Self {
            record,
            accepted,
            arrived,
            completed,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.record.loopback_addr().unwrap());
        self.worker.take().unwrap().join().unwrap();
    }
}

fn client_with_anchor(root: &Path, record: V5EndpointRecord) -> (V5DaemonClient, TcpStream) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let writer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server_anchor, _) = listener.accept().unwrap();
    let anchor = V5DaemonProcessOwner {
        reader: BufReader::new(writer.try_clone().unwrap()),
        writer,
        record,
        poisoned: false,
    };
    (
        V5DaemonClient {
            identity: CoreIdentity::production_v5(),
            configuration: Some(Configuration {
                state_root: root.to_path_buf(),
                executable: root.join("absent-daemon-executable"),
                idle_grace: Duration::from_secs(1),
            }),
            state: Mutex::new(ConnectionState {
                anchor: Arc::new(anchor),
                refreshing: false,
            }),
            refreshed: Condvar::new(),
        },
        server_anchor,
    )
}

fn unrelated_record() -> V5EndpointRecord {
    V5EndpointRecord::new(CoreIdentity::production_v5(), 1).unwrap()
}

#[test]
fn concurrent_refresh_retains_one_successor_anchor_and_honors_waiter_deadline() {
    let root = tempfile::tempdir().unwrap();
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    let directory = DaemonStateDirectory::open(&root_path, &CoreIdentity::production_v5()).unwrap();
    let (release, gate) = mpsc::channel();
    let endpoint = Endpoint::start(false, Some(gate));
    directory
        .publish_v5_endpoint_record(&endpoint.record)
        .unwrap();
    let (client, _old_anchor) = client_with_anchor(&root_path, unrelated_record());
    let client = Arc::new(client);
    let deadline = Instant::now() + Duration::from_secs(3);
    let first_client = Arc::clone(&client);
    let first = thread::spawn(move || first_client.connect_peer_before(deadline));
    endpoint
        .arrived
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(client.state.lock().unwrap().refreshing);

    let followers: Vec<_> = (0..4)
        .map(|_| {
            let client = Arc::clone(&client);
            thread::spawn(move || client.connect_peer_before(deadline))
        })
        .collect();
    let short_client = Arc::clone(&client);
    let short = thread::spawn(move || {
        short_client.connect_peer_before(Instant::now() + Duration::from_millis(30))
    });
    let short_result = short.join().unwrap();
    assert!(matches!(
        short_result,
        Err(V5TransportError::RequestNotSent(_))
    ));
    assert!(
        client.state.lock().unwrap().refreshing,
        "the waiting caller must time out before refresh completes"
    );
    release.send(()).unwrap();
    let mut peers = vec![first.join().unwrap().unwrap()];
    for follower in followers {
        peers.push(follower.join().unwrap().unwrap());
    }
    assert!(peers.iter().all(|peer| peer.record == endpoint.record));
    drop(peers);
    let retained = client.state.lock().unwrap();
    assert_eq!(retained.anchor.record, endpoint.record);
    assert!(!retained.refreshing);
    drop(retained);
    for _ in 0..6 {
        endpoint
            .completed
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
    }
    let accepted = endpoint.accepted.lock().unwrap();
    assert_eq!(
        accepted.len(),
        6,
        "one replacement anchor plus five operation peers"
    );
    accepted[0].set_nonblocking(true).unwrap();
    let mut byte = [0];
    assert_eq!(
        accepted[0].peek(&mut byte).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(client);
    accepted[0].set_nonblocking(false).unwrap();
    accepted[0]
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    assert_eq!(
        accepted[0].peek(&mut byte).unwrap(),
        0,
        "dropping the client releases the successor anchor"
    );
}

#[test]
fn malformed_handshake_preserves_endpoint_without_attempting_spawn() {
    for changed_endpoint in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let root_path = std::fs::canonicalize(root.path()).unwrap();
        let directory =
            DaemonStateDirectory::open(&root_path, &CoreIdentity::production_v5()).unwrap();
        let endpoint = Endpoint::start(true, None);
        directory
            .publish_v5_endpoint_record(&endpoint.record)
            .unwrap();
        let initial = if changed_endpoint {
            unrelated_record()
        } else {
            endpoint.record.clone()
        };
        let (client, _old_anchor) = client_with_anchor(&root_path, initial.clone());
        let result = client.connect_peer_before(Instant::now() + Duration::from_secs(1));
        let Err(V5TransportError::RequestNotSent(message)) = result else {
            panic!("malformed handshake must fail")
        };
        assert_eq!(message, "protocol-v5 handshake response is not strict JSON");
        assert_eq!(
            directory.read_v5_endpoint_record().unwrap(),
            Some(endpoint.record.clone())
        );
        assert_eq!(client.state.lock().unwrap().anchor.record, initial);
        assert!(!client.state.lock().unwrap().refreshing);
    }
}

#[test]
fn expired_acquisition_does_not_create_discovery_state() {
    let root = tempfile::tempdir().unwrap();
    let absent_root = root.path().join("not-yet-created");
    let (client, _old_anchor) = client_with_anchor(&absent_root, unrelated_record());
    let result = client.connect_peer_before(Instant::now());
    assert!(matches!(result, Err(V5TransportError::RequestNotSent(_))));
    assert!(!absent_root.exists());
    assert!(!client.state.lock().unwrap().refreshing);
}

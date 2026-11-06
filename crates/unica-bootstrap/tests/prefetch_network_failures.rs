//! rule: INV.PKG.STARTUP-FAILURE-DIAGNOSIS

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use flate2::{write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use tar::{Builder, Header};
use unica_artifact_delivery::{
    DownloadObserver, Downloader, HostTarget, RuntimeInstaller, RuntimeManifest, SilentDownload,
};
use uuid::Uuid;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn archive(path: &str, bytes: &[u8]) -> Vec<u8> {
    let mut tar = Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, path, bytes).unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

struct BytesDownloader(BTreeMap<String, Vec<u8>>);

impl Downloader for BytesDownloader {
    fn download(
        &self,
        url: &str,
        destination: &Path,
        _observer: &dyn DownloadObserver,
    ) -> unica_artifact_delivery::Result<()> {
        let bytes = self
            .0
            .get(url)
            .expect("only the named fixture asset may be warmed");
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::write(destination, bytes)?;
        Ok(())
    }
}

struct Fixture {
    scratch: PathBuf,
    plugin: PathBuf,
    cache: PathBuf,
    target_artifact: &'static str,
    target_url: String,
}

impl Fixture {
    fn new(target_artifact: &'static str) -> Self {
        let scratch =
            std::env::temp_dir().join(format!("unica-prefetch-network-{}", Uuid::new_v4()));
        let plugin = scratch.join("plugin");
        let cache = scratch.join("cache");
        fs::create_dir_all(&plugin).unwrap();
        let version = env!("CARGO_PKG_VERSION");
        let mut core_targets = serde_json::Map::new();
        let mut engine_targets = serde_json::Map::new();
        let mut assets = BTreeMap::new();
        for host in HostTarget::ALL {
            let target = host.as_str();
            let core_path = format!("bin/{target}/{}", host.executable_name());
            let core = b"controlled core bytes";
            let core_archive = archive(&core_path, core);
            let core_name = format!("unica-runtime-{target}.tar.gz");
            let core_url = format!("https://github.com/IngvarConsulting/unica/releases/download/v{version}/{core_name}");
            assets.insert(core_url.clone(), core_archive.clone());
            core_targets.insert(target.into(), serde_json::json!({
                "asset": {"name":core_name,"url":core_url,"mediaType":"application/gzip","sha256":digest(&core_archive)},
                "files":[{"path":core_path,"sha256":digest(core),"executable":true}],
                "entrypoint":core_path
            }));
            let engine_path = format!("bin/{target}/rlm-bsl-index");
            let engine = b"controlled engine bytes";
            let engine_archive = archive(&engine_path, engine);
            let engine_name = format!("rlm-tools-bsl-{target}.tar.gz");
            let engine_url = format!("https://github.com/IngvarConsulting/unica-toolchain/releases/download/rlm-tools-bsl-v1.33.0-build.3/{engine_name}");
            assets.insert(engine_url.clone(), engine_archive.clone());
            engine_targets.insert(target.into(), serde_json::json!({
                "asset":{"name":engine_name,"url":engine_url,"mediaType":"application/gzip","sha256":digest(&engine_archive)},
                "files":[{"path":engine_path,"sha256":digest(engine),"executable":true}]
            }));
        }
        let mut artifacts = serde_json::Map::new();
        artifacts.insert(
            "unica".into(),
            serde_json::json!({"version":version,"role":"core","targets":core_targets}),
        );
        if target_artifact != "unica" {
            artifacts.insert(
                "rlm-tools-bsl".into(),
                serde_json::json!({"version":"1.33.0","role":"engine","targets":engine_targets}),
            );
        }
        let manifest: RuntimeManifest = serde_json::from_value(serde_json::json!({
            "schemaVersion":2,"pluginVersion":version,
            "source":{"repository":"https://github.com/IngvarConsulting/unica","commit":COMMIT},
            "release":{"repository":"https://github.com/IngvarConsulting/unica","tag":format!("v{version}")},
            "artifacts":artifacts
        })).unwrap();
        manifest.validate(version).unwrap();
        let host = HostTarget::current().unwrap();
        let target_url = manifest
            .artifact_target(target_artifact, host)
            .unwrap()
            .asset
            .url
            .clone();
        if target_artifact != "unica" {
            RuntimeInstaller::new(cache.clone(), version, Arc::new(BytesDownloader(assets)))
                .ensure_artifact(&manifest, "unica", host, &SilentDownload)
                .unwrap();
        }
        fs::write(
            plugin.join("runtime-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        Self {
            scratch,
            plugin,
            cache,
            target_artifact,
            target_url,
        }
    }

    fn command(&self, proxy: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_unica-bootstrap"));
        command
            .args(["prefetch", "--plugin-root"])
            .arg(&self.plugin)
            .env("UNICA_RUNTIME_CACHE_DIR", &self.cache)
            .env("UNICA_TEST_NETWORK_CONNECT_WAIT_MS", "10000")
            .env("UNICA_TEST_NETWORK_READ_WAIT_MS", "10000")
            .env("HTTPS_PROXY", proxy)
            .env("HTTP_PROXY", proxy)
            .env("ALL_PROXY", proxy)
            .env("NO_PROXY", "");
        for variable in [
            "https_proxy",
            "http_proxy",
            "all_proxy",
            "no_proxy",
            "UNICA_RUNTIME_MANIFEST",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
        ] {
            command.env_remove(variable);
        }
        command
    }

    fn assert_failure(&self, output: &Output, code: i32, reason: &str) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{}: {stderr}",
            self.target_artifact
        );
        assert!(stderr.contains(&format!("reason: {reason}")), "{stderr}");
        assert!(
            stderr.contains(&self.target_url),
            "failure must name the actual selected asset: {stderr}"
        );
        assert!(
            stderr.contains("cure:"),
            "failure must name a recovery action: {stderr}"
        );
        assert!(
            output.stdout.is_empty(),
            "failed prefetch must not print success"
        );
        let target_cache = self.cache.join(self.target_artifact);
        assert!(
            !contains_ready_marker(&target_cache),
            "failed artifact was marked ready"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

fn contains_ready_marker(root: &Path) -> bool {
    if !root.exists() {
        return false;
    }
    fs::read_dir(root).unwrap().any(|entry| {
        let entry = entry.unwrap();
        entry.file_name() == ".ready.json"
            || (entry.path().is_dir() && contains_ready_marker(&entry.path()))
    })
}

enum ConnectBehavior {
    Refuse,
    Stall,
    Tunnel(String),
}

struct ConnectStand {
    address: std::net::SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
    active: Arc<Mutex<Vec<TcpStream>>>,
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl ConnectStand {
    fn start(behavior: ConnectBehavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let active = Arc::new(Mutex::new(Vec::new()));
        let (stop, stopped) = mpsc::channel();
        let observed = Arc::clone(&seen);
        let connected = Arc::clone(&active);
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            connected.lock().unwrap().push(stream.try_clone().unwrap());
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            observed.lock().unwrap().push(line.trim().to_owned());
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
            }
            match behavior {
                ConnectBehavior::Stall => {
                    // The client must expire its network wait; the stand sends no reply.
                    let _ = stopped.recv();
                }
                ConnectBehavior::Refuse => {
                    let _ = stream.write_all(b"HTTP/1.1 502 Controlled refusal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                }
                ConnectBehavior::Tunnel(authority) => {
                    let mut upstream = TcpStream::connect(authority).unwrap();
                    connected
                        .lock()
                        .unwrap()
                        .push(upstream.try_clone().unwrap());
                    upstream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    upstream
                        .set_write_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    stream
                        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .unwrap();
                    upstream.write_all(reader.buffer()).unwrap();
                    let mut upstream_read = upstream.try_clone().unwrap();
                    let mut client_write = stream.try_clone().unwrap();
                    let downstream = thread::spawn(move || {
                        let _ = std::io::copy(&mut upstream_read, &mut client_write);
                        let _ = client_write.shutdown(Shutdown::Write);
                    });
                    let _ = std::io::copy(&mut reader.into_inner(), &mut upstream);
                    let _ = upstream.shutdown(Shutdown::Write);
                    downstream.join().expect("proxy relay exits");
                }
            }
        });
        Self {
            address,
            seen,
            active,
            stop: Some(stop),
            worker: Some(worker),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    fn assert_github_connect(&self) {
        assert_eq!(
            self.seen.lock().unwrap().as_slice(),
            ["CONNECT github.com:443 HTTP/1.1"]
        );
    }
}

impl Drop for ConnectStand {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        for stream in self.active.lock().unwrap().iter() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        // Wake accept even if CLI validation failed before opening a socket.
        let _ = TcpStream::connect(self.address);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("CONNECT stand exits");
        }
    }
}

/// rule: INV.PKG.PREFETCH-EXIT-STATUS
#[test]
fn prefetch_cli_retains_network_refusal_for_core_and_engine() {
    for artifact in ["unica", "rlm-tools-bsl"] {
        let fixture = Fixture::new(artifact);
        let stand = ConnectStand::start(ConnectBehavior::Refuse);
        let output = fixture.command(&stand.url()).output().unwrap();
        stand.assert_github_connect();
        fixture.assert_failure(&output, 69, "network");
    }
}

/// rule: INV.PKG.PREFETCH-EXIT-STATUS
#[test]
fn prefetch_cli_retains_network_wait_expiry_for_core_and_engine() {
    for artifact in ["unica", "rlm-tools-bsl"] {
        let fixture = Fixture::new(artifact);
        let stand = ConnectStand::start(ConnectBehavior::Stall);
        let output = fixture
            .command(&stand.url())
            .env("UNICA_TEST_NETWORK_READ_WAIT_MS", "2000")
            .output()
            .unwrap();
        stand.assert_github_connect();
        fixture.assert_failure(&output, 75, "timeout");
    }
}

#[path = "platform/prefetch_network_failures.rs"]
mod platform;

/// rule: INV.PKG.STARTUP-FAILURE-DIAGNOSIS
#[test]
fn prefetch_cli_names_the_cache_when_preparation_fails_before_delivery() {
    for artifact in ["unica", "rlm-tools-bsl"] {
        let fixture = Fixture::new(artifact);
        let blocked = fixture.cache.join(".partial").join(artifact);
        fs::create_dir_all(blocked.parent().unwrap()).unwrap();
        fs::write(&blocked, b"keep this file").unwrap();
        let stand = ConnectStand::start(ConnectBehavior::Refuse);
        let output = fixture.command(&stand.url()).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(74), "{stderr}");
        assert!(stderr.contains("reason: disk"), "{stderr}");
        assert!(
            stderr.contains(&fixture.cache.display().to_string()),
            "{stderr}"
        );
        assert!(stderr.contains("cure:"), "{stderr}");
        assert!(stand.seen.lock().unwrap().is_empty());
        assert_eq!(fs::read(blocked).unwrap(), b"keep this file");
        assert!(!contains_ready_marker(&fixture.cache.join(artifact)));
    }
}

fn assert_trusted_cli_tls_failure(stall_body: bool) {
    use unica_network::test_support::{TestRoot, TlsTransferReply, TlsTransferStand};
    for artifact in ["unica", "rlm-tools-bsl"] {
        let fixture = Fixture::new(artifact);
        let root = TestRoot::generate("unica-prefetch-cli-root");
        let root_file = fixture.scratch.join("root.pem");
        root.write_pem(&root_file);
        let reply = if stall_body {
            TlsTransferReply::StalledBody {
                prefix: b"partial compressed archive".to_vec(),
                total_bytes: 1024,
            }
        } else {
            TlsTransferReply::Complete {
                body: b"wrong archive bytes".to_vec(),
                content_type: "application/gzip".into(),
            }
        };
        let stand = TlsTransferStand::start(&root, "github.com", reply);
        let proxy = ConnectStand::start(ConnectBehavior::Tunnel(stand.authority()));
        let empty_cert_directory = fixture.scratch.join("empty-cert-directory");
        fs::create_dir_all(&empty_cert_directory).unwrap();
        let output = fixture
            .command(&proxy.url())
            .env("SSL_CERT_FILE", &root_file)
            .env("SSL_CERT_DIR", empty_cert_directory)
            .env(
                "UNICA_TEST_NETWORK_READ_WAIT_MS",
                if stall_body { "5000" } else { "10000" },
            )
            .output()
            .unwrap();
        proxy.assert_github_connect();
        let path = fixture
            .target_url
            .strip_prefix("https://github.com")
            .unwrap();
        assert_eq!(
            stand.requests(),
            [format!("GET {path} HTTP/1.1")],
            "TLS handshake and real asset GET must precede this failure"
        );
        let (code, reason) = if stall_body {
            (75, "timeout")
        } else {
            (76, "checksum")
        };
        fixture.assert_failure(&output, code, reason);
    }
}

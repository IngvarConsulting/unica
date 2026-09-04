//! Доставка движка по требованию.
//!
//! Вызов, которому нужен ещё не доставленный движок, не отказывает: он ждёт
//! столько, сколько разумно, и, не дождавшись, отвечает состоянием. Доставка
//! принадлежит серверу, а не вызову, который её застал, — она возобновляема,
//! переживает отмену и достаётся следующему.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use unica_bootstrap::{
    BootstrapError, DeliveryForm, DownloadObserver, Failure, HostTarget, HttpDownloader,
    RuntimeInstaller, RuntimeManifest,
};

use crate::application::shared_work::{
    ArtifactReady, DeliveryFailure, DeliveryFailureClass, DeliveryWorkKey, EngineDeliveryState,
    SharedWork, SharedWorkError, SharedWorkKey, SharedWorkLifetime, SharedWorkProgress,
    SharedWorkSnapshot,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::progress::{ProgressEvent, ProgressSink};

/// Ключ, под которым ход доставки едет в `notifications/progress`.
pub(crate) const DELIVERY_PROGRESS_META_KEY: &str = "io.unica/deliveryProgress";

/// Как часто вызывающий узнаёт, что доставка идёт.
///
/// Загрузчик считает байты каждые 64 КБ — на быстром канале это сотни сообщений
/// в секунду. Вызывающему столько не нужно, а хосту столько вредно.
const DELIVERY_PROGRESS_STEP: Duration = Duration::from_millis(500);

/// Кто ведёт учёт идущих доставок.
pub(crate) struct DeliveryDesk {
    exact: SharedWork<ArtifactReady, DeliveryFailure>,
}

impl Default for DeliveryDesk {
    fn default() -> Self {
        Self {
            exact: SharedWork::new(SharedWorkLifetime::ProducerBound),
        }
    }
}

impl DeliveryDesk {
    #[allow(dead_code)] // Canonical daemon handlers join here after durable task handoff.
    pub(crate) fn join<W>(
        &self,
        key: DeliveryWorkKey,
        work: W,
    ) -> crate::application::shared_work::SharedWorkLease<ArtifactReady, DeliveryFailure>
    where
        W: FnOnce(
                crate::application::shared_work::SharedWorkProducer,
            ) -> Result<ArtifactReady, DeliveryFailure>
            + Send
            + 'static,
    {
        let expected = key.clone();
        self.exact
            .join_or_start(SharedWorkKey::from(&key), move |producer| {
                let ready = work(producer)?;
                if ready.identity() != &expected {
                    return Err(DeliveryFailure::new(
                        DeliveryFailureClass::Internal,
                        "artifact-ready identity differs from exact delivery key",
                    ));
                }
                Ok(ready)
            })
    }

    pub(crate) fn request<W>(
        &self,
        key: DeliveryWorkKey,
        work: W,
        window: Duration,
        cancellation: &CancellationToken,
        progress: &dyn ProgressSink,
    ) -> EngineDeliveryState
    where
        W: FnOnce(
                crate::application::shared_work::SharedWorkProducer,
            ) -> Result<ArtifactReady, DeliveryFailure>
            + Send
            + 'static,
    {
        self.request_with_poll_step(
            key,
            work,
            window,
            cancellation,
            progress,
            DELIVERY_PROGRESS_STEP,
        )
    }

    fn request_with_poll_step<W>(
        &self,
        key: DeliveryWorkKey,
        work: W,
        window: Duration,
        cancellation: &CancellationToken,
        progress: &dyn ProgressSink,
        poll_step: Duration,
    ) -> EngineDeliveryState
    where
        W: FnOnce(
                crate::application::shared_work::SharedWorkProducer,
            ) -> Result<ArtifactReady, DeliveryFailure>
            + Send
            + 'static,
    {
        let artifact = key.artifact().to_string();
        let lease = self.join(key, work);
        let wait_window = if lease.started_here() {
            window
        } else {
            Duration::ZERO
        };
        let deadline = Instant::now() + wait_window;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let cancelled_before_wait = cancellation.is_cancelled();
            let wait = if cancelled_before_wait {
                Duration::ZERO
            } else {
                remaining.min(poll_step)
            };
            let snapshot = lease.wait_timeout(wait);
            match snapshot {
                SharedWorkSnapshot::Ready(ready) => return EngineDeliveryState::Ready(ready),
                SharedWorkSnapshot::Failed(error) => {
                    let failure = match &*error {
                        SharedWorkError::Producer(failure) => Arc::clone(failure),
                        SharedWorkError::ProducerPanicked => Arc::new(DeliveryFailure::new(
                            DeliveryFailureClass::Internal,
                            "engine delivery worker panicked",
                        )),
                        SharedWorkError::ProducerSpawnFailed => Arc::new(DeliveryFailure::new(
                            DeliveryFailureClass::Internal,
                            "engine delivery worker could not start",
                        )),
                    };
                    return EngineDeliveryState::Failed {
                        artifact: artifact.clone(),
                        failure,
                    };
                }
                SharedWorkSnapshot::Running {
                    progress: moved,
                    elapsed,
                } => {
                    if cancellation.is_cancelled() || remaining.is_zero() {
                        return EngineDeliveryState::Working {
                            artifact: artifact.clone(),
                            received: moved.completed,
                            total: moved.total,
                            poll_interval_ms: poll_hint(moved, elapsed),
                        };
                    }
                    publish_exact(&artifact, moved, progress);
                }
            }
        }
    }
}

fn poll_hint(progress: SharedWorkProgress, elapsed: Duration) -> Option<u64> {
    let total = progress.total?;
    if progress.completed == 0 || progress.completed >= total || elapsed.is_zero() {
        return None;
    }
    let remaining =
        (total - progress.completed) as f64 / (progress.completed as f64 / elapsed.as_secs_f64());
    Some((remaining * 1000.0).max(1000.0) as u64)
}

fn publish_exact(artifact: &str, moved: SharedWorkProgress, progress: &dyn ProgressSink) {
    progress.publish(ProgressEvent {
        meta_key: DELIVERY_PROGRESS_META_KEY,
        payload: json!({
            "artifact": artifact,
            "receivedBytes": moved.completed,
            "totalBytes": moved.total,
        }),
        progress: moved.completed as f64,
        total: moved.total.unwrap_or(0) as f64,
        message: match moved.total {
            Some(total) => format!(
                "delivering {artifact}: {} of {total} bytes",
                moved.completed
            ),
            None => format!("delivering {artifact}: {} bytes", moved.completed),
        },
    });
}

/// Что нужно доставить, чтобы у инструмента появился движок.
///
/// Заказ существует только там, где источник известен: в кеше артефактов и в
/// манифесте поставки. Исходный чекаут ни того, ни другого не несёт — там
/// инструменты собирает `scripts/ci/build-unica-tools.py`, и доставке взяться
/// неоткуда.
pub(crate) struct EngineOrder {
    artifact: String,
    pub(crate) manifest_path: PathBuf,
    cache_root: PathBuf,
}

pub(crate) struct PreparedEngineOrder {
    artifact: String,
    manifest: RuntimeManifest,
    host: HostTarget,
    cache_root: PathBuf,
    identity: DeliveryWorkKey,
}

/// Куда bootstrap ставит артефакты. Без неё рантайм запущен не загрузчиком, и
/// доставлять некуда.
const ARTIFACT_CACHE_ENV: &str = "UNICA_ARTIFACT_CACHE";

/// Где лежит манифест поставки.
///
/// В установленной поставке своим корнем рантайм считает каталог установки в
/// кеше: манифест инструментов упакован в архив ядра и лежит там. Релизный
/// манифест туда не попадает — он остался в каталоге плагина, и путь к нему
/// передаёт загрузчик. В дереве разработки загрузчика нет, и манифест ищется
/// рядом с корнем по-прежнему.
const RUNTIME_MANIFEST_ENV: &str = "UNICA_RUNTIME_MANIFEST";

pub(crate) fn order_for(plugin_root: &Path, tool_name: &str) -> Option<EngineOrder> {
    order_for_in(plugin_root, tool_name, &|name| std::env::var_os(name))
}

/// Разрешение с явно названным окружением: так его видно в тесте.
fn order_for_in(
    plugin_root: &Path,
    tool_name: &str,
    read_env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<EngineOrder> {
    let cache_root = read_env(ARTIFACT_CACHE_ENV).map(PathBuf::from)?;
    let artifact = crate::infrastructure::bundled_tools::artifact_for(plugin_root, tool_name)?;
    Some(EngineOrder {
        artifact,
        manifest_path: read_env(RUNTIME_MANIFEST_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| plugin_root.join("runtime-manifest.json")),
        cache_root,
    })
}

impl EngineOrder {
    pub(crate) fn artifact(&self) -> &str {
        &self.artifact
    }

    pub(crate) fn prepare(self) -> Result<PreparedEngineOrder, DeliveryFailure> {
        let manifest = RuntimeManifest::load(&self.manifest_path).map_err(classify_failure)?;
        manifest
            .validate(env!("CARGO_PKG_VERSION"))
            .map_err(classify_failure)?;
        let host = HostTarget::current().map_err(classify_failure)?;
        let artifact = manifest
            .artifact(&self.artifact)
            .map_err(classify_failure)?;
        let target = manifest
            .artifact_target(&self.artifact, host)
            .map_err(classify_failure)?;
        let form = match DeliveryForm::of(&target.asset.media_type) {
            Some(DeliveryForm::Archive) => {
                crate::application::shared_work::DeliveryFormIdentity::Archive
            }
            Some(DeliveryForm::File) => crate::application::shared_work::DeliveryFormIdentity::File,
            None => {
                return Err(DeliveryFailure::new(
                    DeliveryFailureClass::Configuration,
                    format!(
                        "artifact {} has unsupported delivery media type {}",
                        self.artifact, target.asset.media_type
                    ),
                ))
            }
        };
        let identity = DeliveryWorkKey::new(
            self.artifact.clone(),
            artifact.version.clone(),
            host.as_str(),
            target.asset.sha256.clone(),
            form,
        )?;
        Ok(PreparedEngineOrder {
            artifact: self.artifact,
            manifest,
            host,
            cache_root: self.cache_root,
            identity,
        })
    }
}

impl PreparedEngineOrder {
    pub(crate) fn identity(&self) -> &DeliveryWorkKey {
        &self.identity
    }

    /// Доставить артефакт тем же установщиком, что ставит ядро.
    pub(crate) fn acquire(
        self,
        delivery: crate::application::shared_work::SharedWorkProducer,
    ) -> Result<ArtifactReady, DeliveryFailure> {
        RuntimeInstaller::new(
            self.cache_root,
            env!("CARGO_PKG_VERSION"),
            Arc::new(HttpDownloader::default()),
        )
        .ensure_artifact(
            &self.manifest,
            &self.artifact,
            self.host,
            &WatchingExact(delivery),
        )
        .map_err(classify_failure)
        .and_then(|root| ArtifactReady::new(self.identity, root))
    }
}

fn classify_failure(error: BootstrapError) -> DeliveryFailure {
    let class = match error.failure() {
        Failure::Network => DeliveryFailureClass::Network,
        Failure::Timeout => DeliveryFailureClass::Timeout,
        Failure::Disk => DeliveryFailureClass::Disk,
        Failure::Checksum => DeliveryFailureClass::Checksum,
        Failure::Configuration => DeliveryFailureClass::Configuration,
        Failure::Internal => DeliveryFailureClass::Internal,
    };
    DeliveryFailure::new(class, error.to_string())
}

struct WatchingExact(crate::application::shared_work::SharedWorkProducer);

impl DownloadObserver for WatchingExact {
    fn transferred(&self, received: u64, total: Option<u64>) {
        self.0.report(received, total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::application::shared_work::{
        ArtifactReady, DeliveryFailure, DeliveryFailureClass, DeliveryFormIdentity,
        DeliveryWorkKey, ProviderHostKey, SharedWorkKey,
    };
    use crate::domain::progress::NoopProgressSink;

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    #[derive(Default)]
    struct Heard {
        events: Mutex<Vec<ProgressEvent>>,
    }

    impl ProgressSink for Heard {
        fn publish(&self, event: ProgressEvent) {
            self.events.lock().expect("events").push(event);
        }
    }

    fn desk() -> DeliveryDesk {
        DeliveryDesk::default()
    }

    fn absolute_install_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("unica-engine-delivery-{name}"));
        assert!(root.is_absolute(), "test install root must be absolute");
        root
    }

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        let entries = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), std::ffi::OsString::from(*value)))
            .collect::<std::collections::BTreeMap<_, _>>();
        move |name: &str| entries.get(name).cloned()
    }

    fn plugin_root_with_tool(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("unica-order-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("third-party")).expect("plugin root");
        std::fs::write(
            root.join("third-party/manifest.json"),
            r#"{"schemaVersion":2,"tools":[{"name":"rlm-bsl-index","version":"1.33.0","artifact":"rlm-tools-bsl"}]}"#,
        )
        .expect("tool manifest");
        root
    }

    #[test]
    fn the_release_manifest_is_taken_from_where_the_bootstrap_put_it() {
        // В установленной поставке корень рантайма — каталог в кеше, и
        // релизного манифеста рядом нет. Искать его там значит не доставить
        // ничего и не сказать почему.
        let plugin_root = plugin_root_with_tool("handed");

        let order = order_for_in(
            &plugin_root,
            "rlm-bsl-index",
            &environment(&[
                ("UNICA_ARTIFACT_CACHE", "/cache"),
                (
                    "UNICA_RUNTIME_MANIFEST",
                    "/plugins/unica/runtime-manifest.json",
                ),
            ]),
        )
        .expect("заказ собран");

        assert_eq!(
            order.manifest_path,
            PathBuf::from("/plugins/unica/runtime-manifest.json")
        );
        assert_eq!(order.artifact(), "rlm-tools-bsl");
        std::fs::remove_dir_all(&plugin_root).ok();
    }

    #[test]
    fn a_development_tree_still_looks_beside_the_root() {
        let plugin_root = plugin_root_with_tool("dev-tree");

        let order = order_for_in(
            &plugin_root,
            "rlm-bsl-index",
            &environment(&[("UNICA_ARTIFACT_CACHE", "/cache")]),
        )
        .expect("заказ собран");

        assert_eq!(
            order.manifest_path,
            plugin_root.join("runtime-manifest.json")
        );
        std::fs::remove_dir_all(&plugin_root).ok();
    }

    #[test]
    fn without_an_artifact_cache_there_is_nowhere_to_deliver() {
        let plugin_root = plugin_root_with_tool("no-cache");

        assert!(order_for_in(&plugin_root, "rlm-bsl-index", &environment(&[])).is_none());
        std::fs::remove_dir_all(&plugin_root).ok();
    }

    const WIDE: Duration = Duration::from_secs(5);

    fn exact_delivery_key(sha256: char) -> DeliveryWorkKey {
        DeliveryWorkKey::new(
            "rlm-tools-bsl",
            "1.33.0",
            "darwin-arm64",
            sha256.to_string().repeat(64),
            DeliveryFormIdentity::Archive,
        )
        .expect("valid delivery key")
    }

    #[test]
    fn exact_delivery_progress_is_projected_to_the_owning_waiter() {
        let desk = desk();
        let key = exact_delivery_key('0');
        let release = Arc::new(AtomicBool::new(false));
        let heard = Heard::default();
        let outcome = desk.request(
            key.clone(),
            {
                let release = Arc::clone(&release);
                move |producer| {
                    producer.report(1024, Some(4096));
                    while !release.load(Ordering::SeqCst) {
                        thread::yield_now();
                    }
                    ArtifactReady::new(key, absolute_install_root("progress"))
                }
            },
            Duration::from_millis(20),
            &CancellationToken::new(),
            &heard,
        );
        release.store(true, Ordering::SeqCst);

        assert!(matches!(
            outcome,
            EngineDeliveryState::Working {
                received: 1024,
                total: Some(4096),
                ..
            }
        ));
        let events = heard.events.lock().expect("events");
        assert!(events.iter().all(|event| {
            event.meta_key == DELIVERY_PROGRESS_META_KEY
                && event
                    .payload
                    .get("artifact")
                    .and_then(|value| value.as_str())
                    == Some("rlm-tools-bsl")
        }));
    }

    #[test]
    fn cancelling_one_delivery_follower_does_not_stop_the_process_owned_producer() {
        let desk = Arc::new(desk());
        let key = exact_delivery_key('9');
        let producers = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let owner = {
            let desk = Arc::clone(&desk);
            let key = key.clone();
            let producers = Arc::clone(&producers);
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            thread::spawn(move || {
                desk.request(
                    key.clone(),
                    move |producer| {
                        producers.fetch_add(1, Ordering::SeqCst);
                        started.store(true, Ordering::SeqCst);
                        while !release.load(Ordering::SeqCst) {
                            assert!(
                                !producer.is_cancelled(),
                                "process-owned delivery inherited follower cancellation"
                            );
                            thread::yield_now();
                        }
                        ArtifactReady::new(key, absolute_install_root("owner"))
                    },
                    WIDE,
                    &CancellationToken::new(),
                    &NoopProgressSink,
                )
            })
        };
        while !started.load(Ordering::SeqCst) {
            thread::yield_now();
        }
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let follower = desk.request(
            key,
            |_| panic!("a follower must not start another producer"),
            WIDE,
            &cancelled,
            &NoopProgressSink,
        );
        assert!(matches!(follower, EngineDeliveryState::Working { .. }));

        release.store(true, Ordering::SeqCst);
        assert!(matches!(
            owner.join().unwrap(),
            EngineDeliveryState::Ready(_)
        ));
        assert_eq!(producers.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn pre_cancelled_delivery_returns_before_polling_and_never_publishes_progress() {
        let desk = Arc::new(desk());
        let key = exact_delivery_key('8');
        let producers = Arc::new(AtomicUsize::new(0));
        let progress = Arc::new(Heard::default());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let requester = {
            let desk = Arc::clone(&desk);
            let key = key.clone();
            let producers = Arc::clone(&producers);
            let progress = Arc::clone(&progress);
            thread::spawn(move || {
                let outcome = desk.request_with_poll_step(
                    key.clone(),
                    move |_| {
                        producers.fetch_add(1, Ordering::SeqCst);
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        ArtifactReady::new(key, absolute_install_root("pre-cancelled"))
                    },
                    WIDE,
                    &cancelled,
                    progress.as_ref(),
                    Duration::from_secs(30),
                );
                result_tx.send(outcome).unwrap();
            })
        };
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("producer starts independently of the cancelled observer");

        let early = result_rx.recv_timeout(Duration::from_secs(2));
        if early.is_err() {
            release_tx.send(()).unwrap();
            requester.join().unwrap();
            panic!("a pre-cancelled observer entered the injected 30 s progress polling step");
        }
        let outcome = early.unwrap();
        assert!(matches!(outcome, EngineDeliveryState::Working { .. }));
        assert!(
            progress.events.lock().expect("events").is_empty(),
            "a pre-cancelled observer must not publish delivery progress"
        );

        let follower = desk.join(key, |_| {
            panic!("the non-cancelled follower must join the process-owned producer")
        });
        release_tx.send(()).unwrap();
        requester.join().unwrap();
        assert!(matches!(
            follower.wait_timeout(Duration::from_secs(2)),
            SharedWorkSnapshot::Ready(_)
        ));
        assert_eq!(producers.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn two_worktrees_join_one_identical_immutable_delivery() {
        let desk = Arc::new(desk());
        let key = exact_delivery_key('a');
        let producers = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));

        let owner = {
            let desk = Arc::clone(&desk);
            let key = key.clone();
            let producers = Arc::clone(&producers);
            let release = Arc::clone(&release);
            let started = Arc::clone(&started);
            thread::spawn(move || {
                desk.request(
                    key.clone(),
                    move |producer| {
                        producers.fetch_add(1, Ordering::SeqCst);
                        producer.report(1024, Some(4096));
                        started.store(true, Ordering::SeqCst);
                        while !release.load(Ordering::SeqCst) {
                            thread::yield_now();
                        }
                        ArtifactReady::new(key, absolute_install_root("from-worktree-a"))
                    },
                    WIDE,
                    &CancellationToken::new(),
                    &NoopProgressSink,
                )
            })
        };
        while !started.load(Ordering::SeqCst) {
            thread::yield_now();
        }

        let follower = desk.request(
            key,
            |_| panic!("worktree B must join worktree A's immutable delivery"),
            WIDE,
            &CancellationToken::new(),
            &NoopProgressSink,
        );
        assert!(matches!(follower, EngineDeliveryState::Working { .. }));
        release.store(true, Ordering::SeqCst);
        assert!(matches!(
            owner.join().unwrap(),
            EngineDeliveryState::Ready(_)
        ));
        assert_eq!(producers.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn different_delivery_sha256_values_never_share() {
        let desk = desk();
        let producers = Arc::new(AtomicUsize::new(0));

        let first = desk.request(
            exact_delivery_key('b'),
            {
                let producers = Arc::clone(&producers);
                move |_| {
                    producers.fetch_add(1, Ordering::SeqCst);
                    ArtifactReady::new(exact_delivery_key('b'), absolute_install_root("b"))
                }
            },
            WIDE,
            &CancellationToken::new(),
            &NoopProgressSink,
        );
        let second = desk.request(
            exact_delivery_key('c'),
            {
                let producers = Arc::clone(&producers);
                move |_| {
                    producers.fetch_add(1, Ordering::SeqCst);
                    ArtifactReady::new(exact_delivery_key('c'), absolute_install_root("c"))
                }
            },
            WIDE,
            &CancellationToken::new(),
            &NoopProgressSink,
        );

        assert!(matches!(first, EngineDeliveryState::Ready(_)));
        assert!(matches!(second, EngineDeliveryState::Ready(_)));
        assert_eq!(producers.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn interrupted_archive_is_a_classified_failure_and_never_artifact_ready() {
        let desk = desk();
        let outcome = desk.request(
            exact_delivery_key('d'),
            |_| {
                Err(DeliveryFailure::new(
                    DeliveryFailureClass::Network,
                    "connection reset",
                ))
            },
            WIDE,
            &CancellationToken::new(),
            &NoopProgressSink,
        );

        let EngineDeliveryState::Failed { failure, .. } = outcome else {
            panic!("an interrupted archive must not become ArtifactReady");
        };
        assert_eq!(failure.class(), DeliveryFailureClass::Network);
    }

    #[test]
    fn delivery_boundary_rejects_non_delivery_key_mismatched_identity_and_relative_root() {
        let desk = desk();
        let non_delivery = SharedWorkKey::from(
            &ProviderHostKey::new(
                "bsl-analyzer",
                "test-target",
                std::collections::BTreeSet::from(["search".to_string()]),
            )
            .unwrap(),
        );
        let wrong = exact_delivery_key('e');
        let expected = exact_delivery_key('f');

        let non_delivery_failure = DeliveryWorkKey::try_from(non_delivery)
            .expect_err("a runtime key must not enter delivery");
        let mismatched = desk.request(
            expected.clone(),
            move |_| ArtifactReady::new(wrong, absolute_install_root("wrong-identity")),
            WIDE,
            &CancellationToken::new(),
            &NoopProgressSink,
        );
        let relative_failure =
            ArtifactReady::new(exact_delivery_key('7'), PathBuf::from("relative"))
                .expect_err("an ArtifactReady root must be absolute");

        assert_eq!(non_delivery_failure.class(), DeliveryFailureClass::Internal);
        assert_eq!(relative_failure.class(), DeliveryFailureClass::Internal);
        assert!(matches!(
            mismatched,
            EngineDeliveryState::Failed { ref failure, .. }
                if failure.class() == DeliveryFailureClass::Internal
        ));
    }
}

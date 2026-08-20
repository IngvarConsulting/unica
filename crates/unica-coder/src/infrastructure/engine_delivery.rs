//! Доставка движка по требованию.
//!
//! Вызов, которому нужен ещё не доставленный движок, не отказывает: он ждёт
//! столько, сколько разумно, и, не дождавшись, отвечает состоянием. Доставка
//! принадлежит серверу, а не вызову, который её застал, — она возобновляема,
//! переживает отмену и достаётся следующему.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use unica_bootstrap::{
    DownloadObserver, HostTarget, HttpDownloader, RuntimeInstaller, RuntimeManifest,
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

/// Чем кончилось ожидание доставки.
pub(crate) enum Delivered {
    /// Движок на месте, вызов продолжается. Путь установки здесь не нужен:
    /// инструмент найдёт движок обычным разрешением, в кеше артефактов.
    Ready,
    /// Не успела за окно. Доставка идёт дальше, вызывающий получает состояние.
    Working {
        received: u64,
        total: Option<u64>,
    },
    Failed(String),
}

/// Идущая доставка одного артефакта.
pub(crate) struct Delivery {
    started: Instant,
    received: AtomicU64,
    /// Ноль означает «сервер не сказал», а не «нисколько».
    total: AtomicU64,
    outcome: Mutex<Option<Result<PathBuf, String>>>,
    settled: Condvar,
}

impl Delivery {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            received: AtomicU64::new(0),
            total: AtomicU64::new(0),
            outcome: Mutex::new(None),
            settled: Condvar::new(),
        }
    }

    /// Сколько байтов уже на диске и сколько обещал сервер.
    pub(crate) fn moved(&self, received: u64, total: Option<u64>) {
        self.received.store(received, Ordering::Relaxed);
        self.total.store(total.unwrap_or(0), Ordering::Relaxed);
    }

    fn seen(&self) -> (u64, Option<u64>) {
        let total = self.total.load(Ordering::Relaxed);
        (
            self.received.load(Ordering::Relaxed),
            (total > 0).then_some(total),
        )
    }

    /// Через сколько повторять. Считается из измеренного темпа; пока считать
    /// не из чего — `None`, потому что пустое место честнее выдуманного числа.
    ///
    /// Пол в секунду выведен из шага загрузчика: он обновляет счётчик раз в
    /// 64 КБ, и опрос чаще не узнает ничего нового ни на одном канале.
    fn poll_hint(&self) -> Option<u64> {
        let (received, total) = self.seen();
        let total = total?;
        if received == 0 || received >= total {
            return None;
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        if elapsed <= 0.0 {
            return None;
        }
        let remaining = (total - received) as f64 / (received as f64 / elapsed);
        Some((remaining * 1000.0).max(1000.0) as u64)
    }

    fn settle(&self, outcome: Result<PathBuf, String>) {
        *self.outcome.lock().expect("delivery outcome") = Some(outcome);
        self.settled.notify_all();
    }
}

/// Кто ведёт учёт идущих доставок.
#[derive(Default)]
pub(crate) struct DeliveryDesk {
    in_flight: Mutex<HashMap<String, Arc<Delivery>>>,
}

impl DeliveryDesk {
    /// Дождаться движка, начав доставку, если её ещё никто не начал.
    ///
    /// Отмена вызова прекращает ожидание, но не доставку: она доедет и достанется
    /// следующему вызову.
    pub(crate) fn deliver<W>(
        &self,
        artifact: &str,
        work: W,
        window: Duration,
        cancellation: &CancellationToken,
        progress: &dyn ProgressSink,
    ) -> Delivered
    where
        W: FnOnce(Arc<Delivery>) -> Result<PathBuf, String> + Send + 'static,
    {
        let (delivery, started_here) = self.start(artifact, work);
        // Only the caller that created the server-owned delivery spends the
        // synchronous window on it. Followers report the shared work state
        // immediately, so one cold artifact can occupy at most one MCP
        // admission slot instead of all 32.
        let wait_window = if started_here { window } else { Duration::ZERO };
        match self.wait(artifact, &delivery, wait_window, cancellation, progress) {
            Some(outcome) => {
                // Расчёт окончен: следующий вызов либо возьмёт готовое из кеша,
                // либо начнёт заново, если доставка не удалась.
                self.in_flight.lock().expect("in flight").remove(artifact);
                match outcome {
                    Ok(_) => Delivered::Ready,
                    Err(error) => Delivered::Failed(error),
                }
            }
            // Учёт не снимается: доставка идёт, и следующий вызов присоединится
            // к ней, а не начнёт вторую.
            None => {
                let (received, total) = delivery.seen();
                Delivered::Working { received, total }
            }
        }
    }

    /// Подсказка о повторе для идущей доставки.
    pub(crate) fn poll_hint(&self, artifact: &str) -> Option<u64> {
        self.in_flight
            .lock()
            .expect("in flight")
            .get(artifact)
            .and_then(|delivery| delivery.poll_hint())
    }

    fn start<W>(&self, artifact: &str, work: W) -> (Arc<Delivery>, bool)
    where
        W: FnOnce(Arc<Delivery>) -> Result<PathBuf, String> + Send + 'static,
    {
        let mut in_flight = self.in_flight.lock().expect("in flight");
        if let Some(delivery) = in_flight.get(artifact) {
            return (Arc::clone(delivery), false);
        }
        let delivery = Arc::new(Delivery::new());
        in_flight.insert(artifact.to_owned(), Arc::clone(&delivery));
        let running = Arc::clone(&delivery);
        // Поток отцеплен намеренно: доставка переживает вызов, который её начал.
        thread::spawn(move || {
            let outcome = work(Arc::clone(&running));
            running.settle(outcome);
        });
        (delivery, true)
    }

    /// `None` — вызов ушёл, не дождавшись: окно кончилось или его отменили.
    fn wait(
        &self,
        artifact: &str,
        delivery: &Arc<Delivery>,
        window: Duration,
        cancellation: &CancellationToken,
        progress: &dyn ProgressSink,
    ) -> Option<Result<PathBuf, String>> {
        let deadline = Instant::now() + window;
        let mut outcome = delivery.outcome.lock().expect("delivery outcome");
        loop {
            // Копия, а не изъятие: ждать могут несколько вызовов, и забравший
            // исход первым оставил бы остальных ждать вечно.
            if let Some(settled) = outcome.as_ref() {
                return Some(settled.clone());
            }
            if cancellation.is_cancelled() {
                return None;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            publish(artifact, delivery, progress);
            let (next, _) = delivery
                .settled
                .wait_timeout(outcome, left.min(DELIVERY_PROGRESS_STEP))
                .expect("delivery outcome");
            outcome = next;
        }
    }
}

fn publish(artifact: &str, delivery: &Arc<Delivery>, progress: &dyn ProgressSink) {
    let (received, total) = delivery.seen();
    progress.publish(ProgressEvent {
        meta_key: DELIVERY_PROGRESS_META_KEY,
        payload: json!({
            "artifact": artifact,
            "receivedBytes": received,
            "totalBytes": total,
        }),
        progress: received as f64,
        total: total.unwrap_or(0) as f64,
        message: match total {
            Some(total) => format!("delivering {artifact}: {received} of {total} bytes"),
            None => format!("delivering {artifact}: {received} bytes"),
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

    /// Доставить артефакт тем же установщиком, что ставит ядро.
    pub(crate) fn acquire(self, delivery: Arc<Delivery>) -> Result<PathBuf, String> {
        let manifest =
            RuntimeManifest::load(&self.manifest_path).map_err(|error| error.to_string())?;
        let host = HostTarget::current().map_err(|error| error.to_string())?;
        RuntimeInstaller::new(
            self.cache_root,
            env!("CARGO_PKG_VERSION"),
            Arc::new(HttpDownloader::default()),
        )
        .ensure_artifact(&manifest, &self.artifact, host, &Watching(delivery))
        .map_err(|error| error.to_string())
    }
}

/// Наблюдатель загрузки, перекладывающий байты в идущую доставку.
struct Watching(Arc<Delivery>);

impl DownloadObserver for Watching {
    fn transferred(&self, received: u64, total: Option<u64>) {
        self.0.moved(received, total);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn expect_ready(delivered: Delivered) {
        match delivered {
            Delivered::Ready => {}
            Delivered::Working { received, total } => {
                panic!("ожидали движок, получили работу: {received}/{total:?}")
            }
            Delivered::Failed(error) => panic!("ожидали движок, получили отказ: {error}"),
        }
    }

    fn expect_eventually_ready(desk: &DeliveryDesk, artifact: &'static str) {
        let deadline = Instant::now() + WIDE;
        loop {
            match desk.deliver(
                artifact,
                |_| panic!("the existing delivery must not be started twice"),
                WIDE,
                &CancellationToken::new(),
                &NoopProgressSink,
            ) {
                Delivered::Ready => return,
                Delivered::Working { .. } if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Delivered::Working { .. } => panic!("delivery did not settle before the deadline"),
                Delivered::Failed(error) => panic!("delivery failed: {error}"),
            }
        }
    }

    #[test]
    fn a_delivery_inside_the_window_hands_over_the_engine() {
        let desk = desk();

        let delivered = desk.deliver(
            "rlm-tools-bsl",
            |delivery| {
                delivery.moved(7, Some(7));
                Ok(PathBuf::from("/cache/rlm-tools-bsl/1.33.0/darwin-arm64"))
            },
            WIDE,
            &CancellationToken::new(),
            &Heard::default(),
        );

        expect_ready(delivered);
    }

    #[test]
    fn a_delivery_longer_than_the_window_answers_working_and_keeps_going() {
        // Хост режет вызов своим сроком, и его ответ уносит наш целиком.
        // Поэтому отвечаем сами — а доставка продолжается.
        let desk = Arc::new(desk());
        let release = Arc::new(AtomicBool::new(false));
        let held = Arc::clone(&release);

        let delivered = desk.deliver(
            "rlm-tools-bsl",
            move |delivery| {
                delivery.moved(1024, Some(4096));
                while !held.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(5));
                }
                Ok(PathBuf::from("/cache/rlm-tools-bsl"))
            },
            Duration::from_millis(120),
            &CancellationToken::new(),
            &NoopProgressSink,
        );

        assert!(
            matches!(
                delivered,
                Delivered::Working {
                    received: 1024,
                    total: Some(4096)
                }
            ),
            "вызов отвечает состоянием, а не ждёт до конца"
        );
        assert!(
            desk.poll_hint("rlm-tools-bsl").is_some(),
            "подсказка о повторе считается из измеренного темпа"
        );

        release.store(true, Ordering::SeqCst);
        expect_eventually_ready(&desk, "rlm-tools-bsl");
    }

    #[test]
    fn followers_of_an_in_flight_delivery_return_working_without_waiting() {
        // Every waiting tools/call owns one MCP admission slot. If followers
        // wait for the same 30-second delivery window, 32 identical cold calls
        // can occupy the whole dispatcher even though only one download exists.
        let desk = Arc::new(desk());
        let release = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));

        let first_desk = Arc::clone(&desk);
        let first_release = Arc::clone(&release);
        let first_started = Arc::clone(&started);
        let first = thread::spawn(move || {
            first_desk.deliver(
                "rlm-tools-bsl",
                move |delivery| {
                    delivery.moved(1024, Some(4096));
                    first_started.store(true, Ordering::SeqCst);
                    while !first_release.load(Ordering::SeqCst) {
                        thread::yield_now();
                    }
                    Ok(PathBuf::from("/cache/rlm-tools-bsl"))
                },
                WIDE,
                &CancellationToken::new(),
                &NoopProgressSink,
            )
        });
        while !started.load(Ordering::SeqCst) {
            thread::yield_now();
        }

        let follower_desk = Arc::clone(&desk);
        let (sent, received) = std::sync::mpsc::channel();
        let follower = thread::spawn(move || {
            let outcome = follower_desk.deliver(
                "rlm-tools-bsl",
                |_| panic!("the follower must not start a second delivery"),
                WIDE,
                &CancellationToken::new(),
                &NoopProgressSink,
            );
            sent.send(outcome).unwrap();
        });
        let prompt = received.recv_timeout(Duration::from_millis(200)).ok();

        release.store(true, Ordering::SeqCst);
        let first_outcome = first.join().expect("first caller");
        follower.join().expect("follower caller");
        expect_ready(first_outcome);

        assert!(
            matches!(
                prompt,
                Some(Delivered::Working {
                    received: 1024,
                    total: Some(4096)
                })
            ),
            "a follower held an admission slot instead of returning current work state"
        );
    }

    #[test]
    fn two_calls_share_one_delivery_but_only_the_owner_waits() {
        // Один архив, две сессии. Качать его дважды значит платить дважды за то
        // же самое и получить два писателя на один файл.
        let desk = Arc::new(desk());
        let started = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let owner_desk = Arc::clone(&desk);
        let owner_started = Arc::clone(&started);
        let owner_release = Arc::clone(&release);
        let owner = thread::spawn(move || {
            owner_desk.deliver(
                "rlm-tools-bsl",
                move |_| {
                    owner_started.fetch_add(1, Ordering::SeqCst);
                    while !owner_release.load(Ordering::SeqCst) {
                        thread::yield_now();
                    }
                    Ok(PathBuf::from("/cache/rlm-tools-bsl"))
                },
                WIDE,
                &CancellationToken::new(),
                &NoopProgressSink,
            )
        });
        while started.load(Ordering::SeqCst) == 0 {
            thread::yield_now();
        }
        let follower = desk.deliver(
            "rlm-tools-bsl",
            |_| panic!("the shared delivery must not start twice"),
            WIDE,
            &CancellationToken::new(),
            &NoopProgressSink,
        );
        assert!(matches!(follower, Delivered::Working { .. }));
        release.store(true, Ordering::SeqCst);
        expect_ready(owner.join().expect("owner"));
        assert_eq!(started.load(Ordering::SeqCst), 1, "качали один раз");
    }

    #[test]
    fn a_cancelled_call_does_not_cancel_the_delivery() {
        // Загрузка возобновляема и полезна следующему вызову, поэтому
        // принадлежит серверу, а не вызову, который её застал.
        let desk = Arc::new(desk());
        let started = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let cancellation = CancellationToken::new();
        let leaving = {
            let desk = Arc::clone(&desk);
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            let cancellation = cancellation.clone();
            thread::spawn(move || {
                desk.deliver(
                    "rlm-tools-bsl",
                    move |_| {
                        started.fetch_add(1, Ordering::SeqCst);
                        while !release.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Ok(PathBuf::from("/cache/rlm-tools-bsl"))
                    },
                    Duration::from_secs(30),
                    &cancellation,
                    &NoopProgressSink,
                )
            })
        };
        while started.load(Ordering::SeqCst) == 0 {
            thread::sleep(Duration::from_millis(5));
        }

        cancellation.cancel();

        assert!(
            matches!(leaving.join().expect("waiter"), Delivered::Working { .. }),
            "ушедший вызов перестаёт ждать, но доставку не отменяет"
        );
        release.store(true, Ordering::SeqCst);

        expect_eventually_ready(&desk, "rlm-tools-bsl");
        assert_eq!(started.load(Ordering::SeqCst), 1, "качали один раз");
    }

    #[test]
    fn waiting_shows_that_a_delivery_is_running() {
        // Сколько байтов успеет приехать к первому сообщению — дело гонки, а
        // вот что сообщение уходит и несёт ключ доставки — нет.
        let desk = desk();
        let heard = Heard::default();

        desk.deliver(
            "rlm-tools-bsl",
            |delivery| {
                delivery.moved(1024, Some(4096));
                thread::sleep(Duration::from_millis(60));
                Ok(PathBuf::from("/cache/rlm-tools-bsl"))
            },
            WIDE,
            &CancellationToken::new(),
            &heard,
        );

        let events = heard.events.lock().expect("events").clone();
        assert!(!events.is_empty(), "вызывающий видит, что доставка идёт");
        assert!(
            events
                .iter()
                .all(|event| event.meta_key == DELIVERY_PROGRESS_META_KEY),
            "у доставки свой ключ: {events:?}"
        );
    }

    #[test]
    fn a_progress_event_names_the_bytes_on_disk() {
        let delivery = Arc::new(Delivery::new());
        delivery.moved(1024, Some(4096));
        let heard = Heard::default();

        publish("rlm-tools-bsl", &delivery, &heard);

        let event = heard.events.lock().expect("events")[0].clone();
        assert_eq!(event.progress, 1024.0);
        assert_eq!(event.total, 4096.0);
        assert!(
            event.message.contains("1024") && event.message.contains("4096"),
            "в сообщении названы байты: {}",
            event.message
        );
        assert_eq!(
            event.payload.get("artifact").and_then(|name| name.as_str()),
            Some("rlm-tools-bsl")
        );
    }

    #[test]
    fn a_delivery_whose_size_is_unknown_still_reports_what_arrived() {
        // Сервер не обязан обещать длину. Ноль в total означает «не сказал», а
        // не «нисколько», и вызывающему всё равно видно движение.
        let delivery = Arc::new(Delivery::new());
        delivery.moved(1024, None);
        let heard = Heard::default();

        publish("rlm-tools-bsl", &delivery, &heard);

        let event = heard.events.lock().expect("events")[0].clone();
        assert_eq!(event.progress, 1024.0);
        assert_eq!(event.total, 0.0);
        assert!(event.message.contains("1024"), "{}", event.message);
    }

    #[test]
    fn a_delivery_that_failed_is_tried_again_by_the_next_call() {
        // Отказ доставки — не приговор артефакту: сеть вернётся, и следующий
        // вызов должен получить движок, а не запомненную неудачу.
        let desk = desk();
        let attempts = Arc::new(AtomicUsize::new(0));

        let first = desk.deliver(
            "rlm-tools-bsl",
            {
                let attempts = Arc::clone(&attempts);
                move |_| {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err("канал оборвался".to_string())
                }
            },
            WIDE,
            &CancellationToken::new(),
            &NoopProgressSink,
        );
        assert!(matches!(first, Delivered::Failed(_)));

        let second = desk.deliver(
            "rlm-tools-bsl",
            {
                let attempts = Arc::clone(&attempts);
                move |_| {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Ok(PathBuf::from("/cache/rlm-tools-bsl"))
                }
            },
            WIDE,
            &CancellationToken::new(),
            &NoopProgressSink,
        );

        assert!(matches!(second, Delivered::Ready));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }
}

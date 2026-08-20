//! Доставка движка по требованию.
//!
//! Вызов, которому нужен ещё не доставленный движок, не отказывает: он ждёт с
//! прогрессом и исполняется. Доставка при этом принадлежит серверу, а не вызову,
//! который её застал, — она возобновляема и полезна следующему.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

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

/// Идущая доставка одного артефакта.
pub(crate) struct Delivery {
    received: AtomicU64,
    /// Ноль означает «сервер не сказал», а не «нисколько».
    total: AtomicU64,
    outcome: Mutex<Option<Result<PathBuf, String>>>,
    settled: Condvar,
}

impl Delivery {
    fn new() -> Self {
        Self {
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
    pub(crate) fn wait_for<W>(
        &self,
        artifact: &str,
        work: W,
        cancellation: &CancellationToken,
        progress: &dyn ProgressSink,
    ) -> Result<PathBuf, String>
    where
        W: FnOnce(Arc<Delivery>) -> Result<PathBuf, String> + Send + 'static,
    {
        let delivery = self.start(artifact, work);
        let outcome = self.wait(artifact, &delivery, cancellation, progress);
        if outcome.is_some() {
            // Расчёт окончен: следующий вызов либо возьмёт готовое из кеша, либо
            // начнёт заново, если доставка не удалась.
            self.in_flight.lock().expect("in flight").remove(artifact);
        }
        outcome.unwrap_or_else(|| Err(format!("delivery of {artifact} was cancelled by the call")))
    }

    fn start<W>(&self, artifact: &str, work: W) -> Arc<Delivery>
    where
        W: FnOnce(Arc<Delivery>) -> Result<PathBuf, String> + Send + 'static,
    {
        let mut in_flight = self.in_flight.lock().expect("in flight");
        if let Some(delivery) = in_flight.get(artifact) {
            return Arc::clone(delivery);
        }
        let delivery = Arc::new(Delivery::new());
        in_flight.insert(artifact.to_owned(), Arc::clone(&delivery));
        let running = Arc::clone(&delivery);
        // Поток отцеплен намеренно: доставка переживает вызов, который её начал.
        thread::spawn(move || {
            let outcome = work(Arc::clone(&running));
            running.settle(outcome);
        });
        delivery
    }

    /// `None` — вызов ушёл, не дождавшись.
    fn wait(
        &self,
        artifact: &str,
        delivery: &Arc<Delivery>,
        cancellation: &CancellationToken,
        progress: &dyn ProgressSink,
    ) -> Option<Result<PathBuf, String>> {
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
            publish(artifact, delivery, progress);
            let (next, _) = delivery
                .settled
                .wait_timeout(outcome, DELIVERY_PROGRESS_STEP)
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
    manifest_path: PathBuf,
    cache_root: PathBuf,
}

/// Куда bootstrap ставит артефакты. Без неё рантайм запущен не загрузчиком, и
/// доставлять некуда.
const ARTIFACT_CACHE_ENV: &str = "UNICA_ARTIFACT_CACHE";

pub(crate) fn order_for(plugin_root: &Path, tool_name: &str) -> Option<EngineOrder> {
    let cache_root = std::env::var_os(ARTIFACT_CACHE_ENV).map(PathBuf::from)?;
    let artifact = crate::infrastructure::bundled_tools::artifact_for(plugin_root, tool_name)?;
    Some(EngineOrder {
        artifact,
        manifest_path: plugin_root.join("runtime-manifest.json"),
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

    #[test]
    fn a_call_waits_for_the_engine_and_gets_it() {
        let desk = desk();
        let heard = Heard::default();

        let delivered = desk
            .wait_for(
                "rlm-tools-bsl",
                |delivery| {
                    delivery.moved(7, Some(7));
                    Ok(PathBuf::from("/cache/rlm-tools-bsl/1.33.0/darwin-arm64"))
                },
                &CancellationToken::new(),
                &heard,
            )
            .expect("движок доставлен");

        assert_eq!(
            delivered,
            PathBuf::from("/cache/rlm-tools-bsl/1.33.0/darwin-arm64")
        );
    }

    #[test]
    fn two_calls_wait_for_one_delivery() {
        // Один архив, две сессии. Качать его дважды значит платить дважды за то
        // же самое и получить два писателя на один файл.
        let desk = Arc::new(desk());
        let started = Arc::new(AtomicUsize::new(0));
        let waiters = (0..2)
            .map(|_| {
                let desk = Arc::clone(&desk);
                let started = Arc::clone(&started);
                thread::spawn(move || {
                    desk.wait_for(
                        "rlm-tools-bsl",
                        move |_| {
                            started.fetch_add(1, Ordering::SeqCst);
                            thread::sleep(Duration::from_millis(120));
                            Ok(PathBuf::from("/cache/rlm-tools-bsl"))
                        },
                        &CancellationToken::new(),
                        &NoopProgressSink,
                    )
                })
            })
            .collect::<Vec<_>>();

        let delivered = waiters
            .into_iter()
            .map(|waiter| waiter.join().expect("waiter"))
            .collect::<Result<Vec<_>, _>>()
            .expect("оба вызова получают движок");

        assert_eq!(delivered[0], delivered[1]);
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
                desk.wait_for(
                    "rlm-tools-bsl",
                    move |_| {
                        started.fetch_add(1, Ordering::SeqCst);
                        while !release.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Ok(PathBuf::from("/cache/rlm-tools-bsl"))
                    },
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
            leaving.join().expect("waiter").is_err(),
            "ушедший вызов получает отказ по отмене"
        );
        release.store(true, Ordering::SeqCst);

        let delivered = desk
            .wait_for(
                "rlm-tools-bsl",
                |_| panic!("доставка уже идёт, второй раз её не начинают"),
                &CancellationToken::new(),
                &NoopProgressSink,
            )
            .expect("следующий вызов забирает то, что доехало");
        assert_eq!(delivered, PathBuf::from("/cache/rlm-tools-bsl"));
        assert_eq!(started.load(Ordering::SeqCst), 1, "качали один раз");
    }

    #[test]
    fn waiting_shows_that_a_delivery_is_running() {
        // Сколько байтов успеет приехать к первому сообщению — дело гонки, а
        // вот что сообщение уходит и несёт ключ доставки — нет.
        let desk = desk();
        let heard = Heard::default();

        desk.wait_for(
            "rlm-tools-bsl",
            |delivery| {
                delivery.moved(1024, Some(4096));
                thread::sleep(Duration::from_millis(60));
                Ok(PathBuf::from("/cache/rlm-tools-bsl"))
            },
            &CancellationToken::new(),
            &heard,
        )
        .expect("доставлено");

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

        let first = desk.wait_for(
            "rlm-tools-bsl",
            {
                let attempts = Arc::clone(&attempts);
                move |_| {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err("канал оборвался".to_string())
                }
            },
            &CancellationToken::new(),
            &NoopProgressSink,
        );
        assert!(first.is_err());

        let second = desk.wait_for(
            "rlm-tools-bsl",
            {
                let attempts = Arc::clone(&attempts);
                move |_| {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Ok(PathBuf::from("/cache/rlm-tools-bsl"))
                }
            },
            &CancellationToken::new(),
            &NoopProgressSink,
        );

        assert!(second.is_ok());
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }
}

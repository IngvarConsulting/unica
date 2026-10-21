//! Доставка движка по требованию.
//!
//! Вызов, которому нужен ещё не доставленный движок, не отказывает: он ждёт
//! столько, сколько разумно, и, не дождавшись, отвечает состоянием. Доставка
//! принадлежит серверу, а не вызову, который её застал, — она возобновляема,
//! переживает отмену и достаётся следующему.

use crate::domain::engine::MissingEngineRefusal;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use unica_artifact_delivery::desk::DELIVERY_PROGRESS_META_KEY;
use unica_artifact_delivery::desk::{CanonicalDeliveryProgress, DeliveryDesk, EngineOrder};

use serde_json::json;

use unica_artifact_delivery::shared::{DeliveryFailureClass, EngineDeliveryState};

use unica_execution_contract::invocation::DomainResult;
use unica_refusal::{RefusalCode, RefusalDetail};
use unica_work_control::cancellation::CancellationToken;
#[cfg(test)]
use unica_work_control::progress::ProgressEvent;
use unica_work_control::progress::ProgressSink;

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

/// The one acquisition path for legacy adapters and canonical operations.
/// Both inspect the same installed location and join an exact delivery in
/// their coordinator, so a request never starts a second download under
/// another key in one process.
pub(crate) fn deliver_if_missing(
    desk: &DeliveryDesk,
    plugin_root: &Path,
    tool_name: &str,
    cancellation: &CancellationToken,
    progress: &dyn ProgressSink,
) -> EngineDeliveryState {
    if crate::infrastructure::bundled_tools::installed_engine_path(plugin_root, tool_name).is_some()
    {
        return EngineDeliveryState::NotRequired;
    }
    let Some(order) = order_for(plugin_root, tool_name) else {
        return EngineDeliveryState::NotRequired;
    };
    order.deliver(
        desk,
        || unica_work_control::long_work::sync_window(unica_bootstrap::host_tool_deadline()),
        cancellation,
        progress,
    )
}

/// Canonical callers return a bounded, typed delivery state. A pending
/// delivery is a retryable refusal, never a successful result for an operation
/// that has not run. The bootstrap error text can contain a remote URL or
/// filesystem path, so only its closed class is projected onto the response.
pub(crate) fn canonical_delivery_result(
    state: EngineDeliveryState,
    progress: &CanonicalDeliveryProgress,
) -> Option<DomainResult> {
    match state {
        EngineDeliveryState::NotRequired | EngineDeliveryState::Ready(_) => None,
        EngineDeliveryState::Working {
            artifact,
            received,
            total,
            poll_interval_ms,
        } => {
            let (received, total) = progress.observed(&artifact).unwrap_or((received, total));
            let mut result = DomainResult::canonical_rejection_detailed(
                None,
                RefusalDetail::DeliveryInProgress,
                format!("delivery of {artifact} is continuing; repeat the same call"),
            );
            result.data = Some(json!({
                "delivery": {
                    "artifact": artifact,
                    "status": "working",
                    "receivedBytes": received,
                    "totalBytes": total,
                    "retryAfterMs": poll_interval_ms.unwrap_or(1000),
                }
            }));
            Some(result)
        }
        EngineDeliveryState::Failed { artifact, failure } => {
            let (class, refusal, advice) = match failure.class() {
                DeliveryFailureClass::Network => (
                    "network",
                    Ok(RefusalDetail::ProviderAbsent),
                    "check network access to the pinned release and repeat the call",
                ),
                DeliveryFailureClass::UntrustedCertificate => (
                    "untrusted-certificate",
                    Ok(RefusalDetail::ProviderAbsent),
                    "the operating system does not trust the certificate presented by the release host; most often HTTPS is intercepted by security software or a corporate gateway whose root is not in the OS trust store: add the host to the HTTPS-scanning exclusions, install the gateway root into the OS trust store, or set HTTPS_PROXY (NO_PROXY lists exceptions), then restart Unica",
                ),
                DeliveryFailureClass::Timeout => (
                    "timeout",
                    Err(RefusalCode::DeadlineExceeded),
                    "repeat the call after the delivery timeout",
                ),
                DeliveryFailureClass::Disk => (
                    "disk",
                    Ok(RefusalDetail::ProviderAbsent),
                    "check the runtime cache disk and repeat the call",
                ),
                DeliveryFailureClass::Checksum => (
                    "checksum",
                    Ok(RefusalDetail::ProviderAbsent),
                    "check the pinned artifact and its published checksum",
                ),
                DeliveryFailureClass::Configuration => (
                    "configuration",
                    Ok(RefusalDetail::ProviderAbsent),
                    "check the runtime manifest and plugin installation",
                ),
                DeliveryFailureClass::Internal => (
                    "internal",
                    Ok(RefusalDetail::BackendBroken),
                    "inspect the Unica runtime delivery failure",
                ),
            };
            let message = format!("delivery of {artifact} failed ({class}); {advice}");
            let mut result = match refusal {
                Ok(detail) => DomainResult::canonical_rejection_detailed(None, detail, message),
                Err(code) => DomainResult::canonical_rejection(None, code, message),
            };
            result.data = Some(json!({
                "delivery": {"artifact": artifact, "status": "failed", "failureClass": class}
            }));
            Some(result)
        }
    }
}

/// Состояние доставки движка `tool_name` для канонического маршрута.
///
/// Неудачная доставка, после которой движка нет, — тот же отказ «нет
/// поставляемого движка»: рядом с классом сбоя доставки `data` несёт поля
/// движка на шаге `delivery` (#549). Срок и внутренний сбой остаются своими
/// кодами и полей движка не получают.
pub(crate) fn canonical_engine_delivery_result(
    state: EngineDeliveryState,
    progress: &CanonicalDeliveryProgress,
    plugin_root: &Path,
    tool_name: &str,
) -> Option<DomainResult> {
    let mut result = canonical_delivery_result(state, progress)?;
    let absent = result
        .diagnostics
        .first()
        .and_then(|diagnostic| diagnostic.get("detailCode"))
        == Some(&serde_json::Value::from(
            RefusalDetail::ProviderAbsent.as_str(),
        ));
    if absent {
        let engine = crate::infrastructure::bundled_tools::delivery_failure(plugin_root, tool_name);
        if let (Some(serde_json::Value::Object(data)), serde_json::Value::Object(fields)) =
            (result.data.as_mut(), engine.refusal_data())
        {
            data.extend(fields);
        }
    }
    Some(result)
}

/// Разрешение с явно названным окружением: так его видно в тесте.
fn order_for_in(
    plugin_root: &Path,
    tool_name: &str,
    read_env: &dyn Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<EngineOrder> {
    let cache_root = read_env(ARTIFACT_CACHE_ENV).map(PathBuf::from)?;
    let artifact = crate::infrastructure::bundled_tools::artifact_for(plugin_root, tool_name)?;
    Some(EngineOrder::new(
        artifact,
        read_env(RUNTIME_MANIFEST_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| plugin_root.join("runtime-manifest.json")),
        cache_root,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_artifact_delivery::desk::classify_failure;

    use unica_artifact_delivery::shared::{
        ArtifactReady, DeliveryFailure, DeliveryFailureClass, DeliveryWorkKey,
    };

    use unica_analysis_services::provider_host::ProviderHostKey;
    use unica_artifact_delivery::desk::test_support::{absolute_install_root, exact_delivery_key};
    fn desk() -> DeliveryDesk {
        DeliveryDesk::default()
    }
    use unica_work_control::progress::NoopProgressSink;

    use std::time::Duration;

    #[test]
    fn canonical_delivery_state_reports_progress_without_claiming_search_completed() {
        let progress = CanonicalDeliveryProgress::default();
        progress.publish(ProgressEvent {
            meta_key: DELIVERY_PROGRESS_META_KEY,
            payload: json!({"artifact": "rlm-tools-bsl", "receivedBytes": 4096, "totalBytes": 8192}),
            progress: 4096.0,
            total: 8192.0,
            message: "receiving".to_string(),
        });
        let result = canonical_delivery_result(
            EngineDeliveryState::Working {
                artifact: "rlm-tools-bsl".to_string(),
                received: 0,
                total: None,
                poll_interval_ms: Some(1500),
            },
            &progress,
        )
        .expect("unfinished delivery has a public state");
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
        assert_eq!(result.diagnostics[0]["detailCode"], "delivery_in_progress");
        assert_eq!(result.diagnostics[0]["outcome"], "retry");
        assert_eq!(
            result.data.as_ref().unwrap()["delivery"]["status"],
            "working"
        );
        assert_eq!(
            result.data.as_ref().unwrap()["delivery"]["receivedBytes"],
            4096
        );
        assert_eq!(
            result.data.as_ref().unwrap()["delivery"]["totalBytes"],
            8192
        );
        assert_eq!(
            result.data.as_ref().unwrap()["delivery"]["retryAfterMs"],
            1500
        );
    }

    #[test]
    fn canonical_delivery_failure_exposes_a_closed_class_without_bootstrap_prose() {
        let result = canonical_delivery_result(
            EngineDeliveryState::Failed {
                artifact: "v8-runner".to_string(),
                failure: Arc::new(DeliveryFailure::new(
                    DeliveryFailureClass::Checksum,
                    "secret network URL and private cache path",
                )),
            },
            &CanonicalDeliveryProgress::default(),
        )
        .expect("failed delivery has a public refusal");
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
        assert_eq!(
            result.data.as_ref().unwrap()["delivery"]["failureClass"],
            "checksum"
        );
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("secret network URL"));
        assert!(!encoded.contains("private cache path"));
    }

    /// #549: неудачная доставка, после которой движка нет, — тот же отказ «нет
    /// движка» с полями на шаге `delivery`; срок доставки остаётся своим кодом.
    #[test]
    fn a_failed_engine_delivery_carries_the_missing_engine_fields() {
        let plugin_root = plugin_root_with_tool("delivery-fields");
        let failed = |class| EngineDeliveryState::Failed {
            artifact: "rlm-tools-bsl".to_string(),
            failure: Arc::new(DeliveryFailure::new(class, "private cache path")),
        };

        let result = canonical_engine_delivery_result(
            failed(DeliveryFailureClass::Network),
            &CanonicalDeliveryProgress::default(),
            &plugin_root,
            "rlm-bsl-index",
        )
        .expect("failed delivery has a public refusal");
        assert_eq!(result.diagnostics[0]["detailCode"], "provider_absent");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["delivery"]["failureClass"], "network");
        assert_eq!(data["tool"], "rlm-bsl-index");
        assert_eq!(data["stage"], "delivery");
        assert_eq!(data["pinnedVersion"], "1.33.0");
        assert_eq!(data["retryable"], false);
        assert!(
            data["target"].is_string() && data["path"].is_string(),
            "{data}"
        );

        let timeout = canonical_engine_delivery_result(
            failed(DeliveryFailureClass::Timeout),
            &CanonicalDeliveryProgress::default(),
            &plugin_root,
            "rlm-bsl-index",
        )
        .unwrap();
        assert_eq!(timeout.diagnostics[0]["code"], "deadline_exceeded");
        assert!(timeout.data.as_ref().unwrap().get("tool").is_none());
    }

    /// Первая доставка движка за антивирусом: ОС не доверяет подменённому
    /// корню. Ответ называет вероятную причину и что исправить, а не
    /// «проверьте сеть и повторите» — повтор здесь не поможет.
    #[test]
    fn an_untrusted_certificate_reaches_the_caller_as_its_own_class_with_the_cure() {
        use unica_artifact_delivery::{Downloader, HttpDownloader, SilentDownload};
        use unica_network::test_support::{client_with, TestRoot, TlsStand};

        let root = TestRoot::generate("Unica delivery untrusted root");
        let stand = TlsStand::start(&root, "archive", "application/octet-stream");
        let destination = tempfile::tempdir().expect("scratch");
        let error = HttpDownloader::with_client(client_with(&[]))
            .download(
                &stand.url("v8-runner.tar.gz"),
                &destination.path().join("v8-runner.tar.gz"),
                &SilentDownload,
            )
            .expect_err("корень не доверен");

        let failure = classify_failure(error);
        assert_eq!(failure.class(), DeliveryFailureClass::UntrustedCertificate);
        assert!(
            failure.legacy_diagnostic().contains("intercepted"),
            "{}",
            failure.legacy_diagnostic()
        );
        let result = canonical_delivery_result(
            EngineDeliveryState::Failed {
                artifact: "v8-runner".to_string(),
                failure: Arc::new(failure),
            },
            &CanonicalDeliveryProgress::default(),
        )
        .expect("failed delivery has a public refusal");
        assert_eq!(
            result.data.as_ref().unwrap()["delivery"]["failureClass"],
            "untrusted-certificate"
        );
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(encoded.contains("intercepted"), "{encoded}");
        assert!(encoded.contains("trust store"), "{encoded}");
        assert!(
            !encoded.contains("127.0.0.1"),
            "адрес не попадает в ответ: {encoded}"
        );
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

    #[test]
    fn delivery_boundary_rejects_non_delivery_key_mismatched_identity_and_relative_root() {
        let desk = desk();
        assert_ne!(
            std::any::TypeId::of::<DeliveryWorkKey>(),
            std::any::TypeId::of::<ProviderHostKey>(),
            "provider and delivery identities must remain distinct types",
        );
        let wrong = exact_delivery_key('e');
        let expected = exact_delivery_key('f');

        let mismatched = desk.request(
            expected.clone(),
            move |_| ArtifactReady::new(wrong, absolute_install_root("wrong-identity")),
            Duration::from_secs(5),
            &CancellationToken::new(),
            &NoopProgressSink,
        );
        let relative_failure =
            ArtifactReady::new(exact_delivery_key('7'), PathBuf::from("relative"))
                .expect_err("an ArtifactReady root must be absolute");

        assert_eq!(relative_failure.class(), DeliveryFailureClass::Internal);
        assert!(matches!(
            mismatched,
            EngineDeliveryState::Failed { ref failure, .. }
                if failure.class() == DeliveryFailureClass::Internal
        ));
    }
}

use super::*;
use std::sync::Arc;
use std::time::Duration;
use unica_documentation::model::{DocumentationRegistry, DocumentationSectionStatus};
use unica_network::test_support::{client_with, HttpStand};
#[test]
fn real_v8std_status_failures_and_public_projection_hide_endpoint_userinfo() {
    for status in [401, 404] {
        let stand = HttpStand::respond(status, &[], Vec::new(), "text/plain");
        let endpoint = stand
            .url("mcp")
            .replacen("http://", "http://DocsUser2566:DocsPass2566@", 1);
        let provider = V8StdDocumentationProvider {
            endpoint: resolve_standards_endpoint_with(
                &DocumentationPolicy::default(),
                Some(endpoint),
            ),
            network: NetworkAccess::Allow,
            http: Arc::new(crate::standards_transport::NetworkHttpClient::with_client(
                client_with(&[]),
            )),
            cancellation: Default::default(),
            search_cache_ttl: Duration::ZERO,
            revalidate_search: true,
        };
        let request = DocumentationSearchRequest {
            query: format!("synthetic {status}"),
            source_kinds: Vec::new(),
            limit: 1,
            language: "ru".into(),
        };
        let context = DocumentationContext {
            installation_root: None,
            platform_version: None,
        };
        let sections = provider.search(&request, &context);
        let DocumentationSectionStatus::Failed { diagnostic } = &sections[0].status else {
            panic!("expected a Failed source")
        };
        for marker in ["DocsUser2566", "DocsPass2566"] {
            assert!(!diagnostic.contains(marker), "{diagnostic}");
        }
        for safe in ["127.0.0.1", "/mcp", &format!("HTTP {status}")] {
            assert!(diagnostic.contains(safe), "{diagnostic}");
        }
        let registry = DocumentationRegistry::new(vec![Arc::new(provider)]).unwrap();
        let error =
            unica_documentation::projection::search(&registry, &request, &context).unwrap_err();
        for marker in ["DocsUser2566", "DocsPass2566"] {
            assert!(!error.contains(marker), "{error}");
        }
        for safe in [
            "v8std/public-standards",
            "127.0.0.1",
            "/mcp",
            &format!("HTTP {status}"),
        ] {
            assert!(error.contains(safe), "{error}");
        }
        assert_eq!(stand.requests(), vec!["POST /mcp HTTP/1.1"; 2]);
        assert_eq!(
            stand.header("Authorization"),
            vec![vec!["Basic RG9jc1VzZXIyNTY2OkRvY3NQYXNzMjU2Ng==".to_owned()]; 2]
        );
    }
}

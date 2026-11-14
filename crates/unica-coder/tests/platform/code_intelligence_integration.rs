use unica_adapter_bsl_analyzer::provider::{
    parse_call_graph_answer, BslAnalyzerProvider, BslToolClient,
};
use unica_adapter_lexical::provider::*;

use unica_code_intelligence::model::*;
use unica_code_intelligence::source_locations::*;

use unica_source_contract::location::SourceLocation;

use unica_work_control::cancellation::CancellationToken;

fn parse_bsl_analyzer_search(
    text: &str,
    context: &CodeIntelligenceContext,
    limit: usize,
    cancel: &CancellationToken,
) -> ProviderSearchSection {
    unica_adapter_bsl_analyzer::provider::parse_bsl_analyzer_search(
        text,
        context,
        limit,
        cancel,
        crate::infrastructure::analysis_composition::source_bindings(),
    )
}
#[cfg(test)]
fn location_path(location: &SourceLocation) -> &str {
    match location {
        SourceLocation::Unaddressable { path, .. } => path,
        SourceLocation::Addressed { .. } => "",
    }
}
#[cfg(test)]
mod medium_tests {
    use super::{
        git_grep_bsl_pathspecs, location_path, parse_call_graph_answer, BslAnalyzerProvider,
        BslToolClient, GitGrepProvider,
    };
    use serde_json::{json, Value};
    use unica_code_intelligence::model::CallEdgeProvenance;
    use unica_code_intelligence::model::CallGraphDirection;
    use unica_code_intelligence::model::CallGraphState;
    use unica_code_intelligence::model::CodeIntelligenceContext;
    use unica_code_intelligence::model::CodeIntelligenceProvider;
    use unica_code_intelligence::model::CodeIntelligenceReadData;
    use unica_code_intelligence::model::CodeIntelligenceReadRequest;
    use unica_code_intelligence::model::CodeSearchScope;
    use unica_code_intelligence::model::ProviderId;
    use unica_code_intelligence::model::ProviderSectionStatus;
    use unica_code_intelligence::model::RelativeSearchFilter;
    use unica_code_intelligence::model::SearchCountRelation;
    use unica_code_intelligence::model::SearchRequest;
    use unica_process::mcp_session::McpToolOutput;
    use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
    use unica_source_contract::location::SourceLocation;
    use unica_source_contract::workspace::ResolvedSourceRoot;
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::cancellation::CANCELLED_PREFIX;
    use unica_work_control::provider_deadline::ProviderDeadline;

    /// Замеренные ответы анализатора, а не придуманные.
    ///
    /// Фикстура снята с живого `bsl-analyzer 0.2.67`, доставленного из
    /// `tools.lock.json` с проверенной sha256. Каждое утверждение здесь — факт
    /// замера: счёт берётся из поля анализатора, незавершённый индекс назван
    /// состоянием, происхождение ребра сохранено, неизвестный узел отказывает.
    #[test]
    fn the_call_graph_answer_is_read_as_the_analyzer_writes_it() {
        let payloads: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/bsl_analyzer/graph-0.2.67.json"
        ))
        .expect("измеренные ответы анализатора");

        let callers = parse_call_graph_answer(&payloads["callers"], CallGraphDirection::Callers)
            .expect("входящие рёбра разобраны");
        assert_eq!(callers.state, CallGraphState::Ready);
        // Счёт пришёл из `inTotal`, а не из длины страницы.
        assert_eq!(callers.total, Some(1));
        assert_eq!(callers.edges.len(), 1);
        assert_eq!(callers.edges[0].id, "method/common/Вызывающий/Источник");
        assert_eq!(callers.edges[0].provenance, CallEdgeProvenance::Resolved);
        assert_eq!(callers.revision, Some(1));
        assert_eq!(callers.stale, Some(false));
        assert!(callers.complete);

        // У исходящего ребра соседа называет другой конец, и счёт лежит в
        // другом поле: направление — не признак, а разная форма ответа.
        let callees = parse_call_graph_answer(&payloads["callees"], CallGraphDirection::Callees)
            .expect("исходящие рёбра разобраны");
        assert_eq!(callees.total, Some(1));
        assert_eq!(callees.edges[0].id, "method/common/Вызываемый/Цель");

        // Перепутанное направление не выдаёт пустой ответ за правду: поля
        // счёта нет, и разбор отказывает.
        assert!(
            parse_call_graph_answer(&payloads["callers"], CallGraphDirection::Callees).is_err(),
            "счёт исходящих рёбер во входящем ответе не живёт"
        );

        // Индекс ещё строится: состояние названо, счёт не напечатан.
        let indexing = parse_call_graph_answer(&payloads["loading"], CallGraphDirection::Callers)
            .expect("незавершённый индекс — состояние, а не ошибка");
        assert_eq!(indexing.state, CallGraphState::Indexing);
        assert_eq!(indexing.total, None);
        assert!(indexing.edges.is_empty());

        // Неизвестный узел назван в теле ответа, и это отказ, а не пустота.
        let error = parse_call_graph_answer(&payloads["not_found"], CallGraphDirection::Callers)
            .expect_err("неизвестный узел отказывает");
        assert_eq!(error, "not_found");

        // Незнакомое происхождение не приравнивается к разрешённому.
        let mut forged = payloads["callers"].clone();
        forged["result"]["edges"][0]["provenance"] = json!("guessed");
        assert!(
            parse_call_graph_answer(&forged, CallGraphDirection::Callers)
                .expect_err("словарь происхождения закрыт")
                .contains("unknown provenance"),
        );
    }

    /// rule: INV.SOURCE.CALL-GRAPH-STATE-IS-NAMED
    #[test]
    fn call_graph_distinguishes_a_recursive_call_from_a_capped_answer() {
        let payloads: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/bsl_analyzer/graph-0.2.67.json"
        ))
        .unwrap();
        let mut recursive = payloads["callers"].clone();
        recursive["result"]["in_total"] = json!(0);
        recursive["result"]["total"] = json!(0);
        recursive["result"]["returned"] = json!(0);
        recursive["result"]["nodes"] = json!([]);
        recursive["result"]["edges"] = json!([{"kind":"call","provenance":"resolved"}]);
        let root = recursive["result"]["root"]["id"].as_str().unwrap();
        let parsed = parse_call_graph_answer(&recursive, CallGraphDirection::Callers).unwrap();
        assert_eq!(parsed.total, Some(1));
        assert_eq!(parsed.edges[0].id, root);
        assert!(parsed.complete);

        let mut capped = payloads["callers"].clone();
        capped["result"]["returned"] = json!(0);
        capped["result"]["dropped_count"] = json!(1);
        capped["result"]["edges"] = json!([]);
        let parsed = parse_call_graph_answer(&capped, CallGraphDirection::Callers).unwrap();
        assert_eq!(parsed.total, Some(1));
        assert!(!parsed.complete);

        let mut non_call = payloads["callers"].clone();
        non_call["result"]["edges"][0]["kind"] = json!("manager_access");
        assert!(
            parse_call_graph_answer(&non_call, CallGraphDirection::Callers)
                .unwrap_err()
                .contains("non-call")
        );

        let mut missing_edges = payloads["callers"].clone();
        missing_edges["result"]["total"] = json!(0);
        missing_edges["result"]["in_total"] = json!(0);
        missing_edges["result"]["returned"] = json!(0);
        missing_edges["result"]["edges"] = Value::Null;
        assert!(
            parse_call_graph_answer(&missing_edges, CallGraphDirection::Callers)
                .unwrap_err()
                .contains("edge array")
        );
    }
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    thread_local! {
        static MANUAL_NOW: RefCell<Option<Instant>> = const { RefCell::new(None) };
    }

    fn parse_bsl_analyzer_search(
        text: &str,
        context: &CodeIntelligenceContext,
    ) -> unica_code_intelligence::model::ProviderSearchSection {
        super::parse_bsl_analyzer_search(text, context, usize::MAX, &CancellationToken::new())
    }

    struct FakeRunner {
        output: ProcessOutput,
        commands: Mutex<Vec<ProcessCommand>>,
    }

    impl ProcessRunner for FakeRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.commands.lock().unwrap().push(command.clone());
            Ok(self.output.clone())
        }
    }

    fn context() -> CodeIntelligenceContext {
        CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: PathBuf::from("/workspace"),
                workspace_root: PathBuf::from("/workspace"),
                cache_root: PathBuf::from("/cache"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: PathBuf::from("/workspace/src"),
            },
            std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                std::path::Path::new("/workspace"),
            )),
        )
    }

    fn metadata_scoped_context() -> CodeIntelligenceContext {
        context().with_search_scope(CodeSearchScope {
            source_set: "main".to_string(),
            source_root: PathBuf::from("/workspace/src"),
            filters: vec![RelativeSearchFilter::Exact(PathBuf::from(
                "CommonModules/Scoped/Ext/Module.bsl",
            ))],
            excluded_subtrees: Vec::new(),
            legacy_selector: false,
        })
    }

    fn output(stdout: &str) -> ProcessOutput {
        ProcessOutput {
            status_success: true,
            status: "exit status: 0".to_string(),
            stdout: stdout.to_string(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        }
    }

    fn assert_sales_module_location(location: &SourceLocation) {
        let SourceLocation::Addressed {
            source_set,
            metadata_path,
            target_kind,
        } = location
        else {
            panic!("expected an addressed search location, got {location:?}");
        };
        assert_eq!(source_set, "main");
        assert_eq!(
            metadata_path.as_ref().map(|path| path.as_str()),
            Some("CommonModule.Sales.Module")
        );
        assert_eq!(
            *target_kind,
            unica_address::source_target::TargetKind::Module
        );
    }

    #[test]
    fn every_search_provider_uses_the_same_logical_location_projection() {
        let temporary = tempfile::tempdir().unwrap();
        let workspace_root = temporary.path().join("workspace");
        let source_root = workspace_root.join("src");
        let descriptor = source_root.join("CommonModules/Sales.xml");
        let module = source_root.join("CommonModules/Sales/Ext/Module.bsl");
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(
            workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Sales</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&module, "Procedure Post()\nEndProcedure\n").unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
            std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                &workspace_root,
            )),
        );
        let analyzer = parse_bsl_analyzer_search(
            "#1 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)\n",
            &context,
        );
        let runner = FakeRunner {
            output: output("CommonModules/Sales/Ext/Module.bsl\x001\x00Procedure Post()\n"),
            commands: Mutex::new(Vec::new()),
        };
        let lexical = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        for section in [&analyzer, &lexical] {
            assert_eq!(section.status, ProviderSectionStatus::Ok);
            assert_sales_module_location(&section.hits[0].location);
        }
    }

    #[test]
    fn search_location_projection_is_cached_per_source_file() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        static RESOLUTIONS: AtomicUsize = AtomicUsize::new(0);

        fn resolve_for_test(
            _workspace: &WorkspaceContext,
            request: &unica_navigation::source_navigation::SourceLocateRequest,
            _cancellation: &CancellationToken,
        ) -> Result<unica_navigation::source_navigation::SourceLocateResult, String> {
            RESOLUTIONS.fetch_add(1, Ordering::SeqCst);
            Ok(unica_navigation::source_navigation::SourceLocateResult {
                source_set: request.source_set.clone(),
                relative_path: request.path.clone(),
                metadata_path: None,
                target_kind: None,
                owner_metadata_path: None,
                rejection: Some(unica_source_contract::location::LocateRejection::NotAddressable),
            })
        }

        RESOLUTIONS.store(0, Ordering::SeqCst);
        let temporary = tempfile::tempdir().unwrap();
        let workspace_root = temporary.path().join("workspace");
        let source_root = workspace_root.join("src");
        let first = source_root.join("Catalogs/Items/Ext/ObjectModule.bsl");
        let second = source_root.join("Catalogs/Items/Ext/ManagerModule.bsl");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&second, "second").unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
            std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                &workspace_root,
            )),
        );
        let cancellation = CancellationToken::new();
        let mut locations = super::SearchLocationProjector::with_resolver(
            &context,
            &cancellation,
            resolve_for_test,
        );

        locations
            .project(Path::new("Catalogs/Items/Ext/ObjectModule.bsl"))
            .unwrap();
        locations
            .project(Path::new("Catalogs/Items/Ext/ObjectModule.bsl"))
            .unwrap();
        assert_eq!(RESOLUTIONS.load(Ordering::SeqCst), 1);

        locations
            .project(Path::new("Catalogs/Items/Ext/ManagerModule.bsl"))
            .unwrap();
        assert_eq!(
            RESOLUTIONS.load(Ordering::SeqCst),
            2,
            "different modules in one parent directory must not share an address"
        );

        cancellation.cancel();
        let error = locations
            .project(Path::new("Catalogs/Items/Ext/ObjectModule.bsl"))
            .expect_err("cancellation must win even over a cached location");
        assert!(error.starts_with(CANCELLED_PREFIX), "{error}");
        assert_eq!(RESOLUTIONS.load(Ordering::SeqCst), 2);
    }

    fn owned_git_scope(root: &Path, relative: &str) -> CodeIntelligenceContext {
        let root = std::fs::canonicalize(root).unwrap();
        let mut context = context();
        context.workspace.workspace_root = root.clone();
        context.workspace.cwd = root.clone();
        context.source_root.path = root.clone();
        context.with_search_scope(CodeSearchScope {
            source_set: "main".to_string(),
            source_root: root,
            filters: vec![RelativeSearchFilter::Exact(PathBuf::from(relative))],
            excluded_subtrees: Vec::new(),
            legacy_selector: false,
        })
    }

    #[test]
    fn git_grep_absent_scoped_paths_are_complete_empty_without_dispatch() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("present-parent")).unwrap();
        let runner = FakeRunner {
            output: output(""),
            commands: Mutex::new(Vec::new()),
        };
        for relative in ["present-parent/Module.bsl", "missing/Ext/Module.bsl"] {
            let section = GitGrepProvider::with_runner(
                &runner,
                crate::infrastructure::analysis_composition::source_bindings(),
            )
            .search(
                &SearchRequest {
                    query: "needle".into(),
                    limit: 20,
                },
                &owned_git_scope(root.path(), relative),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
                &CancellationToken::new(),
            );
            assert_eq!(section.status, ProviderSectionStatus::Empty, "{section:?}");
            assert!(section.hits.is_empty());
            assert!(section.diagnostics.is_empty());
            assert!(
                runner.commands.lock().unwrap().is_empty(),
                "absent scope dispatched Git"
            );
        }
    }

    #[test]
    fn git_grep_scoped_preflight_refuses_invalid_routes_without_dispatch() {
        assert_git_grep_invalid_routes(&[
            "deleted-root",
            "root-file",
            "different-root",
            "absolute",
            "foreign-absolute",
            "parent",
            "empty",
            "parent-file",
        ]);
    }

    #[test]
    fn git_grep_scoped_preflight_refuses_dangling_links_without_dispatch() {
        assert_git_grep_invalid_routes(&["dangling-link"]);
    }

    fn assert_git_grep_invalid_routes(cases: &[&str]) {
        for &case in cases {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("src");
            std::fs::create_dir(&root).unwrap();
            let mut context = owned_git_scope(&root, "missing/Module.bsl");
            match case {
                "deleted-root" => std::fs::remove_dir(&root).unwrap(),
                "root-file" => {
                    std::fs::remove_dir(&root).unwrap();
                    std::fs::write(&root, "foreign").unwrap();
                }
                "different-root" => {
                    context.search_scope.as_mut().unwrap().source_root = temp.path().to_path_buf()
                }
                "absolute" => {
                    context.search_scope.as_mut().unwrap().filters =
                        vec![RelativeSearchFilter::Exact(root.join("absent"))]
                }
                "foreign-absolute" => {
                    context.search_scope.as_mut().unwrap().filters =
                        vec![RelativeSearchFilter::Exact(PathBuf::from(
                            "C:\\outside\\Module.bsl",
                        ))];
                }
                "parent" => {
                    context.search_scope.as_mut().unwrap().filters =
                        vec![RelativeSearchFilter::Exact(PathBuf::from("../absent"))]
                }
                "empty" => {
                    context.search_scope.as_mut().unwrap().filters =
                        vec![RelativeSearchFilter::Exact(PathBuf::new())]
                }
                "parent-file" => std::fs::write(root.join("missing"), "foreign").unwrap(),
                "dangling-link" => {
                    match unica_workspace::test_support::create_test_directory_link(
                        &temp.path().join("absent"),
                        &root.join("missing"),
                    ) {
                        Ok(()) => {}
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::PermissionDenied
                                    | std::io::ErrorKind::Unsupported
                            ) =>
                        {
                            unica_process::test_support::skip_without_host_capability(
                                unica_process::test_support::HostCapability::Symlink,
                                &format!("directory-link fixture unavailable: {error}"),
                            );
                            return;
                        }
                        Err(error) => panic!("directory-link fixture failed: {error}"),
                    }
                }
                _ => unreachable!(),
            }
            let runner = FakeRunner {
                output: output(""),
                commands: Mutex::new(Vec::new()),
            };
            let section = GitGrepProvider::with_runner(
                &runner,
                crate::infrastructure::analysis_composition::source_bindings(),
            )
            .search(
                &SearchRequest {
                    query: "needle".into(),
                    limit: 20,
                },
                &context,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
                &CancellationToken::new(),
            );
            assert_eq!(
                section.status,
                ProviderSectionStatus::Failed,
                "{case}: {section:?}"
            );
            assert!(
                runner.commands.lock().unwrap().is_empty(),
                "{case} dispatched Git"
            );
        }
    }

    #[test]
    fn git_grep_scoped_preflight_checks_interruption_after_observing_absence() {
        let root = tempfile::tempdir().unwrap();
        let context = owned_git_scope(root.path(), "Module.bsl");
        for cancelled in [true, false] {
            let mut checks = 0;
            let error = super::git_grep_scope_is_absent(&context, &mut || {
                checks += 1;
                if checks == 2 {
                    Err(if cancelled {
                        unica_work_control::cancellation::cancelled_error(
                            "cancelled after observing absence",
                        )
                    } else {
                        "git-grep provider deadline exceeded".to_string()
                    })
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
            assert_eq!(checks, 2);
            assert_eq!(error.starts_with(CANCELLED_PREFIX), cancelled);
        }
    }

    #[test]
    fn git_grep_scoped_preflight_preserves_interruptions_and_existing_dispatch() {
        let root = tempfile::tempdir().unwrap();
        let mut context = owned_git_scope(root.path(), "Module.bsl");
        let runner = FakeRunner {
            output: output(""),
            commands: Mutex::new(Vec::new()),
        };
        for cancelled in [true, false] {
            let cancellation = CancellationToken::new();
            if cancelled {
                cancellation.cancel();
            }
            let deadline = ProviderDeadline::new(
                Instant::now()
                    + if cancelled {
                        Duration::from_secs(15)
                    } else {
                        Duration::ZERO
                    },
            );
            let section = GitGrepProvider::with_runner(
                &runner,
                crate::infrastructure::analysis_composition::source_bindings(),
            )
            .search(
                &SearchRequest {
                    query: "needle".into(),
                    limit: 20,
                },
                &context,
                deadline,
                &cancellation,
            );
            assert_eq!(section.status, ProviderSectionStatus::Failed);
            assert!(runner.commands.lock().unwrap().is_empty());
        }
        std::fs::write(root.path().join("Module.bsl"), "// needle").unwrap();
        context
            .search_scope
            .as_mut()
            .unwrap()
            .filters
            .push(RelativeSearchFilter::Subtree(PathBuf::from("missing")));
        GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "needle".into(),
                limit: 20,
            },
            &context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );
        assert_eq!(runner.commands.lock().unwrap().len(), 1);
    }

    /// rule: INV.RUNTIME.TIMERS-PRESERVE-WORK
    #[test]
    fn git_grep_without_deadline_passes_absence_to_the_process_runner() {
        let runner = FakeRunner {
            output: output("CommonModules/Sales/Ext/Module.bsl\x004\x00Post();\n"),
            commands: Mutex::new(Vec::new()),
        };
        let provider = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        );
        let section = provider.search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::no_deadline(),
            &CancellationToken::new(),
        );
        assert_eq!(section.status, ProviderSectionStatus::Ok);
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].timeout, None);
        assert!(!commands[0].cancellation.is_cancelled());
    }

    #[test]
    fn git_grep_is_literal_source_scoped_and_uses_the_upstream_deadline() {
        let runner = FakeRunner {
            output: output("CommonModules/Sales/Ext/Module.bsl\x004\x00Post();\n"),
            commands: Mutex::new(Vec::new()),
        };
        let provider = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        );
        let mut scope =
            CodeSearchScope::all("main".to_string(), PathBuf::from("/workspace/src"), false);
        scope.excluded_subtrees.push(PathBuf::from("lib[ab]"));
        let context = context().with_search_scope(scope);

        let section = provider.search(
            &SearchRequest {
                query: "Post.*".to_string(),
                limit: 20,
            },
            &context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(60)),
            &CancellationToken::new(),
        );

        assert_eq!(section.identity, ProviderId::GitGrep.identity());
        assert_eq!(section.status, ProviderSectionStatus::Ok);
        let commands = runner.commands.lock().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].cwd, PathBuf::from("/workspace/src"));
        assert_eq!(
            commands[0].args,
            [
                "-c",
                "core.quotepath=false",
                "grep",
                "--no-color",
                "--untracked",
                "--no-exclude-standard",
                "--null",
                "-n",
                "-F",
                "-e",
                "Post.*",
                "--",
                ":(glob)**/*.[bB][sS][lL]",
                ":(exclude,glob)**/.build/**",
                ":(exclude,literal)lib[ab]",
            ]
        );
        assert!(commands[0].args.iter().any(|arg| arg == "-F"));
        assert!(commands[0].args.iter().any(|arg| arg == "Post.*"));
        assert!(!commands[0].args.iter().any(|arg| arg == "-i"));
        let timeout = commands[0].timeout.unwrap();
        assert!(timeout > Duration::from_secs(15), "{timeout:?}");
        assert!(timeout <= Duration::from_secs(60), "{timeout:?}");
    }

    /// Git for Windows turns every `\` of a pathspec into `/`, so a backslash
    /// escape splits `Ignored[1]` into the directory `Ignored/`. The subtree
    /// prefix is quoted with one-character classes that mean the same on
    /// every platform; the real `git grep` run below proves they match.
    #[test]
    fn git_grep_subtree_pathspec_quotes_glob_characters_without_backslashes() {
        let mut scope =
            CodeSearchScope::all("main".to_string(), PathBuf::from("/workspace/src"), false);
        scope.filters = vec![
            RelativeSearchFilter::Subtree(PathBuf::from("CommonModules/Ignored[1]")),
            RelativeSearchFilter::Subtree(PathBuf::from("Odd/a*b?c]")),
        ];

        let pathspecs = git_grep_bsl_pathspecs(Some(&scope));

        assert_eq!(
            pathspecs,
            [
                ":(glob)CommonModules/Ignored[[]1[]]/**/*.[bB][sS][lL]",
                ":(glob)Odd/a[*]b[?]c[]]/**/*.[bB][sS][lL]",
            ]
        );
        assert!(
            pathspecs.iter().all(|pathspec| !pathspec.contains('\\')),
            "{pathspecs:?}"
        );
    }

    /// The generated cache is not source. The lexical corpus skips
    /// `<sourceRoot>/.build/bsl-graph.db`: `.build` sorts before every metadata
    /// directory, `git grep` emits in path order, and on a real workspace the
    /// analyzer databases there run to gigabytes. A corpus that keeps them
    /// spends the whole provider budget reading Unica's own index of the very
    /// tree it was asked to search, and publishes generated bytes in place of
    /// the source hit it displaced.
    /// rule: INV.APP.CODE-SEARCH-SOURCE-BOUNDARIES
    #[test]
    fn git_grep_excludes_the_generated_cache_from_the_lexical_corpus() {
        struct TracedGit;
        impl ProcessRunner for TracedGit {
            fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
                unica_process::runner::system_process_runner().run(command)
            }

            fn run_streaming(
                &self,
                command: &ProcessCommand,
                max_line_bytes: usize,
                on_line: &mut dyn FnMut(usize, &[u8]) -> unica_process::StreamControl,
            ) -> Result<unica_process::runner::ProcessStreamOutput, String> {
                eprintln!("git-grep command={command:?}");
                let mut records = 0;
                let started = Instant::now();
                let result = unica_process::runner::system_process_runner().run_streaming(
                    command,
                    max_line_bytes,
                    &mut |number, bytes| {
                        records += 1;
                        let reply = on_line(number, bytes);
                        eprintln!("git-grep record={number} bytes={bytes:?} consumer={reply:?}");
                        reply
                    },
                );
                eprintln!(
                    "git-grep stream elapsed={:?} records={records} result={result:?}",
                    started.elapsed()
                );
                // Compare the same command after a missing record without
                // replacing or retrying the provider result.
                if records == 0 {
                    let mut diagnostic_command = command.clone();
                    diagnostic_command.capture_limits = Some((4096, 4096));
                    diagnostic_command.timeout = Some(Duration::from_secs(2));
                    let direct =
                        unica_process::runner::system_process_runner().run(&diagnostic_command);
                    eprintln!("git-grep direct={direct:?}");
                }
                result
            }
        }
        let root = std::env::temp_dir().join(format!(
            "unica-git-grep-generated-cache-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source_root = root.join("src");
        let database = source_root.join(".build/bsl-graph.db");
        let sidecar = source_root.join(".build/entries.txt");
        // The walks skip a directory of that name at any depth, so the corpus
        // rule is recursive too. Without this one a top-level-only pathspec —
        // the shorter thing to write — passes the test: `Catalogs` sorts after
        // `.build` and before `CommonModules`, so a nested cache that survived
        // exclusion would take the single result the module expects.
        let nested = source_root.join("Catalogs/.build/entries.txt");
        let module = source_root.join("CommonModules/Продажи/Ext/Module.bsl");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        // The two shapes the real cache produces, in the order `git grep`
        // reaches them: a database whose NUL bytes make it "Binary file …
        // matches" — a record with no fields, so it lands in the diagnostics —
        // and a text sidecar that parses into a hit and takes the budget.
        let mut database_bytes = vec![0_u8];
        database_bytes.extend_from_slice("index ПолитикаУчетаСерий\n".as_bytes());
        std::fs::write(&database, &database_bytes).unwrap();
        std::fs::write(&sidecar, "index entry ПолитикаУчетаСерий\n").unwrap();
        std::fs::write(&nested, "index entry ПолитикаУчетаСерий\n").unwrap();
        // BSL-shaped cache entries pass the positive corpus pathspec; only
        // the generated-directory exclusion can keep them out of the result.
        for generated in [
            source_root.join(".build/Module.bsl"),
            source_root.join("Catalogs/.build/Module.bsl"),
        ] {
            std::fs::write(generated, "// ПолитикаУчетаСерий\n").unwrap();
        }
        std::fs::write(&module, "// ПолитикаУчетаСерий\n").unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());

        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root.clone(),
            },
            std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                &root,
            )),
        );

        // One result is the whole budget here: the generated file would take it
        // and leave the module unreachable, so the assertion below states which
        // of the two the lexical role is allowed to spend it on.
        let section = GitGrepProvider::with_runner(
            &TracedGit,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "ПолитикаУчетаСерий".to_string(),
                limit: 1,
            },
            &context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            &CancellationToken::new(),
        );

        let paths = section
            .hits
            .iter()
            .map(|hit| location_path(&hit.location).to_string())
            .collect::<Vec<_>>();
        assert!(
            !paths.iter().any(|path| path.contains(".build")),
            "the generated cache is outside the lexical corpus: {paths:?}"
        );
        eprintln!("git-grep section={section:#?}");
        assert_eq!(
            paths,
            ["CommonModules/Продажи/Ext/Module.bsl"],
            "{section:#?}"
        );
        assert!(
            section.diagnostics.is_empty(),
            "an excluded corpus reports nothing about it: {:?}",
            section.diagnostics
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_grep_excludes_only_the_literal_nested_source_root() {
        let root = tempfile::tempdir().unwrap();
        let nested = "lib[ab]";
        for directory in [nested, "liba", "libb"] {
            std::fs::create_dir(root.path().join(directory)).unwrap();
            std::fs::write(
                root.path().join(directory).join("Module.bsl"),
                "// Needle\n",
            )
            .unwrap();
        }
        assert!(std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success());

        let source_root = root.path().to_path_buf();
        let mut scope = CodeSearchScope::all("main".to_string(), source_root.clone(), false);
        scope.excluded_subtrees.push(PathBuf::from(nested));
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: source_root.clone(),
                workspace_root: source_root.clone(),
                cache_root: source_root.join(".build"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root.clone(),
            },
            std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                &source_root,
            )),
        )
        .with_search_scope(scope);
        let section = crate::infrastructure::analysis_composition::lexical_provider().search(
            &SearchRequest {
                query: "Needle".to_string(),
                limit: 20,
            },
            &context,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            &CancellationToken::new(),
        );
        let paths = section
            .hits
            .iter()
            .map(|hit| location_path(&hit.location).to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            ["liba/Module.bsl", "libb/Module.bsl"],
            "{section:#?}"
        );
    }

    #[test]
    fn git_grep_reads_tracked_untracked_and_ignored_bsl_only_in_the_selected_source() {
        let root = tempfile::tempdir().unwrap();
        let source_root = root.path().join("src");
        let tracked = "CommonModules/Tracked/Ext/Module.bsl";
        let untracked = "CommonModules/Untracked/Ext/Module.bsl";
        let ignored = "CommonModules/Ignored[1]/Ext/Module.bsl";
        let uppercase = "CommonModules/Uppercase/Ext/Module.BSL";
        for relative in [
            tracked,
            untracked,
            ignored,
            uppercase,
            "CommonModules/Ignored[1].xml",
            "CommonModules/Ignored[1]/credentials.txt",
            "credentials.txt",
            ".build/Module.bsl",
            "CommonModules/.build/Module.bsl",
            "vendor/Module.bsl",
        ] {
            let file = source_root.join(relative);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "// Needle\n").unwrap();
        }
        std::fs::create_dir(root.path().join("private")).unwrap();
        std::fs::write(root.path().join("private/Module.bsl"), "// Needle\n").unwrap();
        std::fs::write(
            root.path().join(".gitignore"),
            "**/Ignored*/\n**/Uppercase/\n*.txt\n.build/\nvendor/\nprivate/\n",
        )
        .unwrap();
        assert!(std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .args(["add", "--force", &format!("src/{tracked}")])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .args(["check-ignore", "--quiet", &format!("src/{ignored}")])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success());

        let workspace_root = std::fs::canonicalize(root.path()).unwrap();
        let source_root = std::fs::canonicalize(source_root).unwrap();

        use unica_code_intelligence::model::RelativeSearchFilter;
        for (filters, expected) in [
            (vec![], vec![ignored, tracked, untracked, uppercase]),
            (
                vec![RelativeSearchFilter::Subtree(PathBuf::from(
                    "CommonModules",
                ))],
                vec![ignored, tracked, untracked, uppercase],
            ),
            (
                vec![RelativeSearchFilter::Subtree(PathBuf::from(
                    "CommonModules/Ignored[1]",
                ))],
                vec![ignored],
            ),
            (
                vec![RelativeSearchFilter::Exact(PathBuf::from(ignored))],
                vec![ignored],
            ),
            (
                vec![RelativeSearchFilter::Exact(PathBuf::from(
                    "CommonModules/Ignored[1].xml",
                ))],
                vec![],
            ),
            (
                vec![
                    RelativeSearchFilter::Exact(PathBuf::from("CommonModules/Ignored[1].xml")),
                    RelativeSearchFilter::Subtree(PathBuf::from("CommonModules/Ignored[1]")),
                ],
                vec![ignored],
            ),
        ] {
            let mut scope = CodeSearchScope::all("main".into(), source_root.clone(), false);
            scope.filters = filters.clone();
            scope.excluded_subtrees.push(PathBuf::from("vendor"));
            let context = CodeIntelligenceContext::new(
                WorkspaceContext {
                    cwd: workspace_root.clone(),
                    workspace_root: workspace_root.clone(),
                    cache_root: source_root.join(".build"),
                    workspace_epoch: 1,
                },
                ResolvedSourceRoot {
                    source_set: Some("main".into()),
                    path: source_root.clone(),
                },
                std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                    &workspace_root,
                )),
            )
            .with_search_scope(scope);
            let section = crate::infrastructure::analysis_composition::lexical_provider().search(
                &SearchRequest {
                    query: "Needle".into(),
                    limit: 20,
                },
                &context,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
                &CancellationToken::new(),
            );
            let mut paths = section
                .hits
                .iter()
                .map(|hit| location_path(&hit.location).to_string())
                .collect::<Vec<_>>();
            paths.sort();
            assert_eq!(paths, expected, "filters={filters:?}: {section:#?}");
            assert_eq!(
                section.status,
                if expected.is_empty() {
                    ProviderSectionStatus::Empty
                } else {
                    ProviderSectionStatus::Ok
                },
                "{section:#?}"
            );
            assert!(section.search_complete, "{section:#?}");
            assert!(section.diagnostics.is_empty(), "{section:#?}");
        }
    }

    #[test]
    fn git_grep_returns_the_first_unranked_provider_traversal_prefix() {
        let runner = FakeRunner {
            output: output(
                "b/Module.bsl\x009\x00Second\n\
                 a/Module.bsl\x007\x00Later\n\
                 a/Module.bsl\x002\x00First\n",
            ),
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "needle".to_string(),
                limit: 2,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::LimitReached);
        assert_eq!(section.hits.len(), 2);
        assert_eq!(section.hits[0].rank, None);
        assert_eq!(location_path(&section.hits[0].location), "b/Module.bsl");
        assert_eq!(section.hits[0].line, 9);
        assert_eq!(section.hits[1].rank, None);
        assert_eq!(location_path(&section.hits[1].location), "a/Module.bsl");
        assert_eq!(section.hits[1].line, 7);
        assert!(section.hits.iter().all(|hit| hit.provider_score.is_none()));
    }

    #[test]
    fn git_grep_preserves_utf8_paths() {
        let runner = FakeRunner {
            output: output("Catalogs/Номенклатура.xml\x007\x00<Name>Номенклатура</Name>\n"),
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Номенклатура".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(
            location_path(&section.hits[0].location),
            "Catalogs/Номенклатура.xml"
        );
    }

    #[test]
    fn git_grep_timeout_preserves_already_streamed_hits_as_a_lower_bound() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: "a/Module.bsl\x002\x00First\nb/Module.bsl\x009\x00Second\n".to_string(),
                stderr: String::new(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "needle".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::TimedOut);
        assert_eq!(section.hits.len(), 2);
        assert!(!section.search_complete);
        assert_eq!(
            section.matches.relation,
            unica_code_intelligence::model::SearchCountRelation::LowerBound
        );
    }

    #[test]
    fn git_grep_cancellation_discards_already_streamed_hits() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "cancelled".to_string(),
                stdout: "a/Module.bsl\x002\x00First\n".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "needle".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(!section.search_complete);
        assert!(section
            .diagnostics
            .iter()
            .any(|detail| detail.contains("cancel")));
    }

    #[test]
    fn git_grep_keeps_every_row_when_the_capture_was_not_truncated() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: concat!(
                    "CommonModules/Other/Ext/Module.bsl\x0012\x00Procedure Other()\n",
                    "CommonModules/Test/Ext/Module.bsl\x001\x00Procedure Test()\n"
                )
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Procedure".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 2);
    }

    #[test]
    fn git_grep_reports_non_repository_workspace_as_unavailable() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 128".to_string(),
                stdout: String::new(),
                stderr: "fatal: not a git repository".to_string(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Procedure".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert!(section.hits.is_empty());
    }

    #[test]
    fn git_grep_exit_one_without_stderr_is_empty() {
        let runner = FakeRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 1".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "absent".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Empty);
        assert!(section.hits.is_empty());
    }

    #[test]
    fn git_grep_non_empty_malformed_output_is_failed() {
        let runner = FakeRunner {
            output: output("not-a-git-grep-row\n"),
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "needle".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(section.diagnostics[0].contains("without any valid results"));
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn git_grep_keeps_valid_hits_but_reports_malformed_siblings_as_partial() {
        let runner = FakeRunner {
            output: output("malformed\nCommonModules/Sales/Ext/Module.bsl\x001\x00Post();\n"),
            commands: Mutex::new(Vec::new()),
        };
        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Partial);
        assert_eq!(section.hits.len(), 1);
        assert!(!section.search_complete);
        assert_eq!(section.matches.relation, SearchCountRelation::LowerBound);

        let capped = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 1,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );
        assert_eq!(capped.status, ProviderSectionStatus::Partial);
        assert!(capped.diagnostics.iter().any(|item| item.contains("limit")));
    }

    /// rule: INV.APP.CODE-SEARCH-SOURCE-BOUNDARIES
    #[test]
    fn git_grep_does_not_publish_a_parent_escape_as_a_location() {
        let runner = FakeRunner {
            output: output("../outside/Secret.bsl\x002\x00Password = 1;\n"),
            commands: Mutex::new(Vec::new()),
        };

        let section = GitGrepProvider::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Password".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(!serde_json::to_string(&section)
            .unwrap()
            .contains("Secret.bsl"));
    }

    #[test]
    fn bsl_analyzer_ranked_text_parser_projects_absolute_paths_inside_source_set() {
        let section = parse_bsl_analyzer_search(
            "#1 [L+S] /workspace/src/CommonModules/Sales/Ext/Module.bsl:42-58 :: Post (procedure)\n\
               graph_id: method/common/Sales/Post\n\
               │ Procedure Post()\n\
               │     Return;\n\
             \n\
             #2 [L] Catalogs/Goods/Ext/ManagerModule.bsl:7-9 :: Find (function)\n\
               │ Function Find()\n\
             \n\
             -- semantic skipped: not configured --\n",
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 2);
        assert_eq!(
            location_path(&section.hits[0].location),
            "CommonModules/Sales/Ext/Module.bsl"
        );
        assert_eq!(section.hits[0].line, 42);
        assert_eq!(section.hits[0].end_line, Some(58));
        assert_eq!(section.hits[0].attributes["modality"], "L+S");
        assert_eq!(
            section.hits[0].attributes["graphId"],
            "method/common/Sales/Post"
        );
        assert_eq!(section.hits[0].snippet, "Procedure Post()\n    Return;");
        assert_eq!(section.hits[1].rank, Some(2));
        assert_eq!(section.hits[1].attributes["modality"], "L");
        assert_eq!(
            section.diagnostics,
            vec!["semantic skipped: not configured".to_string()]
        );
    }

    #[test]
    fn bsl_analyzer_parser_normalizes_provider_local_ranks_from_result_order() {
        let section = parse_bsl_analyzer_search(
            "#7 [L] a/Module.bsl:2 :: First (procedure)\n\
               │ Procedure First()\n\
             \n\
             #9 [S] b/Module.bsl:4 :: Second (procedure)\n\
               │ Procedure Second()\n",
            &context(),
        );

        assert_eq!(
            section.hits.iter().map(|hit| hit.rank).collect::<Vec<_>>(),
            vec![Some(1), Some(2)]
        );
    }

    #[test]
    fn bsl_analyzer_does_not_publish_an_absolute_path_outside_source_set() {
        let section = parse_bsl_analyzer_search(
            "#1 [L] /outside/Secret.bsl:2 :: Password (variable)\n\
               │ Password = 1;\n",
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(!serde_json::to_string(&section)
            .unwrap()
            .contains("Secret.bsl"));
    }

    #[test]
    fn bsl_analyzer_budget_envelope_reaches_the_provider_limit_without_failing() {
        // Так анализатор отвечает на обычный запрос к большой конфигурации:
        // вывод не помещается в его бюджет, и лишние попадания отброшены.
        let envelope = serde_json::json!({
            "action": "search_code",
            "budget_exhausted": true,
            "freshness": {"completeness": {"status": "partial", "reasons": [
                {"code": "output_budget", "detail": "hits trimmed to fit max_output_tokens"}
            ]}},
            "shown": 1,
            "total": 20,
            "hits": [{
                "rank": 1,
                "graph_id": "method/common/Module/Run",
                "kind": "procedure",
                "line_start": 3,
                "line_end": 7,
                "location": {"path": "Module.bsl"},
                "modality": "L",
                "path": "Module.bsl",
                "snippet": "Procedure Run()",
                "symbol": "Run"
            }]
        });

        let section = parse_bsl_analyzer_search(&envelope.to_string(), &context());

        assert_eq!(section.status, ProviderSectionStatus::LimitReached);
        assert_eq!(
            section.termination.as_ref().unwrap().code,
            unica_code_intelligence::model::SearchTerminationCode::LimitReached
        );
        assert!(!section.search_complete);
        assert_eq!(section.hits.len(), 1);
        let hit = &section.hits[0];
        assert_eq!(hit.rank, Some(1));
        assert_eq!(hit.line, 3);
        assert_eq!(hit.end_line, Some(7));
        assert_eq!(hit.symbol.as_deref(), Some("Run"));
        assert_eq!(hit.kind.as_deref(), Some("procedure"));
        assert_eq!(hit.snippet, "Procedure Run()");
        assert_eq!(hit.attributes["graphId"], "method/common/Module/Run");
        assert!(section
            .diagnostics
            .iter()
            .any(|line| line.contains("max_output_tokens")));
    }

    #[test]
    fn bsl_analyzer_budget_envelope_cannot_name_a_place_outside_the_source() {
        let envelope = serde_json::json!({
            "budget_exhausted": true,
            "hits": [{"line_start": 2, "location": {"path": "/outside/Secret.bsl"}}]
        });

        let section = parse_bsl_analyzer_search(&envelope.to_string(), &context());

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(!serde_json::to_string(&section)
            .unwrap()
            .contains("Secret.bsl"));
    }

    #[test]
    fn bsl_analyzer_not_ready_envelope_is_a_retryable_dependency() {
        // The analyzer names this state `Pending` and ships `retry_after_ms`
        // alongside: it promises the next attempt can differ. Flattening that
        // promise into a permanent `unavailable` is what made the release
        // assessment refuse to wait for an index that was about to finish.
        let section = parse_bsl_analyzer_search(
            r#"{"status":"not_ready","detail":"indexing 40%","retry_after_ms":1500}"#,
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::TimedOut);
        let termination = section.termination.as_ref().unwrap();
        assert_eq!(
            termination.code,
            unica_code_intelligence::model::SearchTerminationCode::DependencyPending
        );
        assert!(termination.retryable);
        assert_eq!(termination.detail_code.as_deref(), Some("buildingIndex"));
        assert_eq!(section.diagnostics, vec!["indexing 40%".to_string()]);
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn bsl_analyzer_empty_results_with_index_ready_footer_are_empty() {
        for text in [
            "No results found.",
            "No results found.\nIndexing lexical: ready\nIndexing semantic: disabled",
            "No results found.\nIndexing lexical: ready\nIndexing semantic: ready\n",
            "No results found.\nIndexing semantic: ready\nIndexing lexical: ready",
            "No results found.\nIndexing semantic: disabled\nIndexing lexical: ready",
        ] {
            let section = parse_bsl_analyzer_search(text, &context());
            assert_eq!(section.status, ProviderSectionStatus::Empty);
            assert!(section.search_complete);
            assert!(section.hits.is_empty());
            assert_eq!(section.matches.total, Some(0));
        }
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn bsl_analyzer_empty_marker_does_not_hide_untrusted_tail() {
        for text in [
            "No results found.\nunknown provider error",
            "No results found.\nIndexing lexical: ready",
            "No results found.\nIndexing semantic: ready",
            "No results found.\nIndexing semantic: disabled",
            "No results found.\nIndexing lexical: ready\nIndexing lexical: ready",
            "No results found.\nIndexing lexical: ready\nIndexing lexical: ready\nIndexing semantic: ready",
            "No results found.\nIndexing lexical: ready\nIndexing semantic: ready\nIndexing semantic: ready",
            "No results found.\nIndexing lexical: ready\nIndexing semantic: ready\nIndexing semantic: disabled",
            "No results found.\nIndexing lexical: failed\nIndexing semantic: disabled",
            "No results found.\nIndexing lexical: ready\nIndexing semantic: failed",
            "No results found.\nIndexing lexical: disabled\nIndexing semantic: running",
            "No results found.\nIndexing lexical: cancelled\nIndexing semantic: disabled",
            "No results found.\nIndexing lexical: superseded\nIndexing semantic: disabled",
            "No results found.\nIndexing lexical: ready\nIndexing semantic: superseded",
            "No results found.\nIndexing lexical: ready\nIndexing semantic: unknown",
            "No results found.\nIndexing lexical: waiting: 1/2 chunks\nIndexing semantic: disabled",
            "No results found.\nIndexing lexical: unknown extra\nIndexing semantic: disabled",
            "No results found.\nIndexing lexical: runningx\nIndexing semantic: disabled",
            "No results found.\nIndexing lexical: ready\nIndexing semantic: ready junk",
            "No results found.\nIndexing lexical: running\nIndexing semantic: failed",
            "No results found.\nIndexing lexical: running\nIndexing lexical: running",
            "No results found.\nIndexing lexical: running\nIndexing semantic: ready\nextra",
            "No results found.\n#broken header",
            "No results found.\nNo results found.",
        ] {
            let section = parse_bsl_analyzer_search(text, &context());
            assert_eq!(section.status, ProviderSectionStatus::Failed, "{text}");
            assert!(!section.search_complete, "{text}");
            assert_eq!(section.matches.total, None, "{text}");
            assert_eq!(section.matches.relation, SearchCountRelation::Unknown, "{text}");
        }
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn bsl_analyzer_busy_or_building_index_is_a_retryable_wait_not_an_empty_answer() {
        use unica_code_intelligence::model::SearchTerminationCode;
        for (text, code, detail) in [
            (
                "No results found.\nIndexing pending: initializing (no live counters yet)",
                SearchTerminationCode::DependencyPending,
                Some("buildingIndex"),
            ),
            (
                "No results found (overlay is warming up, only baseline search available).",
                SearchTerminationCode::DependencyPending,
                Some("updatingIndex"),
            ),
            (
                "Search index is busy (a long operation is holding it); please try again in a moment.",
                SearchTerminationCode::CapacityExhausted,
                None,
            ),
            (
                "Semantic search is busy (a long operation is holding the index). Lexical search is available in the meantime.",
                SearchTerminationCode::CapacityExhausted,
                None,
            ),
        ] {
            let section = parse_bsl_analyzer_search(text, &context());
            let termination = section.termination.as_ref().expect(text);
            assert_eq!(termination.code, code, "{text}");
            assert!(termination.retryable, "{text}");
            assert_eq!(termination.detail_code.as_deref(), detail, "{text}");
            assert!(!section.search_complete, "{text}");
            assert!(section.hits.is_empty(), "{text}");
            assert_ne!(section.status, ProviderSectionStatus::Empty, "{text}");
            assert_ne!(section.matches.relation, SearchCountRelation::Exact, "{text}");
        }
        // A failed index state or any other line is no wait. Index footers
        // proper are covered by the queued-or-running test below.
        for text in [
            "No results found.\nIndexing lexical: running\nIndexing semantic: failed",
            "No results found.\nIndexing lexical: running\nIndexing semantic: failed (embedding)",
            "No results found.\nIndexing lexical: running\nunknown provider error",
            "No results found (overlay is warming up, only baseline search available).\nIndexing lexical: failed",
            "Search index is busy (a long operation is holding it); please try again in a moment.\nunexpected tail",
        ] {
            let section = parse_bsl_analyzer_search(text, &context());
            assert_eq!(section.status, ProviderSectionStatus::Failed, "{text}");
        }
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn bsl_analyzer_empty_marker_with_a_queued_or_running_index_is_pending() {
        // After a published edit the analyzer answers before its index update
        // lands. No hits then prove nothing, but the provider has not failed
        // either: the next attempt is promised to differ.
        for (text, detail_code) in [
            (
                "No results found.\nIndexing lexical: running (parsing): 1/2 chunks\nIndexing semantic: disabled",
                "updatingIndex",
            ),
            (
                "No results found.\nIndexing semantic: ready\nIndexing lexical: running",
                "updatingIndex",
            ),
            (
                "No results found.\nIndexing lexical: ready\nIndexing semantic: running",
                "buildingIndex",
            ),
            (
                "No results found.\nIndexing lexical: waiting\nIndexing semantic: ready",
                "indexQueued",
            ),
            // Seen live right after an edit: the analyzer could not snapshot the
            // index its update was holding.
            (
                "No results found.\nIndexing lexical: unknown\nIndexing semantic: unknown",
                "updatingIndex",
            ),
        ] {
            let section = parse_bsl_analyzer_search(text, &context());
            assert_eq!(section.status, ProviderSectionStatus::TimedOut, "{text}");
            assert!(!section.search_complete, "{text}");
            assert_ne!(section.matches.relation, SearchCountRelation::Exact, "{text}");
            let termination = section.termination.as_ref().unwrap();
            assert_eq!(
                termination.code,
                unica_code_intelligence::model::SearchTerminationCode::DependencyPending,
                "{text}"
            );
            assert!(termination.retryable, "{text}");
            assert_eq!(termination.detail_code.as_deref(), Some(detail_code), "{text}");
        }
    }

    #[test]
    fn bsl_analyzer_non_empty_malformed_output_is_failed() {
        let section = parse_bsl_analyzer_search(
            "plain output from an incompatible analyzer\n#broken header\n",
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section.hits.is_empty());
        assert!(section.diagnostics[0].contains("without any valid search hits"));
        assert!(section
            .diagnostics
            .iter()
            .any(|item| item.contains("malformed bsl-analyzer search header")));
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn bsl_analyzer_does_not_claim_complete_when_one_header_is_unreadable() {
        let section = parse_bsl_analyzer_search(
            "#broken header\n#2 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)\n",
            &context(),
        );
        assert_eq!(section.hits.len(), 1);
        assert_eq!(section.status, ProviderSectionStatus::Partial);
        assert!(!section.search_complete);
        assert_eq!(section.matches.relation, SearchCountRelation::LowerBound);
        assert_eq!(
            section.termination.unwrap().code,
            unica_code_intelligence::model::SearchTerminationCode::ProviderFailed
        );

        let capped = super::parse_bsl_analyzer_search(
            "#broken header\n#2 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)\n",
            &context(),
            2,
            &CancellationToken::new(),
        );
        assert_eq!(capped.status, ProviderSectionStatus::Partial);
        assert!(capped.diagnostics.iter().any(|item| item.contains("limit")));
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn cancelled_projection_is_not_reported_as_a_malformed_search_result() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let analyzer = super::parse_bsl_analyzer_search(
            "#1 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)\n",
            &context(),
            20,
            &cancellation,
        );
        {
            let section = analyzer;
            assert_eq!(section.status, ProviderSectionStatus::Failed);
            assert!(section.hits.is_empty());
            assert!(section.diagnostics[0].starts_with(CANCELLED_PREFIX));
        }
    }

    #[test]
    fn bsl_analyzer_separator_without_text_does_not_add_empty_diagnostic() {
        let section = parse_bsl_analyzer_search(
            "#1 [L] CommonModules/X/Ext/Module.bsl:2 :: First (procedure)\n\
               │ Procedure First()\n\
             \n\
             ----\n",
            &context(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 1);
        assert!(section
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.is_empty()));
    }

    struct FakeBslClient {
        calls: Mutex<Vec<(PathBuf, Value, Duration)>>,
        output: McpToolOutput,
    }

    struct FailingBslClient {
        error: String,
    }

    impl BslToolClient for FailingBslClient {
        fn call(
            &self,
            _tool: &'static str,
            _context: &CodeIntelligenceContext,
            _arguments: Value,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<McpToolOutput, String> {
            Err(self.error.clone())
        }

        fn call_before(
            &self,
            _tool: &'static str,
            _context: &CodeIntelligenceContext,
            _arguments: Value,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
            _yields: bool,
        ) -> Result<McpToolOutput, String> {
            Err(self.error.clone())
        }
    }

    #[test]
    fn bsl_analyzer_missing_bundled_runtime_is_unavailable() {
        let client = FailingBslClient {
            error: "could not locate Unica plugin root for workspace bsl-analyzer service"
                .to_string(),
        };

        let section = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert_eq!(section.diagnostics, vec![client.error]);
    }

    impl BslToolClient for FakeBslClient {
        fn call(
            &self,
            _tool: &'static str,
            context: &CodeIntelligenceContext,
            arguments: Value,
            timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<McpToolOutput, String> {
            self.calls
                .lock()
                .unwrap()
                .push((context.source_root.path.clone(), arguments, timeout));
            Ok(self.output.clone())
        }

        fn call_before(
            &self,
            _tool: &'static str,
            _context: &CodeIntelligenceContext,
            _arguments: Value,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
            _yields: bool,
        ) -> Result<McpToolOutput, String> {
            Ok(self.output.clone())
        }
    }

    /// rule: INV.RUNTIME.TIMERS-PRESERVE-WORK
    #[test]
    fn bsl_search_without_deadline_refuses_before_legacy_service_dispatch() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: "[]".to_string(),
                stderr: String::new(),
            },
        };
        let section = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::no_deadline(),
            &CancellationToken::new(),
        );
        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        let termination = section.termination.as_ref().unwrap();
        assert_eq!(
            termination.code,
            unica_code_intelligence::model::SearchTerminationCode::ProviderUnavailable
        );
        assert!(!termination.retryable);
        assert_eq!(termination.detail_code, None);
        assert_eq!(
            section.diagnostics,
            vec![
                unica_work_control::provider_deadline::ProviderDeadlineError::Unsupported
                    .to_string()
            ]
        );
        assert!(client.calls.lock().unwrap().is_empty());
    }

    /// rule: INV.RUNTIME.TIMERS-PRESERVE-WORK
    #[test]
    fn bsl_graph_without_deadline_refuses_before_legacy_service_dispatch() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: "[]".to_string(),
                stderr: String::new(),
            },
        };
        let outcome = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .read(
            &CodeIntelligenceReadRequest::CallGraph {
                id: "method/common/Module/Post".to_string(),
                direction: CallGraphDirection::Callers,
                limit: 20,
            },
            &context(),
            ProviderDeadline::no_deadline(),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(!outcome.ok);
        assert_eq!(
            outcome.errors,
            vec![
                unica_work_control::provider_deadline::ProviderDeadlineError::Unsupported
                    .to_string()
            ]
        );
        assert!(client.calls.lock().unwrap().is_empty());
    }

    /// Порт отвечает графом, а не только объявляет возможность.
    ///
    /// Канал подменён двойником, который отдаёт замеренный ответ анализатора:
    /// так проверяется путь целиком — аргументы запроса, разбор, состояние и
    /// перенос отказа, — не поднимая движок.
    #[test]
    fn the_call_graph_capability_answers_through_the_analyzer_channel() {
        let payloads: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/bsl_analyzer/graph-0.2.67.json"
        ))
        .expect("измеренные ответы анализатора");
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: payloads["callers"].to_string(),
                stderr: String::new(),
            },
        };
        let outcome = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .read(
            &CodeIntelligenceReadRequest::CallGraph {
                id: "method/common/Вызываемый/Цель".to_string(),
                direction: CallGraphDirection::Callers,
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        )
        .expect("порт отвечает");

        assert!(outcome.ok, "{outcome:?}");
        // Запрос называет действие анализатора и границу страницы.
        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1["action"], "callers");
        assert_eq!(calls[0].1["id"], "method/common/Вызываемый/Цель");
        assert_eq!(calls[0].1["max_nodes"], 20);
        assert_eq!(calls[0].1["edge_kinds"], json!(["call"]));
        let Some(CodeIntelligenceReadData::CallGraph(graph)) = outcome.data else {
            panic!("порт отдаёт типизированный граф: {:?}", outcome.data);
        };
        assert_eq!(graph.state, CallGraphState::Ready);
        assert_eq!(graph.total, Some(1));
        assert_eq!(graph.edges[0].provenance, CallEdgeProvenance::Resolved);

        let mut wrong_root = payloads["callers"].clone();
        wrong_root["result"]["root"]["id"] = json!("method/common/Чужой/Метод");
        let wrong_client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: wrong_root.to_string(),
                stderr: String::new(),
            },
        };
        assert!(BslAnalyzerProvider::new(
            &wrong_client,
            crate::infrastructure::analysis_composition::source_bindings()
        )
        .read(
            &CodeIntelligenceReadRequest::CallGraph {
                id: "method/common/Вызываемый/Цель".to_string(),
                direction: CallGraphDirection::Callers,
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        )
        .unwrap_err()
        .contains("another method"));

        // Недостижимый движок — названное состояние, а не нулевой счёт:
        // «графа нет» и «вызовов нет» — разные ответы.
        let failing = FailingBslClient {
            error: "bundled bsl-analyzer is not installed".to_string(),
        };
        let refused = BslAnalyzerProvider::new(
            &failing,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .read(
            &CodeIntelligenceReadRequest::CallGraph {
                id: "method/common/Вызываемый/Цель".to_string(),
                direction: CallGraphDirection::Callees,
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        )
        .expect("недостижимый движок — ответ, а не паника");
        assert!(!refused.ok);
        let Some(CodeIntelligenceReadData::CallGraph(graph)) = refused.data else {
            panic!("состояние названо даже при отказе: {:?}", refused.data);
        };
        assert_eq!(graph.state, CallGraphState::Unavailable);
        assert_eq!(graph.total, None);
    }
    #[test]
    fn bsl_analyzer_provider_uses_persistent_search_tool_with_resolved_context() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: "No results found.".to_string(),
                stderr: "building call graph for unrelated modules".to_string(),
            },
        };
        let provider = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        );

        let section = provider.search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 50,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(120)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Empty);
        assert_eq!(section.index_freshness.as_deref(), Some("unknown"));
        assert!(section.diagnostics.is_empty());
        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, PathBuf::from("/workspace/src"));
        assert_eq!(calls[0].1["action"], "search_code");
        assert_eq!(calls[0].1["query"], "Post");
        assert_eq!(calls[0].1["limit"], 50);
        assert_eq!(calls[0].1["max_output_tokens"], 50_000);
        assert!(calls[0].2 <= Duration::from_secs(120));
    }

    #[test]
    fn bsl_analyzer_listing_frame_is_not_reported_as_unparsed_output() {
        let section = parse_bsl_analyzer_search(
            "Modality tag per hit: [L] lexical-only\n\
             #1 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)\n\
             \n\
             -- semantic skipped: not configured (set EMBEDDING_URL) --\n\
             Indexing lexical: ready\n\
             Indexing semantic: disabled\n",
            &context(),
        );
        assert_eq!(section.hits.len(), 1);
        assert_eq!(
            section.diagnostics,
            vec!["semantic skipped: not configured (set EMBEDDING_URL)".to_string()]
        );

        // Неготовый индекс рядом с попаданиями не прячется.
        let pending = parse_bsl_analyzer_search(
            "#1 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)\n\
             \n\
             Indexing lexical: running\n",
            &context(),
        );
        assert!(pending
            .diagnostics
            .iter()
            .any(|line| line.contains("Indexing lexical: running")));
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn bsl_analyzer_process_log_never_reaches_the_search_answer() {
        // Журнал сборки индекса растёт вместе с процессом: попав в ответ, он
        // занимал страницу и вытеснял попадания.
        let log = "2026-11-06T16:19:28.850332300+05:00  WARN table not found alias=secret-marker\n"
            .repeat(4_000);
        let budget_envelope = serde_json::json!({
            "budget_exhausted": true,
            "freshness": {"completeness": {"reasons": [
                {"code": "output_budget", "detail": "hits trimmed to fit max_output_tokens"}
            ]}},
            "hits": [{
                "line_start": 3,
                "location": {"path": "Module.bsl"},
                "snippet": "Procedure Run()",
                "symbol": "Run"
            }]
        })
        .to_string();
        for (reply, status, reason) in [
            (
                budget_envelope.as_str(),
                ProviderSectionStatus::LimitReached,
                Some("max_output_tokens"),
            ),
            (
                "#broken header\n#2 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)",
                ProviderSectionStatus::Partial,
                Some("malformed"),
            ),
            (
                r#"{"status":"not_ready","detail":"indexing 40%","retry_after_ms":1500}"#,
                ProviderSectionStatus::TimedOut,
                Some("indexing 40%"),
            ),
            (
                "unexpected analyzer reply",
                ProviderSectionStatus::Failed,
                None,
            ),
        ] {
            let client = FakeBslClient {
                calls: Mutex::new(Vec::new()),
                output: McpToolOutput {
                    result_text: reply.to_string(),
                    stderr: log.clone(),
                },
            };
            let provider = BslAnalyzerProvider::new(
                &client,
                crate::infrastructure::analysis_composition::source_bindings(),
            );

            let section = provider.search(
                &SearchRequest {
                    query: "Post".to_string(),
                    limit: 50,
                },
                &context(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(120)),
                &CancellationToken::new(),
            );

            assert_eq!(section.status, status, "{reply}");
            let text = serde_json::to_string(&section).unwrap();
            assert!(!text.contains("secret-marker"), "{reply}: {text}");
            assert!(text.len() < 4_096, "{reply}: {} bytes", text.len());
            if let Some(reason) = reason {
                assert!(
                    section.diagnostics.iter().any(|line| line.contains(reason)),
                    "{reply}: {:?}",
                    section.diagnostics
                );
            }
        }

        // Причина после журнала остаётся, но одной короткой строкой.
        let fatal = format!("fatal: graph database is unavailable {}", "x".repeat(5_000));
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: "unexpected analyzer reply".to_string(),
                stderr: format!("{log}{fatal}\n"),
            },
        };
        let section = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 50,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(120)),
            &CancellationToken::new(),
        );
        let text = serde_json::to_string(&section).unwrap();
        assert!(
            text.contains("fatal: graph database is unavailable"),
            "{text}"
        );
        assert!(!text.contains("secret-marker"), "{text}");
        assert!(text.len() < 4_096, "{} bytes", text.len());
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn bsl_analyzer_at_requested_limit_does_not_claim_exhaustion() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: "#1 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)\n"
                    .to_string(),
                stderr: String::new(),
            },
        };
        let provider = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        );
        let section = provider.search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 1,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );
        assert_eq!(section.hits.len(), 1);
        assert_eq!(section.status, ProviderSectionStatus::LimitReached);
        assert!(!section.search_complete);
        assert_eq!(section.matches.relation, SearchCountRelation::LowerBound);

        let section = provider.search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 2,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );
        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert!(section.search_complete);
        assert_eq!(section.matches.relation, SearchCountRelation::Exact);
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn bsl_analyzer_at_its_internal_cap_does_not_claim_exhaustion() {
        let output = (1..=50)
            .map(|rank| {
                format!(
                    "#{rank} [L] CommonModules/Sales/Ext/Module.bsl:{rank} :: Post{rank} (procedure)\n"
                )
            })
            .collect::<String>();
        let section =
            super::parse_bsl_analyzer_search(&output, &context(), 200, &CancellationToken::new());
        assert_eq!(section.hits.len(), 50);
        assert_eq!(section.status, ProviderSectionStatus::LimitReached);
        assert!(!section.search_complete);
        assert_eq!(section.matches.relation, SearchCountRelation::LowerBound);
    }

    /// rule: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
    #[test]
    fn bsl_analyzer_output_budget_truncation_does_not_claim_exhaustion() {
        let section = super::parse_bsl_analyzer_search(
            "#1 [L] CommonModules/Sales/Ext/Module.bsl:1 :: Post (procedure)\n\
             -- showing 1 of 5 results (truncated to fit max_output_tokens; raise the budget or narrow the query) --\n",
            &context(),
            200,
            &CancellationToken::new(),
        );
        assert_eq!(section.hits.len(), 1);
        assert_eq!(section.status, ProviderSectionStatus::LimitReached);
        assert!(!section.search_complete);
        assert_eq!(section.matches.relation, SearchCountRelation::LowerBound);
    }

    /// rule: INV.APP.CODE-SEARCH-SOURCE-BOUNDARIES
    #[test]
    fn bsl_analyzer_does_not_broaden_metadata_scoped_search() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: "No results found.".to_string(),
                stderr: String::new(),
            },
        };

        let section = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &metadata_scoped_context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Unavailable);
        assert!(section.diagnostics.join(" ").contains("metadataPath"));
        assert!(client.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn bsl_analyzer_successful_search_omits_provider_stderr() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: "#1 [L] CommonModules/Sales/Ext/Module.bsl:42 :: Post (procedure)\n"
                    .to_string(),
                stderr: "building call graph for unrelated modules\nwarning: unrelated module"
                    .to_string(),
            },
        };

        let section = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Ok);
        assert_eq!(section.hits.len(), 1);
        assert!(section.diagnostics.is_empty());
    }

    #[test]
    fn bsl_analyzer_pending_index_search_keeps_provider_stderr() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text:
                    r#"{"status":"not_ready","detail":"indexing 40%","retry_after_ms":1500}"#
                        .to_string(),
                stderr: "waiting for the BSL index".to_string(),
            },
        };

        let section = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::TimedOut);
        assert_eq!(
            section.termination.as_ref().unwrap().code,
            unica_code_intelligence::model::SearchTerminationCode::DependencyPending
        );
        assert!(section
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("waiting for the BSL index")));
    }

    #[test]
    fn bsl_analyzer_failed_search_keeps_provider_stderr() {
        let client = FakeBslClient {
            calls: Mutex::new(Vec::new()),
            output: McpToolOutput {
                result_text: "incompatible analyzer output".to_string(),
                stderr: "fatal: graph database is unavailable".to_string(),
            },
        };

        let section = BslAnalyzerProvider::new(
            &client,
            crate::infrastructure::analysis_composition::source_bindings(),
        )
        .search(
            &SearchRequest {
                query: "Post".to_string(),
                limit: 20,
            },
            &context(),
            ProviderDeadline::new(Instant::now() + Duration::from_secs(15)),
            &CancellationToken::new(),
        );

        assert_eq!(section.status, ProviderSectionStatus::Failed);
        assert!(section
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("fatal: graph database is unavailable")));
    }

    // This client exercises the production wait loop. Only the clock,
    // service boundary and sleep are controlled; no ready-made attempt is injected.

    #[test]
    fn an_outline_names_the_module_it_read_as_an_absolute_artifact() {
        // Every other adapter reports a filesystem artifact as a path a caller
        // can open, so echoing the relative argument back would name no location.
        let root = std::env::temp_dir().join(format!(
            "unica-outline-artifacts-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source_root = root.join("src");
        let module = source_root.join("CommonModules/X/Ext/Module.bsl");
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(&module, "Процедура П() Экспорт\nКонецПроцедуры\n").unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
            std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                &root,
            )),
        );
        let request = CodeIntelligenceReadRequest::Outline {
            path: "CommonModules/X/Ext/Module.bsl".to_string(),
            include_methods: true,
        };

        let outcome = crate::infrastructure::analysis_composition::bsl_provider()
            .read(
                &request,
                &context,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(outcome.artifacts.len(), 1);
        let artifact = PathBuf::from(&outcome.artifacts[0]);
        assert!(artifact.is_absolute(), "{artifact:?}");
        assert!(artifact.is_file(), "{artifact:?}");
        assert!(
            artifact.ends_with("CommonModules/X/Ext/Module.bsl"),
            "{artifact:?}"
        );

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let stopped = crate::infrastructure::analysis_composition::bsl_provider()
            .read(
                &request,
                &context,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
                &cancelled,
            )
            .unwrap();

        // Same shape as `AdapterOutcome::cancelled`: the prefixed error is the
        // summary and a stopped call claims no artifacts.
        assert!(!stopped.ok);
        assert!(
            stopped.summary.starts_with(CANCELLED_PREFIX),
            "{}",
            stopped.summary
        );
        assert_eq!(stopped.errors, vec![stopped.summary.clone()]);
        assert!(stopped.artifacts.is_empty(), "{:?}", stopped.artifacts);
        assert!(stopped.data.is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unsuccessful_outline_claims_no_module_artifact() {
        let root = std::env::temp_dir().join(format!(
            "unica-outline-failed-artifacts-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source_root = root.join("src");
        let valid_module = source_root.join("CommonModules/Valid/Ext/Module.bsl");
        let invalid_module = source_root.join("CommonModules/Invalid/Ext/Module.bsl");
        std::fs::create_dir_all(valid_module.parent().unwrap()).unwrap();
        std::fs::create_dir_all(invalid_module.parent().unwrap()).unwrap();
        std::fs::write(
            &valid_module,
            "Процедура Проверить() Экспорт\nКонецПроцедуры\n",
        )
        .unwrap();
        std::fs::write(
            &invalid_module,
            "Процедура Сломана(\nКонецПроцедуры\nЕсли Тогда\n",
        )
        .unwrap();
        let context = CodeIntelligenceContext::new(
            WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
            ResolvedSourceRoot {
                source_set: Some("main".to_string()),
                path: source_root,
            },
            std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                &root,
            )),
        );
        let cancellation = CancellationToken::new();
        let cases = [
            (
                "missing module",
                "CommonModules/Missing/Ext/Module.bsl",
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            ),
            (
                "directory instead of module",
                "CommonModules",
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            ),
            (
                "parser diagnostic",
                "CommonModules/Invalid/Ext/Module.bsl",
                ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
            ),
            (
                "deadline before reading",
                "CommonModules/Valid/Ext/Module.bsl",
                ProviderDeadline::new(Instant::now()),
            ),
        ];

        for (label, path, deadline) in cases {
            let outcome = crate::infrastructure::analysis_composition::bsl_provider()
                .read(
                    &CodeIntelligenceReadRequest::Outline {
                        path: path.to_string(),
                        include_methods: true,
                    },
                    &context,
                    deadline,
                    &cancellation,
                )
                .unwrap();

            assert!(!outcome.ok, "{label}: {outcome:?}");
            assert!(
                outcome.artifacts.is_empty(),
                "{label} claimed artifacts: {:?}",
                outcome.artifacts
            );
            assert!(outcome.data.is_none(), "{label}: {:?}", outcome.data);
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}

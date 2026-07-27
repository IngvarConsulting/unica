use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use syn::{
    visit::{self, Visit},
    Expr, ExprCall, ExprMethodCall, ExprPath, ImplItem, Item, ItemUse, Lit, TraitItem, Type,
    UseTree,
};

#[derive(Debug)]
enum CallFact {
    Path(Vec<String>),
    Method(String),
    Ufcs {
        self_type: Vec<String>,
        trait_path: Vec<String>,
        method: String,
    },
}

#[derive(Debug, Default)]
struct FunctionFacts {
    calls: Vec<CallFact>,
    forbidden: Vec<String>,
}

#[derive(Debug)]
struct FunctionNode {
    module: String,
    imports: BTreeMap<String, Vec<String>>,
    facts: FunctionFacts,
}

#[derive(Debug, Clone)]
struct MethodTarget {
    id: String,
    owner: Vec<String>,
    trait_path: Option<Vec<String>>,
}

#[derive(Debug, Default)]
struct CallGraph {
    functions: BTreeMap<String, FunctionNode>,
    methods_by_name: BTreeMap<String, Vec<MethodTarget>>,
    associated_methods: BTreeMap<String, BTreeSet<String>>,
}

impl CallGraph {
    fn from_sources(sources: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut graph = Self::default();
        for (module, source) in sources {
            let file = syn::parse_file(&source)
                .unwrap_or_else(|error| panic!("failed to parse {module}: {error}"));
            graph.insert_items(&module, &file.items);
        }
        graph
    }

    fn insert_items(&mut self, module: &str, items: &[Item]) {
        let imports = module_imports(module, items);
        for item in items {
            match item {
                Item::Fn(function) => {
                    let id = format!("{module}::{}", function.sig.ident);
                    self.insert_body(&id, module, &imports, &function.block);
                }
                Item::Impl(item_impl) => {
                    let Some(owner) = type_path_segments(&item_impl.self_ty)
                        .map(|path| resolve_decl_path(module, &imports, path))
                    else {
                        continue;
                    };
                    let trait_path = item_impl.trait_.as_ref().map(|(_, path, _)| {
                        resolve_decl_path(module, &imports, path_segments(path))
                    });
                    for impl_item in &item_impl.items {
                        let ImplItem::Fn(method) = impl_item else {
                            continue;
                        };
                        let method_name = method.sig.ident.to_string();
                        let trait_label = trait_path
                            .as_ref()
                            .map(|path| format!(" as {}", path.join("::")))
                            .unwrap_or_default();
                        let id = format!(
                            "{module}::<{}{}>::{method_name}",
                            owner.join("::"),
                            trait_label
                        );
                        self.insert_body(&id, module, &imports, &method.block);
                        self.insert_method_target(
                            MethodTarget {
                                id,
                                owner: owner.clone(),
                                trait_path: trait_path.clone(),
                            },
                            &method_name,
                        );
                    }
                }
                Item::Trait(item_trait) => {
                    let trait_path =
                        resolve_decl_path(module, &imports, vec![item_trait.ident.to_string()]);
                    for trait_item in &item_trait.items {
                        let TraitItem::Fn(method) = trait_item else {
                            continue;
                        };
                        let Some(default) = &method.default else {
                            continue;
                        };
                        let method_name = method.sig.ident.to_string();
                        let id =
                            format!("{module}::<trait {}>::{method_name}", trait_path.join("::"));
                        self.insert_body(&id, module, &imports, default);
                        self.insert_method_target(
                            MethodTarget {
                                id,
                                owner: trait_path.clone(),
                                trait_path: Some(trait_path.clone()),
                            },
                            &method_name,
                        );
                    }
                }
                Item::Mod(item_mod) => {
                    if let Some((_, nested)) = &item_mod.content {
                        self.insert_items(&format!("{module}::{}", item_mod.ident), nested);
                    }
                }
                _ => {}
            }
        }
    }

    fn insert_body(
        &mut self,
        id: &str,
        module: &str,
        imports: &BTreeMap<String, Vec<String>>,
        block: &syn::Block,
    ) {
        let mut visitor = FactVisitor::default();
        visitor.visit_block(block);
        self.functions.insert(
            id.to_string(),
            FunctionNode {
                module: module.to_string(),
                imports: imports.clone(),
                facts: visitor.facts,
            },
        );
    }

    fn insert_method_target(&mut self, target: MethodTarget, method_name: &str) {
        let owner_key = format!("{}::{method_name}", target.owner.join("::"));
        self.associated_methods
            .entry(owner_key)
            .or_default()
            .insert(target.id.clone());
        if let Some(trait_path) = &target.trait_path {
            let trait_key = format!("{}::{method_name}", trait_path.join("::"));
            self.associated_methods
                .entry(trait_key)
                .or_default()
                .insert(target.id.clone());
        }
        self.methods_by_name
            .entry(method_name.to_string())
            .or_default()
            .push(target);
    }

    fn violations_from(&self, roots: &[&str]) -> Vec<String> {
        let mut queue = roots
            .iter()
            .map(|root| (*root).to_string())
            .collect::<VecDeque<_>>();
        let mut visited = BTreeSet::new();
        let mut violations = Vec::new();
        while let Some(function_id) = queue.pop_front() {
            if !visited.insert(function_id.clone()) {
                continue;
            }
            let Some(function) = self.functions.get(&function_id) else {
                violations.push(format!("missing architecture root {function_id}"));
                continue;
            };
            for reason in &function.facts.forbidden {
                violations.push(format!("{function_id}: {reason}"));
            }
            for call in &function.facts.calls {
                match call {
                    CallFact::Path(path) => {
                        if path.first().is_some_and(|alias| {
                            function.imports.get(alias).is_some_and(|imported| {
                                imported.iter().any(|segment| {
                                    matches!(segment.as_str(), "roxmltree" | "quick_xml")
                                })
                            })
                        }) {
                            violations.push(format!(
                                "{function_id}: calls imported parser path {}",
                                path.join("::")
                            ));
                            continue;
                        }
                        queue.extend(self.resolve_path(function, path));
                    }
                    CallFact::Method(method) => {
                        if let Some(targets) = self.methods_by_name.get(method) {
                            queue.extend(targets.iter().map(|target| target.id.clone()));
                        }
                    }
                    CallFact::Ufcs {
                        self_type,
                        trait_path,
                        method,
                    } => {
                        let owner = resolve_decl_path(
                            &function.module,
                            &function.imports,
                            self_type.clone(),
                        );
                        let trait_path = resolve_decl_path(
                            &function.module,
                            &function.imports,
                            trait_path.clone(),
                        );
                        if let Some(targets) = self.methods_by_name.get(method) {
                            queue.extend(
                                targets
                                    .iter()
                                    .filter(|target| {
                                        target.owner == owner
                                            && target.trait_path.as_ref() == Some(&trait_path)
                                    })
                                    .map(|target| target.id.clone()),
                            );
                        }
                    }
                }
            }
        }
        violations
    }

    fn resolve_path(&self, caller: &FunctionNode, call: &[String]) -> Vec<String> {
        let candidate = match call {
            [name] => {
                if let Some(imported) = caller.imports.get(name) {
                    imported.join("::")
                } else {
                    format!("{}::{name}", caller.module)
                }
            }
            [first, ..] if matches!(first.as_str(), "crate" | "self" | "super") => {
                absolutize_path(&caller.module, call.to_vec()).join("::")
            }
            [first, rest @ ..] if caller.imports.contains_key(first) => {
                let mut path = caller.imports[first].clone();
                path.extend(rest.iter().cloned());
                path.join("::")
            }
            _ => format!("{}::{}", caller.module, call.join("::")),
        };
        let mut targets = Vec::new();
        if self.functions.contains_key(&candidate) {
            targets.push(candidate.clone());
        }
        if let Some(methods) = self.associated_methods.get(&candidate) {
            targets.extend(methods.iter().cloned());
        }
        targets
    }
}

#[derive(Default)]
struct FactVisitor {
    facts: FunctionFacts,
}

impl<'ast> Visit<'ast> for FactVisitor {
    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if let Expr::Path(path) = node.func.as_ref() {
            if let Some(qself) = &path.qself {
                let segments = path_segments(&path.path);
                if let (Some(self_type), Some(method)) =
                    (type_path_segments(&qself.ty), segments.last().cloned())
                {
                    self.facts.calls.push(CallFact::Ufcs {
                        self_type,
                        trait_path: segments[..segments.len().saturating_sub(1)].to_vec(),
                        method,
                    });
                }
            } else {
                self.facts
                    .calls
                    .push(CallFact::Path(path_segments(&path.path)));
            }
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast ExprPath) {
        let path = path_segments(&node.path);
        if node.qself.is_none() {
            self.facts.calls.push(CallFact::Path(path.clone()));
        }
        if path
            .iter()
            .any(|segment| matches!(segment.as_str(), "roxmltree" | "quick_xml"))
        {
            self.facts
                .forbidden
                .push(format!("imports or calls parser path {}", path.join("::")));
        }
        visit::visit_expr_path(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        self.facts
            .calls
            .push(CallFact::Method(node.method.to_string()));
        if matches!(
            node.method.to_string().as_str(),
            "tag_name" | "namespace" | "attribute"
        ) {
            self.facts
                .forbidden
                .push(format!("calls native parser method {}", node.method));
        }
        if node.method == "join" {
            if let Some(syn::Expr::Lit(literal)) = node.args.first() {
                if let Lit::Str(value) = &literal.lit {
                    if is_native_literal(&value.value()) {
                        self.facts
                            .forbidden
                            .push(format!("joins native layout artifact {}", value.value()));
                    }
                }
            }
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_lit(&mut self, literal: &'ast Lit) {
        if let Lit::Str(value) = literal {
            if is_native_literal(&value.value()) {
                self.facts
                    .forbidden
                    .push(format!("matches native wire value {}", value.value()));
            }
        }
        visit::visit_lit(self, literal);
    }
}

fn type_path_segments(ty: &Type) -> Option<Vec<String>> {
    let Type::Path(path) = ty else {
        return None;
    };
    Some(path_segments(&path.path))
}

fn resolve_decl_path(
    module: &str,
    imports: &BTreeMap<String, Vec<String>>,
    path: Vec<String>,
) -> Vec<String> {
    match path.as_slice() {
        [first, ..] if matches!(first.as_str(), "crate" | "self" | "super") => {
            absolutize_path(module, path)
        }
        [first, rest @ ..] if imports.contains_key(first) => {
            let mut resolved = imports[first].clone();
            resolved.extend(rest.iter().cloned());
            resolved
        }
        _ => {
            let mut resolved = module.split("::").map(str::to_string).collect::<Vec<_>>();
            resolved.extend(path);
            resolved
        }
    }
}

fn is_native_literal(value: &str) -> bool {
    matches!(
        value,
        "Configuration.xml"
            | "ParentConfigurations.bin"
            | "MetaDataObject"
            | "MDClasses"
            | "Catalogs"
            | "Documents"
            | "Reports"
            | "Roles"
            | "Rights.xml"
            | "Form.xml"
            | "Template.xml"
    ) || value.contains("v8.1c.ru/8.3/MDClasses")
}

fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn module_imports(module: &str, items: &[Item]) -> BTreeMap<String, Vec<String>> {
    let mut imports = BTreeMap::new();
    for item in items {
        if let Item::Use(item_use) = item {
            collect_use(module, item_use, &mut imports);
        }
    }
    imports
}

fn collect_use(module: &str, item_use: &ItemUse, imports: &mut BTreeMap<String, Vec<String>>) {
    fn walk(prefix: Vec<String>, tree: &UseTree, output: &mut Vec<(String, Vec<String>)>) {
        match tree {
            UseTree::Path(path) => {
                let mut prefix = prefix;
                prefix.push(path.ident.to_string());
                walk(prefix, &path.tree, output);
            }
            UseTree::Name(name) => {
                let mut path = prefix;
                path.push(name.ident.to_string());
                output.push((name.ident.to_string(), path));
            }
            UseTree::Rename(rename) => {
                let mut path = prefix;
                path.push(rename.ident.to_string());
                output.push((rename.rename.to_string(), path));
            }
            UseTree::Group(group) => {
                for item in &group.items {
                    walk(prefix.clone(), item, output);
                }
            }
            UseTree::Glob(_) => {}
        }
    }

    let mut flattened = Vec::new();
    walk(Vec::new(), &item_use.tree, &mut flattened);
    for (alias, mut path) in flattened {
        path = absolutize_use(module, path);
        imports.insert(alias, path);
    }
}

fn absolutize_use(module: &str, mut path: Vec<String>) -> Vec<String> {
    absolutize_path(module, std::mem::take(&mut path))
}

fn absolutize_path(module: &str, mut path: Vec<String>) -> Vec<String> {
    if path.first().is_some_and(|part| part == "crate") {
        return path;
    }
    let mut absolute = module.split("::").map(str::to_string).collect::<Vec<_>>();
    if path.first().is_some_and(|part| part == "self") {
        path.remove(0);
    }
    while path.first().is_some_and(|part| part == "super") {
        path.remove(0);
        absolute.pop();
    }
    absolute.extend(path);
    absolute
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn rust_sources(root: &Path) -> Vec<(String, String)> {
    fn collect(root: &Path, directory: &Path, output: &mut Vec<(String, String)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect(root, &path, output);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                let relative = path.strip_prefix(root).unwrap();
                let mut parts = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().to_string())
                    .collect::<Vec<_>>();
                let file = parts.pop().unwrap();
                if file != "mod.rs" {
                    parts.push(file.trim_end_matches(".rs").to_string());
                }
                let module = std::iter::once("crate".to_string())
                    .chain(parts)
                    .collect::<Vec<_>>()
                    .join("::");
                output.push((module, fs::read_to_string(path).unwrap()));
            }
        }
    }

    let mut output = Vec::new();
    collect(root, root, &mut output);
    output
}

fn fixture_graph(label: &str, files: &[(&str, &str)]) -> CallGraph {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "unica-task7-call-graph-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    for (relative, source) in files {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, source).unwrap();
    }
    let graph = CallGraph::from_sources(rust_sources(&root));
    fs::remove_dir_all(root).unwrap();
    graph
}

#[test]
fn task7_host_entrypoints_cannot_reach_native_read_or_layout_logic() {
    let host = repo_root().join("crates/unica-coder/src");
    let graph = CallGraph::from_sources(rust_sources(&host));
    let violations = graph.violations_from(&[
        "crate::infrastructure::format_guard::evaluate_format_guard",
        "crate::infrastructure::support_guard::evaluate_support_guard",
        "crate::infrastructure::native_operations::meta::validate_meta",
    ]);
    assert!(
        violations.is_empty(),
        "Task 7 host reachability violation(s):\n{}",
        violations.join("\n")
    );
}

#[test]
fn renamed_cross_file_parser_helper_is_still_rejected() {
    let graph = fixture_graph(
        "renamed-parser",
        &[
            (
                "entry.rs",
                "use crate::bridge::renamed; pub fn run() { renamed(); }",
            ),
            (
                "bridge.rs",
                "pub fn renamed() { crate::deep::parse_anything(); }",
            ),
            (
                "deep.rs",
                "pub fn parse_anything() { let _ = roxmltree::Document::parse(\"<x/>\"); }",
            ),
        ],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("roxmltree")),
        "{violations:?}"
    );
}

#[test]
fn moved_cross_file_native_layout_helper_is_still_rejected() {
    let graph = fixture_graph("moved-layout", &[
        (
            "entry.rs",
            "pub fn run() { crate::middle::forward(); }",
        ),
        (
            "middle.rs",
            "pub fn forward() { crate::moved::locate(); }",
        ),
        (
            "moved.rs",
            "pub fn locate() { let _ = std::path::Path::new(\".\").join(\"Configuration.xml\"); }",
        ),
    ]);

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("Configuration.xml")),
        "{violations:?}"
    );
}

#[test]
fn renamed_inherent_method_with_method_call_syntax_is_rejected() {
    let graph = fixture_graph(
        "inherent-method",
        &[(
            "entry.rs",
            r#"
                pub struct Boundary;
                impl Boundary {
                    fn inspect_hidden(&self) {
                        let _ = std::path::Path::new(".").join("Configuration.xml");
                    }
                }
                pub fn run() {
                    let boundary = Boundary;
                    boundary.inspect_hidden();
                }
            "#,
        )],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("Configuration.xml")),
        "{violations:?}"
    );
}

#[test]
fn renamed_trait_impl_method_and_ufcs_call_are_rejected() {
    let graph = fixture_graph(
        "trait-ufcs",
        &[(
            "entry.rs",
            r#"
                trait BoundaryCheck { fn inspect_hidden(&self); }
                struct Boundary;
                impl BoundaryCheck for Boundary {
                    fn inspect_hidden(&self) {
                        let _ = roxmltree::Document::parse("<x/>");
                    }
                }
                pub fn run() {
                    <Boundary as BoundaryCheck>::inspect_hidden(&Boundary);
                }
            "#,
        )],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("roxmltree")),
        "{violations:?}"
    );
}

#[test]
fn nested_multi_layer_trait_method_logic_is_rejected() {
    let graph = fixture_graph(
        "nested-method-layers",
        &[(
            "entry.rs",
            r#"
                mod nested {
                    pub trait BoundaryCheck { fn inspect_hidden(&self); }
                    pub struct Boundary;
                    impl BoundaryCheck for Boundary {
                        fn inspect_hidden(&self) { first(self); }
                    }
                    fn first(boundary: &Boundary) { second(boundary); }
                    fn second(_boundary: &Boundary) {
                        let _ = std::path::Path::new(".").join("ParentConfigurations.bin");
                    }
                }
                pub fn run() {
                    use nested::BoundaryCheck;
                    nested::Boundary.inspect_hidden();
                }
            "#,
        )],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("ParentConfigurations.bin")),
        "{violations:?}"
    );
}

#[test]
fn unknown_neutral_trait_object_method_terminates_without_false_positive() {
    let graph = fixture_graph(
        "neutral-trait-object",
        &[(
            "entry.rs",
            r#"
                pub trait NeutralPort { fn inspect(&self); }
                pub fn run(port: &dyn NeutralPort) { port.inspect(); }
            "#,
        )],
    );

    assert!(
        graph.violations_from(&["crate::entry::run"]).is_empty(),
        "neutral trait-object calls must terminate at the port boundary"
    );
}

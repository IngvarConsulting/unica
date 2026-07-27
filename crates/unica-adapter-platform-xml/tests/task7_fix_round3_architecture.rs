use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use syn::{
    visit::{self, Visit},
    Expr, ExprCall, ExprMethodCall, ExprPath, FnArg, ImplItem, Item, ItemUse, Lit, Pat, Signature,
    Stmt, TraitItem, Type, TypeParamBound, UseTree,
};

#[derive(Debug, Clone, Default)]
struct Imports {
    aliases: BTreeMap<String, Vec<String>>,
    globs: Vec<Vec<String>>,
}

impl Imports {
    fn overlay(&mut self, other: &Self) {
        self.aliases.extend(other.aliases.clone());
        self.globs.extend(other.globs.clone());
    }
}

#[derive(Debug)]
enum CallFact {
    Path {
        path: Vec<String>,
        imports: Imports,
    },
    Method {
        name: String,
        approved_neutral_port: bool,
    },
    Ufcs {
        self_type: Vec<String>,
        trait_path: Vec<String>,
        method: String,
        imports: Imports,
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
    owner: Option<Vec<String>>,
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
    local_types: BTreeSet<String>,
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
                    self.insert_body(&id, module, &imports, None, &function.sig, &function.block);
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
                        self.insert_body(
                            &id,
                            module,
                            &imports,
                            Some(owner.clone()),
                            &method.sig,
                            &method.block,
                        );
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
                    self.local_types.insert(trait_path.join("::"));
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
                        self.insert_body(
                            &id,
                            module,
                            &imports,
                            Some(trait_path.clone()),
                            &method.sig,
                            default,
                        );
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
                    if item_mod.attrs.iter().any(is_cfg_test) {
                        continue;
                    }
                    if let Some((_, nested)) = &item_mod.content {
                        self.insert_items(&format!("{module}::{}", item_mod.ident), nested);
                    }
                }
                Item::Struct(item_struct) => {
                    self.local_types.insert(
                        resolve_decl_path(module, &imports, vec![item_struct.ident.to_string()])
                            .join("::"),
                    );
                }
                Item::Enum(item_enum) => {
                    self.local_types.insert(
                        resolve_decl_path(module, &imports, vec![item_enum.ident.to_string()])
                            .join("::"),
                    );
                }
                Item::Union(item_union) => {
                    self.local_types.insert(
                        resolve_decl_path(module, &imports, vec![item_union.ident.to_string()])
                            .join("::"),
                    );
                }
                Item::Type(item_type) => {
                    self.local_types.insert(
                        resolve_decl_path(module, &imports, vec![item_type.ident.to_string()])
                            .join("::"),
                    );
                }
                _ => {}
            }
        }
    }

    fn insert_body(
        &mut self,
        id: &str,
        module: &str,
        imports: &Imports,
        owner: Option<Vec<String>>,
        signature: &Signature,
        block: &syn::Block,
    ) {
        let mut visitor =
            FactVisitor::new(module, imports.clone(), neutral_port_receivers(signature));
        visitor.visit_block(block);
        self.functions.insert(
            id.to_string(),
            FunctionNode {
                module: module.to_string(),
                owner,
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
                    CallFact::Path { path, imports } => {
                        let resolution = self.resolve_path(function, path, imports);
                        if resolution.candidates.iter().any(|candidate| {
                            candidate
                                .split("::")
                                .any(|segment| matches!(segment, "roxmltree" | "quick_xml"))
                        }) {
                            violations.push(format!(
                                "{function_id}: calls imported parser path {}",
                                path.join("::")
                            ));
                            continue;
                        }
                        if resolution.targets.is_empty() && resolution.unresolved_local {
                            violations.push(format!(
                                "{function_id}: unresolved local call {}",
                                path.join("::")
                            ));
                        }
                        queue.extend(resolution.targets);
                    }
                    CallFact::Method {
                        name,
                        approved_neutral_port,
                    } => {
                        if *approved_neutral_port {
                            continue;
                        }
                        if let Some(targets) = self.methods_by_name.get(name) {
                            queue.extend(targets.iter().map(|target| target.id.clone()));
                        }
                    }
                    CallFact::Ufcs {
                        self_type,
                        trait_path,
                        method,
                        imports,
                    } => {
                        let owner = resolve_call_path(
                            &function.module,
                            imports,
                            function.owner.as_ref(),
                            self_type.clone(),
                        );
                        let trait_path = resolve_call_path(
                            &function.module,
                            imports,
                            function.owner.as_ref(),
                            trait_path.clone(),
                        );
                        let mut matched = false;
                        if let Some(targets) = self.methods_by_name.get(method) {
                            for target in targets.iter().filter(|target| {
                                target.trait_path.as_ref() == Some(&trait_path)
                                    && (target.owner == owner || target.owner == trait_path)
                            }) {
                                matched = true;
                                queue.push_back(target.id.clone());
                            }
                        }
                        if !matched && (is_local_path(&owner) || is_local_path(&trait_path)) {
                            violations.push(format!(
                                "{function_id}: unresolved local UFCS call <{} as {}>::{method}",
                                owner.join("::"),
                                trait_path.join("::")
                            ));
                        }
                    }
                }
            }
        }
        violations
    }

    fn resolve_path(
        &self,
        caller: &FunctionNode,
        call: &[String],
        imports: &Imports,
    ) -> PathResolution {
        let mut candidates = Vec::new();
        let mut unresolved_local = match call {
            [first, rest @ ..] if first == "Self" => {
                if let Some(owner) = &caller.owner {
                    let mut path = owner.clone();
                    path.extend(rest.iter().cloned());
                    candidates.push(path);
                }
                true
            }
            [name] => {
                if let Some(imported) = imports.aliases.get(name) {
                    candidates.push(imported.clone());
                    is_local_path(imported)
                } else {
                    for glob in &imports.globs {
                        let mut path = glob.clone();
                        path.push(name.clone());
                        candidates.push(path);
                    }
                    let mut local = module_segments(&caller.module);
                    local.push(name.clone());
                    candidates.push(local);
                    !is_prelude_function(name)
                }
            }
            [first, ..] if matches!(first.as_str(), "crate" | "self" | "super") => {
                candidates.push(absolutize_path(&caller.module, call.to_vec()));
                true
            }
            [first, rest @ ..] if imports.aliases.contains_key(first) => {
                let mut path = imports.aliases[first].clone();
                path.extend(rest.iter().cloned());
                let local = is_local_path(&path);
                candidates.push(path);
                local
            }
            [first, ..] => {
                let mut path = module_segments(&caller.module);
                path.extend(call.iter().cloned());
                candidates.push(path);
                first
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_lowercase())
                    && !is_external_crate_root(first)
                    && !is_primitive_type(first)
            }
            [] => false,
        };
        if call.last().is_some_and(|segment| {
            segment
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_uppercase())
        }) {
            unresolved_local = false;
        }
        let mut targets = Vec::new();
        let mut candidate_labels = Vec::new();
        for candidate in candidates {
            let candidate = candidate.join("::");
            candidate_labels.push(candidate.clone());
            if self.functions.contains_key(&candidate) {
                targets.push(candidate.clone());
            }
            if let Some(methods) = self.associated_methods.get(&candidate) {
                targets.extend(methods.iter().cloned());
            }
        }
        if unresolved_local
            && call.len() > 1
            && call
                .first()
                .and_then(|segment| segment.chars().next())
                .is_some_and(|character| character.is_ascii_uppercase())
        {
            let has_local_owner = candidate_labels.iter().any(|candidate| {
                candidate
                    .rsplit_once("::")
                    .is_some_and(|(owner, _)| self.local_types.contains(owner))
            });
            if !has_local_owner {
                unresolved_local = false;
            }
        }
        targets.sort();
        targets.dedup();
        PathResolution {
            targets,
            candidates: candidate_labels,
            unresolved_local,
        }
    }
}

#[derive(Debug)]
struct PathResolution {
    targets: Vec<String>,
    candidates: Vec<String>,
    unresolved_local: bool,
}

struct FactVisitor {
    module: String,
    module_imports: Imports,
    scopes: Vec<Imports>,
    neutral_receivers: BTreeSet<String>,
    facts: FunctionFacts,
}

impl FactVisitor {
    fn new(module: &str, module_imports: Imports, neutral_receivers: BTreeSet<String>) -> Self {
        Self {
            module: module.to_string(),
            module_imports,
            scopes: Vec::new(),
            neutral_receivers,
            facts: FunctionFacts::default(),
        }
    }

    fn effective_imports(&self) -> Imports {
        let mut imports = self.module_imports.clone();
        for scope in &self.scopes {
            imports.overlay(scope);
        }
        imports
    }
}

impl<'ast> Visit<'ast> for FactVisitor {
    fn visit_block(&mut self, node: &'ast syn::Block) {
        let mut scope = Imports::default();
        for statement in &node.stmts {
            if let Stmt::Item(Item::Use(item_use)) = statement {
                collect_use(&self.module, item_use, &mut scope);
            }
        }
        self.scopes.push(scope);
        visit::visit_block(self, node);
        self.scopes.pop();
    }

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
                        imports: self.effective_imports(),
                    });
                }
            } else {
                self.facts.calls.push(CallFact::Path {
                    path: path_segments(&path.path),
                    imports: self.effective_imports(),
                });
            }
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast ExprPath) {
        let path = path_segments(&node.path);
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
        let approved_neutral_port = receiver_ident(node.receiver.as_ref())
            .is_some_and(|receiver| self.neutral_receivers.contains(&receiver));
        self.facts.calls.push(CallFact::Method {
            name: node.method.to_string(),
            approved_neutral_port,
        });
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

fn resolve_decl_path(module: &str, imports: &Imports, path: Vec<String>) -> Vec<String> {
    match path.as_slice() {
        [first, ..] if matches!(first.as_str(), "crate" | "self" | "super") => {
            absolutize_path(module, path)
        }
        [first, rest @ ..] if imports.aliases.contains_key(first) => {
            let mut resolved = imports.aliases[first].clone();
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

fn resolve_call_path(
    module: &str,
    imports: &Imports,
    owner: Option<&Vec<String>>,
    path: Vec<String>,
) -> Vec<String> {
    if path.first().is_some_and(|first| first == "Self") {
        if let Some(owner) = owner {
            let mut resolved = owner.clone();
            resolved.extend(path.into_iter().skip(1));
            return resolved;
        }
    }
    resolve_decl_path(module, imports, path)
}

fn receiver_ident(receiver: &Expr) -> Option<String> {
    let Expr::Path(path) = receiver else {
        return None;
    };
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return None;
    }
    path.path
        .segments
        .first()
        .map(|segment| segment.ident.to_string())
}

fn neutral_port_receivers(signature: &Signature) -> BTreeSet<String> {
    signature
        .inputs
        .iter()
        .filter_map(|input| {
            let FnArg::Typed(argument) = input else {
                return None;
            };
            let Pat::Ident(pattern) = argument.pat.as_ref() else {
                return None;
            };
            is_explicit_neutral_port_trait_object(&argument.ty).then(|| pattern.ident.to_string())
        })
        .collect()
}

fn is_explicit_neutral_port_trait_object(ty: &Type) -> bool {
    let ty = match ty {
        Type::Reference(reference) => reference.elem.as_ref(),
        Type::Paren(paren) => paren.elem.as_ref(),
        Type::Group(group) => group.elem.as_ref(),
        other => other,
    };
    let Type::TraitObject(trait_object) = ty else {
        return false;
    };
    trait_object.bounds.iter().any(|bound| {
        let TypeParamBound::Trait(bound) = bound else {
            return false;
        };
        bound
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident.to_string().ends_with("Port"))
    })
}

fn module_segments(module: &str) -> Vec<String> {
    module.split("::").map(str::to_string).collect()
}

fn is_local_path(path: &[String]) -> bool {
    path.first().is_some_and(|segment| segment == "crate")
}

fn is_prelude_function(name: &str) -> bool {
    matches!(name, "drop" | "Some" | "Ok" | "Err")
}

fn is_external_crate_root(name: &str) -> bool {
    matches!(
        name,
        "std"
            | "core"
            | "alloc"
            | "serde"
            | "serde_json"
            | "syn"
            | "roxmltree"
            | "quick_xml"
            | "unica_format_core"
            | "unica_application"
            | "unica_adapter_platform_xml"
    )
}

fn is_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "str"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
    )
}

fn is_cfg_test(attribute: &syn::Attribute) -> bool {
    attribute.path().is_ident("cfg")
        && attribute.meta.require_list().is_ok_and(|list| {
            list.tokens
                .to_string()
                .split_whitespace()
                .any(|part| part == "test")
        })
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

fn module_imports(module: &str, items: &[Item]) -> Imports {
    let mut imports = Imports::default();
    for item in items {
        if let Item::Use(item_use) = item {
            collect_use(module, item_use, &mut imports);
        }
    }
    imports
}

fn collect_use(module: &str, item_use: &ItemUse, imports: &mut Imports) {
    fn walk(module: &str, prefix: Vec<String>, tree: &UseTree, imports: &mut Imports) {
        match tree {
            UseTree::Path(path) => {
                let mut prefix = prefix;
                prefix.push(path.ident.to_string());
                walk(module, prefix, &path.tree, imports);
            }
            UseTree::Name(name) => {
                if name.ident == "self" {
                    if let Some(alias) = prefix.last() {
                        imports
                            .aliases
                            .insert(alias.to_string(), absolutize_use(module, prefix));
                    }
                } else {
                    let mut path = prefix;
                    path.push(name.ident.to_string());
                    imports
                        .aliases
                        .insert(name.ident.to_string(), absolutize_use(module, path));
                }
            }
            UseTree::Rename(rename) => {
                let mut path = prefix;
                path.push(rename.ident.to_string());
                imports
                    .aliases
                    .insert(rename.rename.to_string(), absolutize_use(module, path));
            }
            UseTree::Group(group) => {
                for item in &group.items {
                    walk(module, prefix.clone(), item, imports);
                }
            }
            UseTree::Glob(_) => imports.globs.push(absolutize_use(module, prefix)),
        }
    }

    walk(module, Vec::new(), &item_use.tree, imports);
}

fn absolutize_use(module: &str, path: Vec<String>) -> Vec<String> {
    if path
        .first()
        .is_some_and(|first| matches!(first.as_str(), "crate" | "self" | "super"))
    {
        absolutize_path(module, path)
    } else {
        path
    }
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

#[test]
fn self_associated_helper_cannot_hide_native_logic() {
    let graph = fixture_graph(
        "self-helper",
        &[(
            "entry.rs",
            r#"
                struct Boundary;
                impl Boundary {
                    fn run() { Self::helper(); }
                    fn helper() {
                        let _ = std::path::Path::new(".").join("Configuration.xml");
                    }
                }
                pub fn run() { Boundary::run(); }
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
fn block_local_alias_import_cannot_hide_native_logic() {
    let graph = fixture_graph(
        "local-alias",
        &[(
            "entry.rs",
            r#"
                mod hidden {
                    pub fn inspect() {
                        let _ = roxmltree::Document::parse("<x/>");
                    }
                }
                pub fn run() {
                    use crate::entry::hidden::inspect as execute;
                    execute();
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
fn block_local_glob_import_cannot_hide_native_logic() {
    let graph = fixture_graph(
        "local-glob",
        &[(
            "entry.rs",
            r#"
                mod hidden {
                    pub fn inspect() {
                        let _ = std::path::Path::new(".").join("ParentConfigurations.bin");
                    }
                }
                pub fn run() {
                    use crate::entry::hidden::*;
                    inspect();
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
fn trait_default_method_dispatch_cannot_hide_native_logic() {
    let graph = fixture_graph(
        "trait-default",
        &[(
            "entry.rs",
            r#"
                trait Boundary {
                    fn inspect() { Self::helper(); }
                    fn helper() {
                        let _ = std::path::Path::new(".").join("Rights.xml");
                    }
                }
                struct Concrete;
                impl Boundary for Concrete {}
                pub fn run() { <Concrete as Boundary>::inspect(); }
            "#,
        )],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("Rights.xml")),
        "{violations:?}"
    );
}

#[test]
fn unknown_receiver_connects_to_renamed_native_method() {
    let graph = fixture_graph(
        "unknown-receiver",
        &[(
            "entry.rs",
            r#"
                trait HiddenBoundary {
                    fn inspect_renamed(&self) {
                        let _ = roxmltree::Document::parse("<x/>");
                    }
                }
                struct Unknown;
                pub fn run(value: Unknown) { value.inspect_renamed(); }
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
fn nested_async_alias_chain_cannot_hide_native_logic() {
    let graph = fixture_graph(
        "nested-async-chain",
        &[(
            "entry.rs",
            r#"
                mod hidden {
                    pub fn first() { second(); }
                    fn second() { third(); }
                    fn third() {
                        let _ = std::path::Path::new(".").join("Form.xml");
                    }
                }
                pub fn run() {
                    async {
                        use crate::entry::hidden::first as begin;
                        let invoke = || begin();
                        invoke();
                    };
                }
            "#,
        )],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("Form.xml")),
        "{violations:?}"
    );
}

#[test]
fn explicit_neutral_port_trait_object_ignores_same_named_local_methods() {
    let graph = fixture_graph(
        "typed-neutral-trait-object",
        &[(
            "entry.rs",
            r#"
                pub trait ValidationPort { fn inspect(&self); }
                trait HiddenBoundary {
                    fn inspect(&self) {
                        let _ = std::path::Path::new(".").join("Template.xml");
                    }
                }
                pub fn run(port: &dyn ValidationPort) { port.inspect(); }
            "#,
        )],
    );

    assert!(
        graph.violations_from(&["crate::entry::run"]).is_empty(),
        "statically typed neutral port call must terminate at the port boundary"
    );
}

#[test]
fn unresolved_local_call_fails_closed() {
    let graph = fixture_graph(
        "unresolved-local",
        &[("entry.rs", "pub fn run() { missing_local_helper(); }")],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("unresolved local call missing_local_helper")),
        "{violations:?}"
    );
}

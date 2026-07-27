use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use quote::ToTokens;
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
        fail_if_unresolved: bool,
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
                    let owner = canonical_type_owner(module, &imports, None, &item_impl.self_ty);
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
        let mut visitor = FactVisitor::new(
            module,
            imports.clone(),
            owner.clone(),
            neutral_port_receivers(module, imports, signature),
            receiver_parameters(module, imports, signature),
        );
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
                        fail_if_unresolved,
                    } => {
                        if *approved_neutral_port {
                            continue;
                        }
                        if let Some(targets) = self.methods_by_name.get(name) {
                            queue.extend(targets.iter().map(|target| target.id.clone()));
                        } else if *fail_if_unresolved {
                            violations
                                .push(format!("{function_id}: unresolved receiver method {name}"));
                        }
                    }
                    CallFact::Ufcs {
                        self_type,
                        trait_path,
                        method,
                        imports,
                    } => {
                        let owner = self_type.clone();
                        let trait_path = resolve_call_path(
                            &function.module,
                            imports,
                            function.owner.as_ref(),
                            trait_path.clone(),
                        );
                        let mut matched = false;
                        if let Some(targets) = self.methods_by_name.get(method) {
                            for target in targets
                                .iter()
                                .filter(|target| target.trait_path.as_ref() == Some(&trait_path))
                            {
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
    owner: Option<Vec<String>>,
    neutral_receivers: BTreeMap<String, BTreeSet<String>>,
    receiver_scopes: Vec<BTreeMap<String, ReceiverProvenance>>,
    facts: FunctionFacts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiverProvenance {
    Local,
    External,
    Unknown,
}

impl FactVisitor {
    fn new(
        module: &str,
        module_imports: Imports,
        owner: Option<Vec<String>>,
        neutral_receivers: BTreeMap<String, BTreeSet<String>>,
        receiver_parameters: BTreeMap<String, ReceiverProvenance>,
    ) -> Self {
        Self {
            module: module.to_string(),
            module_imports,
            scopes: Vec::new(),
            owner,
            neutral_receivers,
            receiver_scopes: vec![receiver_parameters],
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

    fn receiver_provenance(&self, name: &str) -> ReceiverProvenance {
        self.receiver_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .unwrap_or(ReceiverProvenance::Unknown)
    }

    fn expression_provenance(&self, expression: &Expr) -> ReceiverProvenance {
        match expression {
            Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
                let name = path.path.segments[0].ident.to_string();
                let known = self.receiver_provenance(&name);
                if known != ReceiverProvenance::Unknown {
                    known
                } else {
                    path_provenance(&self.module, &self.effective_imports(), &path.path)
                }
            }
            Expr::Path(path) if path.qself.is_none() => {
                path_provenance(&self.module, &self.effective_imports(), &path.path)
            }
            Expr::Struct(expression) => {
                path_provenance(&self.module, &self.effective_imports(), &expression.path)
            }
            Expr::Reference(reference) => self.expression_provenance(&reference.expr),
            Expr::Paren(paren) => self.expression_provenance(&paren.expr),
            Expr::Group(group) => self.expression_provenance(&group.expr),
            Expr::Array(_) | Expr::Tuple(_) | Expr::Lit(_) => ReceiverProvenance::External,
            Expr::Call(call) => {
                let Expr::Path(path) = call.func.as_ref() else {
                    return ReceiverProvenance::Unknown;
                };
                if path.path.segments.len() != 1 {
                    return ReceiverProvenance::Unknown;
                }
                let first = path.path.segments.first().map(|segment| {
                    segment
                        .ident
                        .to_string()
                        .chars()
                        .next()
                        .is_some_and(|character| character.is_ascii_uppercase())
                });
                if first == Some(true) {
                    path_provenance(&self.module, &self.effective_imports(), &path.path)
                } else {
                    ReceiverProvenance::Unknown
                }
            }
            Expr::MethodCall(_) => ReceiverProvenance::Unknown,
            _ => ReceiverProvenance::Unknown,
        }
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
        self.receiver_scopes.push(BTreeMap::new());
        visit::visit_block(self, node);
        self.receiver_scopes.pop();
        self.scopes.pop();
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        visit::visit_local(self, node);
        let (pattern, provenance) = match &node.pat {
            Pat::Type(pattern) => (
                pattern.pat.as_ref(),
                type_provenance(&self.module, &self.effective_imports(), pattern.ty.as_ref()),
            ),
            pattern => (
                pattern,
                node.init
                    .as_ref()
                    .map(|init| self.expression_provenance(&init.expr))
                    .unwrap_or(ReceiverProvenance::Unknown),
            ),
        };
        if let Pat::Ident(pattern) = pattern {
            if let Some(scope) = self.receiver_scopes.last_mut() {
                scope.insert(pattern.ident.to_string(), provenance);
            }
        }
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if let Expr::Path(path) = node.func.as_ref() {
            if let Some(qself) = &path.qself {
                let segments = path_segments(&path.path);
                if let Some(method) = segments.last().cloned() {
                    let imports = self.effective_imports();
                    self.facts.calls.push(CallFact::Ufcs {
                        self_type: canonical_type_owner(
                            &self.module,
                            &imports,
                            self.owner.as_ref(),
                            &qself.ty,
                        ),
                        trait_path: segments[..segments.len().saturating_sub(1)].to_vec(),
                        method,
                        imports,
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
        let method = node.method.to_string();
        let receiver = receiver_ident(node.receiver.as_ref());
        let approved_neutral_port = receiver
            .as_ref()
            .and_then(|receiver| self.neutral_receivers.get(receiver))
            .is_some_and(|methods| methods.contains(&method));
        let fail_if_unresolved = receiver
            .as_deref()
            .map(|receiver| self.receiver_provenance(receiver))
            .unwrap_or_else(|| self.expression_provenance(node.receiver.as_ref()))
            == ReceiverProvenance::Local;
        self.facts.calls.push(CallFact::Method {
            name: method,
            approved_neutral_port,
            fail_if_unresolved,
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

fn canonical_type_owner(
    module: &str,
    imports: &Imports,
    owner: Option<&Vec<String>>,
    ty: &Type,
) -> Vec<String> {
    vec![canonical_type(module, imports, owner, ty)]
}

fn canonical_type(
    module: &str,
    imports: &Imports,
    owner: Option<&Vec<String>>,
    ty: &Type,
) -> String {
    match ty {
        Type::Path(path) if path.qself.is_none() => {
            let plain = path_segments(&path.path);
            let mut resolved = if plain.first().is_some_and(|segment| segment == "Self") {
                owner.cloned().unwrap_or_else(|| {
                    let mut unresolved = module_segments(module);
                    unresolved.push("Self".to_string());
                    unresolved
                })
            } else {
                resolve_decl_path(module, imports, plain)
            };
            let offset = resolved.len().saturating_sub(path.path.segments.len());
            for (index, segment) in path.path.segments.iter().enumerate() {
                let arguments = compact_tokens(&segment.arguments);
                if !arguments.is_empty() {
                    if let Some(resolved_segment) = resolved.get_mut(offset + index) {
                        resolved_segment.push_str(&arguments);
                    }
                }
            }
            resolved.join("::")
        }
        Type::Reference(reference) => format!(
            "&{}{}{}",
            reference
                .lifetime
                .as_ref()
                .map(|lifetime| format!("{} ", lifetime.ident))
                .unwrap_or_default(),
            if reference.mutability.is_some() {
                "mut "
            } else {
                ""
            },
            canonical_type(module, imports, owner, &reference.elem)
        ),
        Type::Ptr(pointer) => format!(
            "*{} {}",
            if pointer.mutability.is_some() {
                "mut"
            } else {
                "const"
            },
            canonical_type(module, imports, owner, &pointer.elem)
        ),
        Type::Slice(slice) => {
            format!("[{}]", canonical_type(module, imports, owner, &slice.elem))
        }
        Type::Array(array) => format!(
            "[{};{}]",
            canonical_type(module, imports, owner, &array.elem),
            compact_tokens(&array.len)
        ),
        Type::Tuple(tuple) => format!(
            "({})",
            tuple
                .elems
                .iter()
                .map(|element| canonical_type(module, imports, owner, element))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Type::Paren(paren) => {
            format!("({})", canonical_type(module, imports, owner, &paren.elem))
        }
        Type::Group(group) => canonical_type(module, imports, owner, &group.elem),
        other => compact_tokens(other),
    }
}

fn compact_tokens(value: &impl ToTokens) -> String {
    value
        .to_token_stream()
        .to_string()
        .split_whitespace()
        .collect()
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

fn neutral_port_receivers(
    module: &str,
    imports: &Imports,
    signature: &Signature,
) -> BTreeMap<String, BTreeSet<String>> {
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
            let methods = approved_neutral_port_methods(module, imports, &argument.ty);
            (!methods.is_empty()).then(|| (pattern.ident.to_string(), methods))
        })
        .collect()
}

fn receiver_parameters(
    module: &str,
    imports: &Imports,
    signature: &Signature,
) -> BTreeMap<String, ReceiverProvenance> {
    let mut parameters = BTreeMap::new();
    for input in &signature.inputs {
        match input {
            FnArg::Receiver(_) => {
                parameters.insert("self".to_string(), ReceiverProvenance::Local);
            }
            FnArg::Typed(argument) => {
                let Pat::Ident(pattern) = argument.pat.as_ref() else {
                    continue;
                };
                parameters.insert(
                    pattern.ident.to_string(),
                    type_provenance(module, imports, &argument.ty),
                );
            }
        }
    }
    parameters
}

fn type_provenance(module: &str, imports: &Imports, ty: &Type) -> ReceiverProvenance {
    match ty {
        Type::Reference(reference) => type_provenance(module, imports, &reference.elem),
        Type::Paren(paren) => type_provenance(module, imports, &paren.elem),
        Type::Group(group) => type_provenance(module, imports, &group.elem),
        Type::Path(path) if path.qself.is_none() => path_provenance(module, imports, &path.path),
        Type::Array(_) | Type::Slice(_) | Type::Ptr(_) | Type::Tuple(_) => {
            ReceiverProvenance::External
        }
        Type::TraitObject(_) => ReceiverProvenance::Local,
        Type::ImplTrait(_) => ReceiverProvenance::Unknown,
        _ => ReceiverProvenance::Unknown,
    }
}

fn path_provenance(module: &str, imports: &Imports, path: &syn::Path) -> ReceiverProvenance {
    let path = path_segments(path);
    let Some(first) = path.first() else {
        return ReceiverProvenance::Unknown;
    };
    if is_primitive_type(first) || is_prelude_type(first) {
        return ReceiverProvenance::External;
    }
    if is_external_crate_root(first) {
        return ReceiverProvenance::External;
    }
    if let Some(imported) = imports.aliases.get(first) {
        return if imported.first().is_some_and(|root| {
            is_external_crate_root(root) || matches!(root.as_str(), "std" | "core" | "alloc")
        }) {
            ReceiverProvenance::External
        } else {
            ReceiverProvenance::Local
        };
    }
    if matches!(first.as_str(), "crate" | "self" | "super" | "Self") {
        return ReceiverProvenance::Local;
    }
    if first
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_uppercase())
    {
        let resolved = resolve_decl_path(module, imports, path);
        return if is_local_path(&resolved) {
            ReceiverProvenance::Local
        } else {
            ReceiverProvenance::External
        };
    }
    ReceiverProvenance::Unknown
}

fn is_prelude_type(name: &str) -> bool {
    matches!(
        name,
        "Box"
            | "Cow"
            | "Option"
            | "Result"
            | "String"
            | "Vec"
            | "VecDeque"
            | "BTreeMap"
            | "BTreeSet"
            | "HashMap"
            | "HashSet"
            | "Arc"
            | "Rc"
    )
}

fn approved_neutral_port_methods(module: &str, imports: &Imports, ty: &Type) -> BTreeSet<String> {
    let ty = match ty {
        Type::Reference(reference) => reference.elem.as_ref(),
        Type::Paren(paren) => paren.elem.as_ref(),
        Type::Group(group) => group.elem.as_ref(),
        other => other,
    };
    let Type::TraitObject(trait_object) = ty else {
        return BTreeSet::new();
    };
    let mut methods = BTreeSet::new();
    for bound in &trait_object.bounds {
        let TypeParamBound::Trait(bound) = bound else {
            continue;
        };
        for candidate in resolve_trait_candidates(module, imports, &bound.path) {
            if let Some(approved) = approved_neutral_trait(&candidate.join("::")) {
                methods.extend(approved.iter().map(|method| (*method).to_string()));
            }
        }
    }
    methods
}

fn resolve_trait_candidates(module: &str, imports: &Imports, path: &syn::Path) -> Vec<Vec<String>> {
    let path = path_segments(path);
    match path.as_slice() {
        [first, ..] if matches!(first.as_str(), "crate" | "self" | "super") => {
            vec![absolutize_path(module, path)]
        }
        [first, rest @ ..] if imports.aliases.contains_key(first) => {
            let mut resolved = imports.aliases[first].clone();
            resolved.extend(rest.iter().cloned());
            vec![resolved]
        }
        [first, ..] if is_external_crate_root(first) => vec![path],
        _ => {
            let mut candidates = imports
                .globs
                .iter()
                .map(|glob| {
                    let mut candidate = glob.clone();
                    candidate.extend(path.iter().cloned());
                    candidate
                })
                .collect::<Vec<_>>();
            let mut local = module_segments(module);
            local.extend(path);
            candidates.push(local);
            candidates
        }
    }
}

fn approved_neutral_trait(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "unica_format_core::ports::ProbePort" => Some(&["probe"]),
        "unica_format_core::ports::CapturedSourceSession" => {
            Some(&["source", "snapshot", "binding", "as_any"])
        }
        "unica_format_core::ports::CapturePort" => Some(&["capture"]),
        "unica_format_core::ports::ReadPort" => Some(&["read"]),
        "unica_format_core::ports::OwnershipPort" => Some(&["resolve"]),
        "unica_format_core::ports::FormatInspectionPort" => Some(&["inspect"]),
        "unica_format_core::ports::SupportPort" => Some(&["inspect"]),
        "unica_format_core::ports::WritePort" => Some(&["write"]),
        "unica_format_core::ports::ValidationPort" => Some(&["validate"]),
        "unica_format_core::ports::CapabilityPort" => Some(&["capabilities"]),
        "unica_format_core::ports::ObjectKindRegistryPort" => {
            Some(&["resolve", "ordered_kinds", "lease", "project"])
        }
        "unica_format_core::ports::SemanticArtifactPort" => Some(&["read", "bytes"]),
        "unica_format_core::ports::CompatibilityPort" => Some(&["inspect"]),
        "unica_format_core::ports::SourceCompatibilityPort" => Some(&["inspect_source"]),
        "unica_format_core::ports::AuthorabilityPort" => Some(&["inspect"]),
        "unica_format_core::ports::ValidationContextPort" => Some(&["inspect"]),
        "unica_format_core::ports::OperationalValidationPort" => Some(&["validate"]),
        "unica_format_core::ports::PublicationPort" => Some(&["publish"]),
        "unica_application::navigation::SourceRegistrationResolver" => {
            Some(&["locate", "authorize_continuation"])
        }
        _ => None,
    }
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
fn local_port_suffix_does_not_terminate_reachability() {
    let graph = fixture_graph(
        "local-port-suffix",
        &[(
            "entry.rs",
            r#"
                pub trait NeutralPort { fn inspect(&self); }
                trait HiddenBoundary {
                    fn inspect(&self) {
                        let _ = std::path::Path::new(".").join("Template.xml");
                    }
                }
                pub fn run(port: &dyn NeutralPort) { port.inspect(); }
            "#,
        )],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("Template.xml")),
        "{violations:?}"
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
                use unica_format_core::ports::ValidationContextPort;
                trait HiddenBoundary {
                    fn inspect(&self) {
                        let _ = std::path::Path::new(".").join("Template.xml");
                    }
                }
                pub fn run(port: &dyn ValidationContextPort) { port.inspect(); }
            "#,
        )],
    );

    assert!(
        graph.violations_from(&["crate::entry::run"]).is_empty(),
        "statically typed neutral port call must terminate at the port boundary"
    );
}

#[test]
fn every_legal_impl_self_type_is_indexed_and_reachable() {
    let graph = fixture_graph(
        "impl-self-types",
        &[(
            "entry.rs",
            r#"
                trait RefBoundary { fn via_ref(&self); }
                trait WrappedBoundary { fn via_wrapper(&self); }
                trait TupleBoundary { fn via_tuple(&self); }
                trait ArrayBoundary { fn via_array(&self); }
                trait SliceBoundary { fn via_slice(&self); }
                struct Boundary;
                struct Wrapper<T>(T);

                impl RefBoundary for &Boundary {
                    fn via_ref(&self) {
                        let _ = std::path::Path::new(".").join("Configuration.xml");
                    }
                }
                impl<T> WrappedBoundary for Wrapper<T> {
                    fn via_wrapper(&self) {
                        let _ = std::path::Path::new(".").join("ParentConfigurations.bin");
                    }
                }
                impl TupleBoundary for (Boundary,) {
                    fn via_tuple(&self) {
                        let _ = std::path::Path::new(".").join("Rights.xml");
                    }
                }
                impl ArrayBoundary for [Boundary; 1] {
                    fn via_array(&self) {
                        let _ = std::path::Path::new(".").join("Form.xml");
                    }
                }
                impl SliceBoundary for [Boundary] {
                    fn via_slice(&self) {
                        let _ = std::path::Path::new(".").join("Template.xml");
                    }
                }

                pub fn run(
                    a: &Boundary,
                    b: Wrapper<Boundary>,
                    c: (Boundary,),
                    d: [Boundary; 1],
                    e: &[Boundary],
                ) {
                    a.via_ref();
                    b.via_wrapper();
                    c.via_tuple();
                    d.via_array();
                    e.via_slice();
                }
            "#,
        )],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    for native_artifact in [
        "Configuration.xml",
        "ParentConfigurations.bin",
        "Rights.xml",
        "Form.xml",
        "Template.xml",
    ] {
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(native_artifact)),
            "{native_artifact} escaped through an unindexed impl self type: {violations:?}"
        );
    }
    let wrapped_owners = &graph.methods_by_name["via_wrapper"];
    assert!(
        wrapped_owners.iter().any(|target| {
            let owner = target.owner.join("::");
            owner.contains("Wrapper<") && owner.contains('T')
        }),
        "generic arguments were erased from impl identity: {wrapped_owners:?}"
    );
}

#[test]
fn receiver_method_without_any_candidate_fails_closed() {
    let graph = fixture_graph(
        "missing-method-candidate",
        &[(
            "entry.rs",
            "pub struct Unknown; pub fn run(value: Unknown) { value.hidden_native_reader(); }",
        )],
    );

    let violations = graph.violations_from(&["crate::entry::run"]);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("unresolved receiver method hidden_native_reader")),
        "{violations:?}"
    );
}

#[test]
fn neutral_port_provenance_cannot_be_spoofed() {
    let cases = [
        (
            "local-host-port",
            r#"
                trait HostPort { fn inspect(&self); }
                trait HiddenBoundary {
                    fn inspect(&self) {
                        let _ = std::path::Path::new(".").join("Configuration.xml");
                    }
                }
                pub fn run(port: &dyn HostPort) { port.inspect(); }
            "#,
        ),
        (
            "reexport-spoof",
            r#"
                mod spoof { pub trait ValidationContextPort { fn inspect(&self); } }
                pub use spoof::ValidationContextPort;
                trait HiddenBoundary {
                    fn inspect(&self) {
                        let _ = std::path::Path::new(".").join("Rights.xml");
                    }
                }
                pub fn run(port: &dyn ValidationContextPort) { port.inspect(); }
            "#,
        ),
        (
            "alias-spoof",
            r#"
                mod spoof { pub trait HostBoundary { fn inspect(&self); } }
                use crate::entry::spoof::HostBoundary as OperationalValidationPort;
                trait HiddenBoundary {
                    fn inspect(&self) {
                        let _ = std::path::Path::new(".").join("Form.xml");
                    }
                }
                pub fn run(port: &dyn OperationalValidationPort) { port.inspect(); }
            "#,
        ),
        (
            "shadow-spoof",
            r#"
                mod local {
                    pub trait ValidationContextPort { fn inspect(&self); }
                }
                use local::ValidationContextPort;
                trait HiddenBoundary {
                    fn inspect(&self) {
                        let _ = std::path::Path::new(".").join("Template.xml");
                    }
                }
                pub fn run(port: &dyn ValidationContextPort) { port.inspect(); }
            "#,
        ),
    ];

    for (label, source) in cases {
        let graph = fixture_graph(label, &[("entry.rs", source)]);
        let violations = graph.violations_from(&["crate::entry::run"]);
        assert!(
            !violations.is_empty(),
            "{label} was accepted as an approved neutral port"
        );
    }
}

#[test]
fn exact_neutral_port_alias_and_glob_imports_are_approved_by_provenance() {
    let graph = fixture_graph(
        "real-port-imports",
        &[(
            "entry.rs",
            r#"
                use unica_format_core::ports::ValidationContextPort as ContextPort;
                use unica_format_core::ports::*;
                trait HiddenBoundary {
                    fn inspect(&self) {
                        let _ = std::path::Path::new(".").join("Configuration.xml");
                    }
                    fn validate(&self) {
                        let _ = std::path::Path::new(".").join("Rights.xml");
                    }
                }
                pub fn run(
                    context: &dyn ContextPort,
                    validation: &dyn OperationalValidationPort,
                ) {
                    context.inspect();
                    validation.validate();
                }
            "#,
        )],
    );

    assert!(
        graph.violations_from(&["crate::entry::run"]).is_empty(),
        "exact neutral port imports must terminate at their declared methods"
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

//! Feature-only integration facade for the protocol-v5 ReceiptLedger contract.
//!
//! The facade owns no scenario logic: every scenario runs through the
//! production scenario runner, and a shape or action the runner does not know
//! is a scenario error, never evidence of anything.

use crate::application::receipt_ledger::{
    canonical_v5_terminal, receipt_key_digest, request_scope_hash, task_link_digest,
    CoreIdentityDigest, ReceiptKey, ReceiptKeyDigest, ReceiptTerminalOutcome, RequestIdentity,
    RequestScopeHash, TaskLinkIdentity, V5ToolIdentity,
};
use crate::domain::invocation::{InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId};
use crate::infrastructure::daemon::runtime_v5::run_supported_receipt_scenario_for_test;
use serde::de::DeserializeOwned;
use std::fmt;
use std::str::FromStr;

pub fn execute_scenario_json(request: &str) -> Result<String, String> {
    run_supported_receipt_scenario_for_test(request)
}

pub fn receipt_writer_wall_load_supported_for_test() -> bool {
    crate::infrastructure::platform::receipt_writer_wall_load_supported_for_test()
}

pub fn request_scope_hash_for_test(workspace_hint: &str) -> String {
    request_scope_hash(workspace_hint)
        .unwrap_or_else(|error| panic!("invalid request scope supplied by test: {error}"))
        .to_string()
}

pub fn receipt_key_digest_for_test(
    invocation_id: &str,
    reserved_task_id: &str,
    core_identity_digest: &str,
    tool_wire_name: &str,
    normalized_arguments_hash: &str,
    request_scope_hash_value: &str,
) -> String {
    let core_identity_digest: CoreIdentityDigest =
        parse_from_str(core_identity_digest, "core identity digest");
    let normalized_arguments_hash: NormalizedArgumentsHash =
        parse_application_json_string(normalized_arguments_hash, "normalized arguments hash");
    let request_scope_hash_value: RequestScopeHash =
        parse_from_str(request_scope_hash_value, "request scope hash");
    let request_identity = RequestIdentity::new(
        core_identity_digest,
        V5ToolIdentity::from_wire_name(tool_wire_name)
            .unwrap_or_else(|| panic!("invalid v5 tool identity supplied by test")),
        normalized_arguments_hash,
        request_scope_hash_value,
    );
    let invocation_id: InvocationId = parse_from_str(invocation_id, "invocation id");
    let reserved_task_id: TaskId = parse_from_str(reserved_task_id, "reserved task id");
    let key = ReceiptKey::new(invocation_id, reserved_task_id, request_identity);
    receipt_key_digest(&key).to_string()
}

pub fn task_link_digest_for_test(
    receipt_key_digest_value: &str,
    task_id: &str,
    invocation_id: &str,
    workspace_identity_hash: &str,
) -> String {
    let receipt_key_digest_value: ReceiptKeyDigest =
        parse_from_str(receipt_key_digest_value, "receipt key digest");
    let task_id: TaskId = parse_from_str(task_id, "task id");
    let invocation_id: InvocationId = parse_from_str(invocation_id, "invocation id");
    let workspace_identity_hash: SafeIdentityHash =
        parse_application_json_string(workspace_identity_hash, "workspace identity hash");
    let identity = TaskLinkIdentity::new(
        receipt_key_digest_value,
        task_id,
        invocation_id,
        workspace_identity_hash,
    );
    task_link_digest(&identity).to_string()
}

pub fn canonical_v5_terminal_for_test(terminal_json: &str) -> (Vec<u8>, String) {
    let outcome: ReceiptTerminalOutcome = serde_json::from_str(terminal_json)
        .unwrap_or_else(|error| panic!("invalid strict v5 terminal supplied by test: {error}"));
    let terminal = canonical_v5_terminal(&outcome)
        .unwrap_or_else(|error| panic!("v5 terminal cannot be canonicalized: {error}"));
    (terminal.payload().to_vec(), terminal.digest().to_string())
}

fn parse_from_str<T>(value: &str, label: &str) -> T
where
    T: FromStr,
    T::Err: fmt::Display,
{
    value
        .parse()
        .unwrap_or_else(|error| panic!("invalid {label} supplied by test: {error}"))
}

fn parse_application_json_string<T>(value: &str, label: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .unwrap_or_else(|error| panic!("invalid {label} supplied by test: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::{TokenStream, TokenTree};
    use std::collections::{BTreeMap, BTreeSet};
    use syn::parse::Parser;
    use syn::visit::Visit;

    const FACADE_PRODUCTION_SOURCES: &[(&str, &str)] = &[(
        "receipt_ledger_test_support.rs",
        include_str!("receipt_ledger_test_support.rs"),
    )];

    const OWNER_HELPER_PRODUCTION_SOURCES: &[(&str, &str)] = &[(
        "infrastructure/daemon/runtime_v5/receipt_scenario_v5.rs",
        include_str!("infrastructure/daemon/runtime_v5/receipt_scenario_v5.rs"),
    )];

    fn facade_forbidden_authority_references(
        source: &str,
        forbidden: &[&str],
    ) -> Result<BTreeSet<String>, String> {
        facade_forbidden_authority_references_with_mode(
            source,
            forbidden,
            FacadeAuthorityGuardMode::SealedRuntimeStrings,
        )
    }

    fn facade_forbidden_authority_references_with_mode(
        source: &str,
        forbidden: &[&str],
        mode: FacadeAuthorityGuardMode,
    ) -> Result<BTreeSet<String>, String> {
        let syntax = syn::parse_file(source).map_err(|error| error.to_string())?;
        let mut finder = FacadeAuthorityReferenceFinder {
            forbidden,
            references: BTreeSet::new(),
            unsupported_syntax: BTreeSet::new(),
            mode,
        };
        finder.visit_file(&syntax);
        if finder.unsupported_syntax.is_empty() {
            Ok(finder.references)
        } else {
            Err(finder
                .unsupported_syntax
                .into_iter()
                .collect::<Vec<_>>()
                .join("; "))
        }
    }

    fn facade_forbidden_authority_references_by_source(
        sources: &[(&str, &str)],
        forbidden: &[&str],
    ) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
        let mut source_paths = BTreeSet::new();
        let mut references_by_source = BTreeMap::new();
        for (path, source) in sources {
            if !source_paths.insert(*path) {
                return Err(format!("duplicate facade production source `{path}`"));
            }
            let references = facade_forbidden_authority_references_with_mode(
                source,
                forbidden,
                FacadeAuthorityGuardMode::RustAuthorityOnly,
            )
            .map_err(|error| format!("{path}: {error}"))?;
            if !references.is_empty() {
                references_by_source.insert((*path).to_string(), references);
            }
        }
        Ok(references_by_source)
    }

    struct FacadeAuthorityReferenceFinder<'a> {
        forbidden: &'a [&'a str],
        references: BTreeSet<String>,
        unsupported_syntax: BTreeSet<String>,
        mode: FacadeAuthorityGuardMode,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum FacadeAuthorityGuardMode {
        SealedRuntimeStrings,
        RustAuthorityOnly,
    }

    impl FacadeAuthorityReferenceFinder<'_> {
        fn scan_text(&mut self, text: &str) {
            for authority in self.forbidden {
                if text.contains(authority) {
                    self.references.insert((*authority).to_string());
                }
            }
        }

        fn scan_path(&mut self, path: &syn::Path) {
            let path = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            self.scan_text(&path);
        }

        fn scan_use_tree(&mut self, tree: &syn::UseTree, prefix: &mut Vec<String>) {
            match tree {
                syn::UseTree::Path(path) => {
                    prefix.push(path.ident.to_string());
                    self.scan_text(&prefix.join("::"));
                    self.scan_use_tree(&path.tree, prefix);
                    prefix.pop();
                }
                syn::UseTree::Name(name) => {
                    prefix.push(name.ident.to_string());
                    self.scan_text(&prefix.join("::"));
                    prefix.pop();
                }
                syn::UseTree::Rename(rename) => {
                    prefix.push(rename.ident.to_string());
                    self.scan_text(&prefix.join("::"));
                    self.scan_text(&rename.rename.to_string());
                    prefix.pop();
                }
                syn::UseTree::Glob(_) => self.scan_text(&prefix.join("::")),
                syn::UseTree::Group(group) => {
                    for tree in &group.items {
                        self.scan_use_tree(tree, prefix);
                    }
                }
            }
        }

        fn scan_literal(&mut self, literal: &syn::Lit) -> String {
            let decoded = match literal {
                syn::Lit::Str(value) => value.value(),
                syn::Lit::ByteStr(value) => {
                    String::from_utf8_lossy(value.value().as_slice()).into_owned()
                }
                syn::Lit::CStr(value) => {
                    String::from_utf8_lossy(value.value().to_bytes()).into_owned()
                }
                syn::Lit::Byte(value) => char::from(value.value()).to_string(),
                syn::Lit::Char(value) => value.value().to_string(),
                syn::Lit::Int(value) => value.base10_digits().to_string(),
                syn::Lit::Float(value) => value.base10_digits().to_string(),
                syn::Lit::Bool(value) => value.value.to_string(),
                syn::Lit::Verbatim(value) => {
                    self.unsupported_syntax.insert(format!(
                        "unsupported literal in facade authority guard: `{value}`"
                    ));
                    value.to_string()
                }
                _ => {
                    self.unsupported_syntax
                        .insert("unsupported future literal in facade authority guard".to_string());
                    String::new()
                }
            };
            self.scan_text(&decoded);
            decoded
        }

        fn scan_concat_tokens(&mut self, tokens: TokenStream) -> String {
            let expressions =
                match syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                    .parse2(tokens)
                {
                    Ok(expressions) => expressions,
                    Err(error) => {
                        self.unsupported_syntax.insert(format!(
                            "unsupported concat! syntax in facade authority guard: {error}"
                        ));
                        return String::new();
                    }
                };
            let mut concatenated = String::new();
            for expression in expressions {
                let fragment = match expression {
                    syn::Expr::Lit(expression) => self.scan_literal(&expression.lit),
                    syn::Expr::Group(expression) => {
                        self.scan_static_string_expression(&expression.expr)
                    }
                    syn::Expr::Paren(expression) => {
                        self.scan_static_string_expression(&expression.expr)
                    }
                    syn::Expr::Macro(expression)
                        if expression
                            .mac
                            .path
                            .segments
                            .last()
                            .is_some_and(|segment| segment.ident == "concat") =>
                    {
                        self.scan_concat_tokens(expression.mac.tokens)
                    }
                    _ => {
                        self.unsupported_syntax.insert(
                            "dynamic concat! expression in facade authority guard".to_string(),
                        );
                        String::new()
                    }
                };
                concatenated.push_str(&fragment);
            }
            self.scan_text(&concatenated);
            concatenated
        }

        fn scan_static_string_expression(&mut self, expression: &syn::Expr) -> String {
            match expression {
                syn::Expr::Lit(expression) => self.scan_literal(&expression.lit),
                syn::Expr::Group(expression) => {
                    self.scan_static_string_expression(&expression.expr)
                }
                syn::Expr::Paren(expression) => {
                    self.scan_static_string_expression(&expression.expr)
                }
                syn::Expr::Macro(expression)
                    if expression
                        .mac
                        .path
                        .segments
                        .last()
                        .is_some_and(|segment| segment.ident == "concat") =>
                {
                    self.scan_concat_tokens(expression.mac.tokens.clone())
                }
                _ => {
                    self.unsupported_syntax
                        .insert("dynamic string expression in facade authority guard".to_string());
                    String::new()
                }
            }
        }

        fn format_static_segments(format: &str) -> Vec<String> {
            let mut segments = vec![String::new()];
            let mut characters = format.chars().peekable();
            while let Some(character) = characters.next() {
                match character {
                    '{' if characters.peek() == Some(&'{') => {
                        characters.next();
                        segments.last_mut().expect("initial segment").push('{');
                    }
                    '}' if characters.peek() == Some(&'}') => {
                        characters.next();
                        segments.last_mut().expect("initial segment").push('}');
                    }
                    '{' => {
                        let mut closed = false;
                        for nested in characters.by_ref() {
                            if nested == '}' {
                                closed = true;
                                break;
                            }
                        }
                        if closed {
                            segments.push(String::new());
                        } else {
                            segments.last_mut().expect("initial segment").push('{');
                        }
                    }
                    character => segments
                        .last_mut()
                        .expect("initial segment")
                        .push(character),
                }
            }
            segments
        }

        fn format_has_explicit_placeholder_selector(format: &str) -> bool {
            let mut characters = format.chars().peekable();
            while let Some(character) = characters.next() {
                if character != '{' {
                    continue;
                }
                if characters.peek() == Some(&'{') {
                    characters.next();
                    continue;
                }
                let selector = characters
                    .by_ref()
                    .take_while(|character| *character != '}')
                    .take_while(|character| *character != ':')
                    .collect::<String>();
                if !selector.is_empty() {
                    return true;
                }
            }
            false
        }

        fn formatting_argument_literal(&mut self, expression: &syn::Expr) -> Option<String> {
            match expression {
                syn::Expr::Lit(expression) => Some(self.scan_literal(&expression.lit)),
                syn::Expr::Assign(expression) => {
                    self.formatting_argument_literal(&expression.right)
                }
                syn::Expr::Group(expression) => self.formatting_argument_literal(&expression.expr),
                syn::Expr::Paren(expression) => self.formatting_argument_literal(&expression.expr),
                _ => None,
            }
        }

        fn scan_formatting_macro(&mut self, tokens: TokenStream, has_destination: bool) {
            let expressions =
                match syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                    .parse2(tokens)
                {
                    Ok(expressions) => expressions.into_iter().collect::<Vec<_>>(),
                    Err(error) => {
                        self.unsupported_syntax.insert(format!(
                            "unsupported formatting macro in facade authority guard: {error}"
                        ));
                        return;
                    }
                };
            let format_index = usize::from(has_destination);
            let Some(syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(format),
                ..
            })) = expressions.get(format_index)
            else {
                self.unsupported_syntax
                    .insert("dynamic formatting template in facade authority guard".to_string());
                return;
            };
            if expressions.len() > format_index + 1
                && Self::format_has_explicit_placeholder_selector(&format.value())
            {
                self.unsupported_syntax.insert(
                    "indexed or named formatting arguments in facade authority guard".to_string(),
                );
                return;
            }
            let segments = Self::format_static_segments(&format.value());
            let arguments = expressions
                .iter()
                .skip(format_index + 1)
                .map(|expression| self.formatting_argument_literal(expression))
                .collect::<Vec<_>>();
            let mut rendered = segments.first().cloned().unwrap_or_default();
            for (index, segment) in segments.iter().skip(1).enumerate() {
                if let Some(Some(argument)) = arguments.get(index) {
                    rendered.push_str(argument);
                }
                rendered.push_str(segment);
            }
            for argument in arguments
                .iter()
                .skip(segments.len().saturating_sub(1))
                .flatten()
            {
                rendered.push_str(argument);
            }
            self.scan_text(&rendered);
        }

        fn scan_macro_tokens(&mut self, tokens: TokenStream) {
            let tokens = tokens.into_iter().collect::<Vec<_>>();
            let mut index = 0;
            while index < tokens.len() {
                match &tokens[index] {
                    TokenTree::Group(group) => {
                        self.scan_macro_tokens(group.stream());
                    }
                    TokenTree::Ident(identifier) => {
                        let identifier = identifier.to_string();
                        if matches!(
                            identifier.as_str(),
                            "concat"
                                | "format"
                                | "format_args"
                                | "write"
                                | "writeln"
                                | "include"
                                | "include_str"
                                | "include_bytes"
                        )
                            && self.mode != FacadeAuthorityGuardMode::RustAuthorityOnly
                            && tokens.get(index + 1).is_some_and(
                                |token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == '!'),
                            )
                        {
                            self.unsupported_syntax.insert(format!(
                                "nested or dynamic `{identifier}!` in facade authority guard"
                            ));
                        }
                        self.scan_text(&identifier);
                    }
                    TokenTree::Literal(literal) => {
                        match syn::parse_str::<syn::Lit>(&literal.to_string()) {
                            Ok(literal) => {
                                self.scan_literal(&literal);
                            }
                            Err(error) => {
                                self.unsupported_syntax.insert(format!(
                                    "unsupported macro literal `{literal}`: {error}"
                                ));
                            }
                        }
                    }
                    TokenTree::Punct(_) => {}
                }
                index += 1;
            }
        }

        fn attributes_have_exact_cfg_test(attributes: &[syn::Attribute]) -> bool {
            attributes.iter().any(|attribute| {
                attribute.path().is_ident("cfg")
                    && attribute
                        .parse_args::<syn::Path>()
                        .is_ok_and(|path| path.is_ident("test"))
            })
        }

        fn item_has_exact_cfg_test(item: &syn::Item) -> bool {
            let attributes: &[syn::Attribute] = match item {
                syn::Item::Const(item) => &item.attrs,
                syn::Item::Enum(item) => &item.attrs,
                syn::Item::ExternCrate(item) => &item.attrs,
                syn::Item::Fn(item) => &item.attrs,
                syn::Item::ForeignMod(item) => &item.attrs,
                syn::Item::Impl(item) => &item.attrs,
                syn::Item::Macro(item) => &item.attrs,
                syn::Item::Mod(item) => &item.attrs,
                syn::Item::Static(item) => &item.attrs,
                syn::Item::Struct(item) => &item.attrs,
                syn::Item::Trait(item) => &item.attrs,
                syn::Item::TraitAlias(item) => &item.attrs,
                syn::Item::Type(item) => &item.attrs,
                syn::Item::Union(item) => &item.attrs,
                syn::Item::Use(item) => &item.attrs,
                syn::Item::Verbatim(_) => &[],
                _ => &[],
            };
            Self::attributes_have_exact_cfg_test(attributes)
        }

        fn impl_item_has_exact_cfg_test(item: &syn::ImplItem) -> bool {
            let attributes: &[syn::Attribute] = match item {
                syn::ImplItem::Const(item) => &item.attrs,
                syn::ImplItem::Fn(item) => &item.attrs,
                syn::ImplItem::Type(item) => &item.attrs,
                syn::ImplItem::Macro(item) => &item.attrs,
                syn::ImplItem::Verbatim(_) => &[],
                _ => &[],
            };
            Self::attributes_have_exact_cfg_test(attributes)
        }

        fn trait_item_has_exact_cfg_test(item: &syn::TraitItem) -> bool {
            let attributes: &[syn::Attribute] = match item {
                syn::TraitItem::Const(item) => &item.attrs,
                syn::TraitItem::Fn(item) => &item.attrs,
                syn::TraitItem::Type(item) => &item.attrs,
                syn::TraitItem::Macro(item) => &item.attrs,
                syn::TraitItem::Verbatim(_) => &[],
                _ => &[],
            };
            Self::attributes_have_exact_cfg_test(attributes)
        }

        fn foreign_item_has_exact_cfg_test(item: &syn::ForeignItem) -> bool {
            let attributes: &[syn::Attribute] = match item {
                syn::ForeignItem::Fn(item) => &item.attrs,
                syn::ForeignItem::Static(item) => &item.attrs,
                syn::ForeignItem::Type(item) => &item.attrs,
                syn::ForeignItem::Macro(item) => &item.attrs,
                syn::ForeignItem::Verbatim(_) => &[],
                _ => &[],
            };
            Self::attributes_have_exact_cfg_test(attributes)
        }
    }

    impl<'ast> Visit<'ast> for FacadeAuthorityReferenceFinder<'_> {
        fn visit_item(&mut self, item: &'ast syn::Item) {
            if Self::item_has_exact_cfg_test(item) {
                return;
            }
            if let syn::Item::Verbatim(tokens) = item {
                self.unsupported_syntax.insert(format!(
                    "unsupported verbatim item in facade authority guard: `{tokens}`"
                ));
                return;
            }
            syn::visit::visit_item(self, item);
        }

        fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
            if Self::impl_item_has_exact_cfg_test(item) {
                return;
            }
            syn::visit::visit_impl_item(self, item);
        }

        fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
            if Self::trait_item_has_exact_cfg_test(item) {
                return;
            }
            syn::visit::visit_trait_item(self, item);
        }

        fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
            if Self::foreign_item_has_exact_cfg_test(item) {
                return;
            }
            syn::visit::visit_foreign_item(self, item);
        }

        fn visit_ident(&mut self, identifier: &'ast proc_macro2::Ident) {
            self.scan_text(&identifier.to_string());
        }

        fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
            self.scan_use_tree(&item.tree, &mut Vec::new());
            syn::visit::visit_item_use(self, item);
        }

        fn visit_path(&mut self, path: &'ast syn::Path) {
            self.scan_path(path);
            syn::visit::visit_path(self, path);
        }

        fn visit_lit(&mut self, literal: &'ast syn::Lit) {
            self.scan_literal(literal);
        }

        fn visit_macro(&mut self, macro_: &'ast syn::Macro) {
            syn::visit::visit_path(self, &macro_.path);
            match macro_.path.segments.last().map(|segment| &segment.ident) {
                Some(identifier) if identifier == "concat" => {
                    self.scan_concat_tokens(macro_.tokens.clone());
                }
                Some(identifier)
                    if matches!(
                        identifier.to_string().as_str(),
                        "format" | "format_args" | "write" | "writeln"
                    ) && self.mode == FacadeAuthorityGuardMode::RustAuthorityOnly =>
                {
                    self.scan_macro_tokens(macro_.tokens.clone());
                }
                Some(identifier)
                    if matches!(
                        identifier.to_string().as_str(),
                        "format" | "format_args" | "write" | "writeln"
                    ) =>
                {
                    self.scan_formatting_macro(
                        macro_.tokens.clone(),
                        matches!(identifier.to_string().as_str(), "write" | "writeln"),
                    );
                }
                Some(identifier)
                    if matches!(
                        identifier.to_string().as_str(),
                        "include" | "include_str" | "include_bytes"
                    ) =>
                {
                    self.unsupported_syntax.insert(format!(
                        "external `{identifier}!` content in facade authority guard"
                    ));
                }
                _ => {
                    self.scan_macro_tokens(macro_.tokens.clone());
                }
            }
        }

        fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
            if expression.method == "concat" {
                self.unsupported_syntax
                    .insert("dynamic `.concat()` in facade authority guard".to_string());
            }
            syn::visit::visit_expr_method_call(self, expression);
        }
    }

    #[test]
    fn authority_wrappers_match_the_frozen_application_vectors() {
        assert_eq!(
            request_scope_hash_for_test("workspace-a"),
            "9f7a5a77bb6eb469cd20147a9aeee9d9769a8372f587bd89635d15684ee02b39"
        );
        assert_eq!(
            receipt_key_digest_for_test(
                "123e4567-e89b-42d3-a456-426614174000",
                "123e4567-e89b-42d3-b456-426614174001",
                &"00".repeat(32),
                "unica.view",
                &"11".repeat(32),
                "9f7a5a77bb6eb469cd20147a9aeee9d9769a8372f587bd89635d15684ee02b39",
            ),
            "9d8f104e7dfb2f4827a24d4b41aefe6c6704bf31bd3df191d84f9893071db549"
        );
        assert_eq!(
            task_link_digest_for_test(
                &"0".repeat(64),
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
                &"a".repeat(64),
            ),
            "4c73d08219973c72e759a9f85e156fa42c9d8e61a56e704b70d1c7c042b73da0"
        );
        assert_eq!(
            canonical_v5_terminal_for_test(r#"{"status":"cancelled"}"#),
            (
                br#"{"status":"cancelled"}"#.to_vec(),
                "f2d0423d2613a0d09397b750542e4542f7653d78ebd5e0448f1326d09145d9ae".to_string(),
            )
        );
    }

    #[test]
    fn scenario_wire_is_closed_at_the_scenario_and_action_levels() {
        let unknown_scenario_field =
            r#"{"clock":"fake","actions":[],"expectedMissing":"forbidden"}"#;
        assert!(execute_scenario_json(unknown_scenario_field).is_err());

        let unknown_action_field = r#"{"clock":"fake","actions":[{"action":"configure_validation","reject":false,"expectedCode":"forbidden"}]}"#;
        assert!(execute_scenario_json(unknown_action_field).is_err());
    }

    fn assert_observed_invalid_request(encoded: &str, label: &str) -> serde_json::Value {
        let envelope: serde_json::Value = serde_json::from_str(encoded).unwrap();
        assert_eq!(
            envelope["kind"], "observed",
            "supported malformed envelopes must execute through the production scenario runner"
        );
        let response = &envelope["payload"]["responses"][label];
        assert_eq!(response["kind"], "rejected", "{label}");
        assert_eq!(response["error"], "invalid_request", "{label}");
        assert!(
            envelope["payload"].get("evidence").is_none(),
            "a supported production response must not mint fallback transition evidence"
        );
        envelope
    }

    #[test]
    fn crash_is_a_staged_control_and_does_not_mint_receipt_transition_evidence() {
        let scenario = r#"{
            "clock":"fake",
            "actions":[
                {"action":"crash","point":"after_side_effect_before_terminal"},
                {"action":"send_outer_envelope","envelope":"missing_invocation_id","label":"strict"}
            ]
        }"#;

        let encoded = execute_scenario_json(scenario)
            .expect("the staged crash allows the next production operation to execute");
        let envelope = assert_observed_invalid_request(&encoded, "strict");
        assert_eq!(
            envelope["payload"]["responses"]
                .as_object()
                .expect("bounded response report")
                .len(),
            1,
            "crash configures the next operation instead of terminating the scenario"
        );
    }

    #[test]
    fn every_malformed_envelope_case_is_routed_individually_through_the_production_pipeline() {
        let cases = [
            "missing_invocation_id",
            "noncanonical_invocation_id",
            "missing_reserved_task_id",
            "noncanonical_reserved_task_id",
            "unknown_tool",
            "unknown_field",
            "malformed_arguments",
            "oversized_arguments",
            "response_budget_above_maximum",
            "empty_workspace_hint",
            "workspace_hint_with_control",
            "malformed_workspace_hint",
            "oversized_workspace_hint",
        ];
        let mut routed_cases = std::collections::BTreeSet::new();

        for envelope_case in cases {
            let scenario = format!(
                r#"{{"clock":"fake","actions":[{{"action":"send_outer_envelope","envelope":"{envelope_case}","label":"strict"}}]}}"#
            );
            let encoded = execute_scenario_json(&scenario)
                .unwrap_or_else(|error| panic!("{envelope_case} reaches production: {error}"));
            assert_observed_invalid_request(&encoded, "strict");
            assert!(routed_cases.insert(envelope_case));
        }

        assert_eq!(routed_cases.len(), cases.len());
    }

    #[test]
    fn facade_authority_guard_scans_every_named_production_source() {
        let sources = [
            ("receipt_ledger_test_support.rs", "fn entrypoint() {}"),
            (
                "synthetic/helper_with_store_authority.rs",
                r#"
                    use crate::application::receipt_ledger::ReceiptLedgerPort;
                    use crate::application::receipt_ledger_actor::ReceiptLedgerActor;
                    use crate::infrastructure::daemon::identity::DaemonStateDirectory;
                    use crate::infrastructure::receipt_ledger::ReceiptLedgerStore;

                    fn bypass_the_owner() {}
                "#,
            ),
            (
                "synthetic/helper_with_retained_state.rs",
                r#"
                    use std::{fs as retained_state_fs};
                    use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;

                    fn bypass_retained_state() {
                        let _ = retained_state_fs::read("receipts");
                    }
                "#,
            ),
        ];
        let forbidden = [
            "ReceiptLedgerStore",
            "ReceiptLedgerActor",
            "ReceiptLedgerPort",
            "DaemonStateDirectory",
            "RetainedDirectoryCapability",
            "::fs",
            "filesystem",
        ];

        let references = facade_forbidden_authority_references_by_source(&sources, &forbidden)
            .expect("synthetic facade sources parse");
        assert_eq!(
            references,
            BTreeMap::from([
                (
                    "synthetic/helper_with_store_authority.rs".to_string(),
                    BTreeSet::from([
                        "DaemonStateDirectory".to_string(),
                        "ReceiptLedgerActor".to_string(),
                        "ReceiptLedgerPort".to_string(),
                        "ReceiptLedgerStore".to_string(),
                    ]),
                ),
                (
                    "synthetic/helper_with_retained_state.rs".to_string(),
                    BTreeSet::from([
                        "::fs".to_string(),
                        "RetainedDirectoryCapability".to_string(),
                        "filesystem".to_string(),
                    ]),
                ),
            ])
        );
    }

    #[test]
    fn facade_and_non_owner_helpers_have_no_direct_receipt_ledger_authority() {
        assert_eq!(
            FACADE_PRODUCTION_SOURCES
                .iter()
                .map(|(path, _)| *path)
                .collect::<Vec<_>>(),
            ["receipt_ledger_test_support.rs"],
            "every non-owner production helper belongs to the guard inventory"
        );

        let forbidden = [
            "ReceiptLedgerStore",
            "ReceiptLedgerActor",
            "ReceiptLedgerPort",
            "DaemonStateDirectory",
            "ReceiptAuthorityLock",
            "RetainedDirectoryCapability",
            "RetainedRegularFileCapability",
            "RetainedChildCapability",
            "open_retained_directory",
            "create_private_retained_subdirectory",
            "::fs",
            "filesystem",
        ];
        let references =
            facade_forbidden_authority_references_by_source(FACADE_PRODUCTION_SOURCES, &forbidden)
                .unwrap_or_else(|error| panic!("facade authority guard failed closed: {error}"));
        assert!(
            references.is_empty(),
            "facade production sources contain direct receipt-ledger authority: {references:?}"
        );
    }

    #[test]
    fn scenario_driver_cannot_mint_runtime_telemetry_events() {
        let references = facade_forbidden_authority_references_by_source(
            OWNER_HELPER_PRODUCTION_SOURCES,
            &["record_runtime_entry", "record_event"],
        )
        .unwrap_or_else(|error| panic!("scenario telemetry guard failed closed: {error}"));
        assert!(
            references.is_empty(),
            "scenario driver mints runtime telemetry instead of reading it: {references:?}"
        );
    }

    #[test]
    fn rust_authority_guard_finds_store_identifiers_inside_formatting_macros() {
        let source = r#"
            fn bypass() {
                let _ = format!("{}", format!("{:?}", ReceiptLedgerStore::open));
            }
        "#;
        let references = facade_forbidden_authority_references_with_mode(
            source,
            &["ReceiptLedgerStore", "ReceiptLedgerPort"],
            FacadeAuthorityGuardMode::RustAuthorityOnly,
        )
        .expect("Rust authority scanning accepts dynamic formatting syntax");

        assert_eq!(
            references,
            BTreeSet::from(["ReceiptLedgerStore".to_owned()])
        );
    }

    #[test]
    fn facade_authority_guard_ignores_only_exact_cfg_test_subtrees() {
        let source = r#"
            fn production_entry() {}

            #[cfg(test)]
            mod tests {
                const EXPECTED_CODE: &str = "writer_path_unavailable";
                fn assertion_mentions_type() {
                    let _ = "ProductionBoundary";
                }
            }
        "#;

        assert!(facade_forbidden_authority_references(
            source,
            &["writer_path_unavailable", "ProductionBoundary"],
        )
        .expect("synthetic facade source parses")
        .is_empty());
    }

    #[test]
    fn facade_authority_guard_rejects_production_string_literals() {
        let source = r#"
            const FORGED_CODE: &str = "writer_path_unavailable";
        "#;

        assert_eq!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses"),
            BTreeSet::from(["writer_path_unavailable".to_string()])
        );
    }

    #[test]
    fn facade_authority_guard_audits_mixed_cfg_subtrees() {
        let source = r#"
            #[cfg(any(test, feature = "receipt-ledger-test-support"))]
            const FORGED_CODE: &str = "writer_path_unavailable";
        "#;

        assert_eq!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses"),
            BTreeSet::from(["writer_path_unavailable".to_string()])
        );
    }

    #[test]
    fn facade_authority_guard_reconstructs_split_concat_literals() {
        let source = r#"
            const FORGED_CODE: &str = concat!("writer_path", "_unavailable");
        "#;

        assert_eq!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses"),
            BTreeSet::from(["writer_path_unavailable".to_string()])
        );
    }

    #[test]
    fn facade_authority_guard_reconstructs_nested_concat_literals() {
        let source = r#"
            const FORGED_CODE: &str =
                concat!("writer_", concat!("path", "_unavailable"));
        "#;

        assert_eq!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses"),
            BTreeSet::from(["writer_path_unavailable".to_string()])
        );
    }

    #[test]
    fn facade_authority_guard_fails_closed_on_dynamic_string_construction() {
        for source in [
            r#"fn forged() { let _ = format!("writer_path{}", "_unavailable"); }"#,
            r#"fn forged() { let _ = format!("{1}{0}", "_unavailable", "writer_path"); }"#,
            r#"fn forged() { let _ = ["writer_path", "_unavailable"].concat(); }"#,
            r#"fn forged(out: &mut String) { let _ = write!(out, "writer_path{}", "_unavailable"); }"#,
            r#"include!("forged-evidence.rs");"#,
            r#"fn forged() { let _ = include_str!("forged-evidence.txt"); }"#,
        ] {
            match facade_forbidden_authority_references(source, &["writer_path_unavailable"]) {
                Err(_) => {}
                Ok(references) => assert!(
                    !references.is_empty(),
                    "dynamic production string construction must fail closed: {source}"
                ),
            }
        }
    }

    #[test]
    fn facade_authority_guard_does_not_concatenate_unrelated_macro_literals() {
        let source = r#"
            fn classify(value: &str) -> bool {
                matches!(value, "writer_path" | "_unavailable")
            }
        "#;

        assert!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("ordinary macro literals are audited independently")
                .is_empty()
        );
    }

    #[test]
    fn facade_authority_guard_ignores_exact_cfg_test_associated_items() {
        let source = r#"
            struct ProductionOwner;
            impl ProductionOwner {
                #[cfg(test)]
                const EXPECTED_CODE: &str = "writer_path_unavailable";
            }
        "#;

        assert!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses")
                .is_empty()
        );
    }

    #[test]
    fn facade_source_keeps_the_exact_abi_and_has_no_production_authority() {
        let source = include_str!("receipt_ledger_test_support.rs");
        let syntax = syn::parse_file(source).expect("facade source parses");
        let public_functions = syntax
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Fn(function) if matches!(function.vis, syn::Visibility::Public(_)) => {
                    Some(function.sig.ident.to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            public_functions,
            [
                "execute_scenario_json",
                "receipt_writer_wall_load_supported_for_test",
                "request_scope_hash_for_test",
                "receipt_key_digest_for_test",
                "task_link_digest_for_test",
                "canonical_v5_terminal_for_test",
            ]
        );

        let forbidden = [
            concat!("sha", "2"),
            concat!("ReceiptLedger", "Store"),
            concat!("Production", "Boundary"),
            concat!("receipt_row", "_absent"),
            concat!("protocol_behavior", "_unavailable"),
            concat!("strict_envelope_observation", "_unavailable"),
            concat!("writer_path", "_unavailable"),
            concat!("task_projection", "_unavailable"),
            concat!("receipt_transition", "_unavailable"),
            concat!("capacity_latch", "_unavailable"),
            concat!("receipt_identity", "_unavailable"),
            concat!("cross_store_intent", "_unavailable"),
            concat!("v5_receipt_runtime", "_entered"),
            concat!("protocol_frame", "_read"),
            concat!("ReachedProduction", "Boundary"),
            concat!("Facade", "Envelope"),
            concat!("production_missing", "_transition"),
        ];
        let references = facade_forbidden_authority_references(source, &forbidden)
            .unwrap_or_else(|error| panic!("facade authority guard failed closed: {error}"));
        assert!(
            references.is_empty(),
            "facade source contains forbidden production authorities: {references:?}"
        );
    }
}

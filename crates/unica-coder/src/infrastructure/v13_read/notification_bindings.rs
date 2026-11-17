//! Обратные ссылки на места создания описания оповещения.

use super::*;
use unica_bsl::projection::MethodKind;
use unica_bsl_parser::symbols::{name_key, LineIndex};
use unica_source_contract::read::module_text_revision;

pub(super) fn can_receive(module: &ModuleProjectionSet, method: &MethodProjection) -> bool {
    let props = &module.summary().props;
    method.method_kind == MethodKind::Procedure
        && method.export
        && (props.role == ModuleRole::Form.as_str()
            || props
                .common_module
                .as_ref()
                .is_some_and(|common| common.global == Some(false)))
}

enum Receiver<'a> {
    ThisForm,
    CommonModule(&'a str),
}

impl Receiver<'_> {
    fn names(&self, name: &str) -> bool {
        let key = name_key(name);
        match self {
            Self::ThisForm => matches!(key.as_str(), "thisobject" | "этотобъект"),
            Self::CommonModule(expected) => key == *expected,
        }
    }
}

#[derive(Default)]
struct NotificationBindings {
    items: Vec<Value>,
    limits: Vec<String>,
    unproved: usize,
    indexing: bool,
}

impl NotificationBindings {
    fn inspect(
        &mut self,
        origin: &ModuleProjectionSet,
        target: &MethodProjection,
        receiver: &Receiver<'_>,
    ) {
        let source = origin.source_text();
        let parsed = unica_bsl_parser::parse(source);
        let sites = parsed.notification_descriptions();
        if sites.is_empty() {
            return;
        }
        let target_name = name_key(&target.name);
        let declarations = parsed.local_methods();
        let lines = LineIndex::new(source);
        let rev = module_text_revision(source.as_bytes());
        for site in sites {
            if site
                .handler
                .as_deref()
                .is_some_and(|handler| name_key(handler) != target_name)
                || site
                    .receiver
                    .as_deref()
                    .is_some_and(|name| !receiver.names(name))
            {
                continue;
            }
            let method = declarations
                .iter()
                .find(|method| {
                    method.range.start <= site.range.start && method.range.end >= site.range.end
                })
                .and_then(|declaration| unique_method(origin, &declaration.name));
            let (line, character) = lines.position(site.range.start);
            let Some(method) = method.filter(|method| {
                site.handler.is_some()
                    && site.receiver.is_some()
                    && (!matches!(receiver, Receiver::ThisForm)
                        || method.compile.form_context == Some(true))
                    && compatible_context(origin, method, target, line + 1)
            }) else {
                self.unproved += 1;
                continue;
            };
            let (end_line, end_character) = lines.position(site.range.end);
            self.items.push(json!({
                "at": method.at,
                "kind": NodeKind::Method.as_str(),
                "binding": "notifyDescription",
                "range": {
                    "start": {"line": line, "character": character},
                    "end": {"line": end_line, "character": end_character},
                },
                "rev": rev,
            }));
        }
    }
}

impl LogicalViewReadAuthority<'_> {
    pub(super) fn method_binding_view(
        &self,
        requested: &QualifiedAddress,
        module_at: &QualifiedAddress,
        module: &ModuleProjectionSet,
        method: &MethodProjection,
        mut declarative: Vec<Value>,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        let mut notifications = NotificationBindings::default();
        if can_receive(module, method) {
            if unique_method(module, &method.name).is_none() {
                notifications
                    .limits
                    .push("Notification bindings require a unique handler declaration".into());
            } else if module.summary().props.role == ModuleRole::Form.as_str() {
                notifications.inspect(module, method, &Receiver::ThisForm);
            } else {
                let module_name = module_at.segments()[0].name().unwrap_or_default();
                self.read_common_notification_bindings(
                    module_name,
                    method,
                    admitted,
                    &mut notifications,
                )?;
            }
        }
        if notifications.unproved > 0 {
            notifications.limits.push(format!(
                "{} notification creation sites have a dynamic name or receiver, an ambiguous method or an unproved context",
                notifications.unproved
            ));
        }
        declarative.extend(notifications.items);
        let props = if notifications.indexing {
            Map::from_iter([("notificationIndex".into(), json!("indexing"))])
        } else {
            Map::new()
        };
        let node = NodeView::new(requested.to_string(), "Binding", "Binding", props)
            .with_limits(notifications.limits);
        Ok(NodeViewData::Collection(CollectionView::new(
            node,
            declarative,
        )))
    }

    fn read_common_notification_bindings(
        &self,
        module_name: &str,
        method: &MethodProjection,
        admitted: &ViewSourceSnapshot,
        notifications: &mut NotificationBindings,
    ) -> Result<(), ViewError> {
        notifications.limits.push(
            "Cross-module notification bindings use the asynchronously refreshed name index of this source set. Dynamic receivers and implicit scopes outside form and common modules are not resolved".into(),
        );
        let Some(candidates) = self.modules_naming(module_name) else {
            notifications.indexing = true;
            notifications
                .limits
                .push("Notification binding source index is building".into());
            return Ok(());
        };
        if !candidates.complete {
            notifications
                .limits
                .push("Notification binding source index is incomplete".into());
        }
        let receiver_name = name_key(module_name);
        let mut unreadable = 0;
        for path in candidates.paths.into_iter().collect::<BTreeSet<_>>() {
            self.read_checkpoint()?;
            let Some(at) = self.module_address_for_path(&path) else {
                unreadable += 1;
                continue;
            };
            let Some(capability) = self.profile.module_capability(&at) else {
                unreadable += 1;
                continue;
            };
            if !matches!(capability.role(), ModuleRole::Common | ModuleRole::Form) {
                continue;
            }
            let projection = (|| {
                self.verify_module_owner(&at, capability, admitted)?;
                if capability.role() == ModuleRole::Form
                    && self.form_attribute_shadows(&at, &receiver_name, admitted)?
                {
                    return Ok(None);
                }
                self.module_projection(&at, capability, admitted).map(Some)
            })();
            match projection {
                Ok(Some(projection)) => {
                    notifications.inspect(
                        &projection,
                        method,
                        &Receiver::CommonModule(&receiver_name),
                    );
                }
                Ok(None) => {}
                Err(error)
                    if matches!(
                        error.code(),
                        RefusalCode::DeadlineExceeded
                            | RefusalCode::Cancelled
                            | RefusalCode::ConcurrentChange
                    ) =>
                {
                    return Err(error)
                }
                Err(_) => unreadable += 1,
            }
        }
        if unreadable > 0 {
            notifications.limits.push(format!(
                "{unreadable} notification binding candidate modules could not be read"
            ));
        }
        Ok(())
    }

    fn form_attribute_shadows(
        &self,
        module_at: &QualifiedAddress,
        receiver: &str,
        admitted: &ViewSourceSnapshot,
    ) -> Result<bool, ViewError> {
        let form_at = module_at.prefix(module_at.segments().len() - 1);
        let target = MetadataAddress::parse_logical(&form_at.logical_path()).map_err(|error| {
            ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
        })?;
        Ok(self
            .form_data(&target, admitted)?
            .attributes
            .iter()
            .any(|attribute| name_key(&attribute.name) == receiver))
    }
}

fn unique_method<'a>(module: &'a ModuleProjectionSet, name: &str) -> Option<&'a MethodProjection> {
    let key = name_key(name);
    let mut matches = module
        .methods()
        .iter()
        .filter(|method| name_key(&method.name) == key);
    let method = matches.next()?;
    matches.next().is_none().then_some(method)
}

fn compatible_context(
    origin: &ModuleProjectionSet,
    caller: &MethodProjection,
    target: &MethodProjection,
    line: usize,
) -> bool {
    let Some((caller_contexts, target_contexts)) = caller
        .compile
        .contexts
        .as_ref()
        .zip(target.compile.contexts.as_ref())
    else {
        return false;
    };
    let site_guard = origin
        .compilation()
        .iter()
        .filter(|range| (range.from_line..=range.to_line).contains(&line))
        .min_by_key(|range| range.to_line - range.from_line);
    let effective_guard = site_guard
        .map(|range| range.guard.as_str())
        .or(caller.compile.guard.as_deref());
    (target.compile.guard.is_none() || target.compile.guard.as_deref() == effective_guard)
        && caller_contexts.iter().any(|context| {
            target_contexts.contains(context)
                && site_guard.is_none_or(|range| {
                    range
                        .contexts
                        .as_ref()
                        .is_some_and(|contexts| contexts.contains(context))
                })
        })
}

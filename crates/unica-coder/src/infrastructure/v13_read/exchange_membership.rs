//! Признак `inExchangePlan` у элементов ветвей видов: входит ли объект
//! хотя бы в один план обмена набора (#3217).
//!
//! Состав всех планов собирается тем же чтением `Ext/Content.xml`, что и
//! `Relation` плана, и сохраняется между вызовами, пока не изменился ни один
//! файл, из которых он собран: страницы ветвей разных видов его не повторяют. Конструктору запросов
//! признак приходит в тех же страницах ветвей, которые он уже читает для
//! дерева источников, — без отдельного обхода составов планов.

use super::*;
use unica_metadata::identifier::is_1c_identifier;
use unica_source_contract::read::FileStamp;

/// Виды, изменения которых записывает состав плана обмена: только они
/// получают признак.
const EXCHANGE_CONTENT_KINDS: &[NodeKind] = &[
    NodeKind::Catalog,
    NodeKind::Document,
    NodeKind::Constant,
    NodeKind::InformationRegister,
    NodeKind::AccumulationRegister,
    NodeKind::ChartOfCharacteristicTypes,
    NodeKind::BusinessProcess,
    NodeKind::Task,
    NodeKind::ChartOfAccounts,
    NodeKind::ChartOfCalculationTypes,
    NodeKind::AccountingRegister,
    NodeKind::CalculationRegister,
];

const PROPERTY: &str = "inExchangePlan";

/// Регистрации, из которых уже построена страница вида.
pub(super) enum ExchangePlanRegistrations {
    Bounded(Arc<Value>),
    Streamed(Arc<RegistrationIndex>),
}

impl ExchangePlanRegistrations {
    fn configuration_stamp(&self, artifacts: &RegistrationCache) -> Option<FileStamp> {
        match self {
            Self::Bounded(payload) => artifacts.configuration_stamp_for_payload(payload),
            Self::Streamed(index) => artifacts.configuration_stamp_for_index(index),
        }
    }

    fn plan_names(
        &self,
        checkpoint: &dyn Fn() -> Result<(), ViewError>,
    ) -> Result<Vec<String>, ViewError> {
        let kind = NodeKind::ExchangePlan.as_str();
        match self {
            Self::Bounded(payload) => Ok(payload["registeredObjects"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| item["kind"].as_str() == Some(kind))
                .filter_map(|item| item["name"].as_str().map(str::to_owned))
                .collect()),
            Self::Streamed(index) => index.names_for_kind(kind, checkpoint),
        }
    }
}

/// Объединённый состав планов обмена одного набора исходников.
#[derive(Debug, Default)]
pub(crate) struct ExchangeMembership {
    /// `Вид.Имя` в нижнем регистре: имена метаданных регистр не различают.
    members: BTreeSet<String>,
    /// Планы, чей состав прочитан не целиком: отсутствие объекта в
    /// прочитанном не доказывает, что он не входит ни в один план.
    unread: Vec<String>,
}

impl ExchangeMembership {
    fn key(kind: &str, name: &str) -> String {
        format!("{kind}.{name}").to_lowercase()
    }

    /// `Some(true)` — входит в прочитанный состав; `Some(false)` — составы
    /// всех планов прочитаны и объекта в них нет; `None` — неизвестно.
    fn includes(&self, kind: &str, name: &str) -> Option<bool> {
        if self.members.contains(&Self::key(kind, name)) {
            Some(true)
        } else if self.unread.is_empty() {
            Some(false)
        } else {
            None
        }
    }
}

impl LogicalViewReadAuthority<'_> {
    /// Дополняет страницу ветви вида признаком состава. Ветвь вида, который
    /// в состав плана не входит, и наборы, кроме конфигурации, не меняются:
    /// состав плана расширения несёт только добавления к заимствованному.
    pub(super) fn with_exchange_membership(
        &self,
        projected: NodeViewData,
        branch_kind: NodeKind,
        registrations: &ExchangePlanRegistrations,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError> {
        if !EXCHANGE_CONTENT_KINDS.contains(&branch_kind)
            || self.read.source_set_kind() != SourceSetKind::Configuration
        {
            return Ok(projected);
        }
        let NodeViewData::Collection(collection) = projected else {
            return Ok(projected);
        };
        let membership = self.exchange_membership(registrations, admitted)?;
        let (node, mut items) = collection.into_parts();
        for item in &mut items {
            let Some(name) = item.get("title").and_then(Value::as_str) else {
                continue;
            };
            if let Some(included) = membership.includes(branch_kind.as_str(), name) {
                item["props"][PROPERTY] = Value::Bool(included);
            }
        }
        let limits = membership
            .unread
            .iter()
            .map(|reason| format!("{PROPERTY} не известен: {reason}"))
            .collect();
        Ok(NodeViewData::Collection(CollectionView::new(
            node.with_additional_limits(limits),
            items,
        )))
    }

    fn exchange_membership(
        &self,
        registrations: &ExchangePlanRegistrations,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Arc<ExchangeMembership>, ViewError> {
        let key = RevisionCacheKey {
            source_set_identity: admitted.source_set_identity.clone(),
            revision: admitted.revision.clone(),
        };
        if let Some(membership) = self
            .exchange_memberships
            .lock()
            .map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::CachePoisoned,
                    "exchange membership cache is poisoned",
                )
            })?
            .get(&key)
        {
            return Ok(Arc::clone(membership));
        }
        // Новый стамп с диска мог бы относиться уже к другому списку планов.
        let configuration_stamp = registrations.configuration_stamp(self.read.artifacts());
        let membership = match configuration_stamp
            .and_then(|stamp| self.read.retained_exchange_membership(stamp))
        {
            Some(membership) => membership,
            None => {
                let mut membership = ExchangeMembership::default();
                let mut plans = registrations.plan_names(&|| self.read_checkpoint())?;
                plans.retain(|plan| {
                    if is_1c_identifier(plan) {
                        return true;
                    }
                    membership.unread.push(format!(
                        "план ExchangePlan.{plan} не прочитан: недопустимое имя"
                    ));
                    false
                });
                let stamps = configuration_stamp
                    .and_then(|stamp| self.read.exchange_membership_stamps(stamp, &plans));
                self.read_exchange_membership(&plans, admitted, &mut membership)?;
                let membership = Arc::new(membership);
                if let Some(stamps) = stamps {
                    self.read
                        .retain_exchange_membership(stamps, Arc::clone(&membership));
                }
                membership
            }
        };
        self.exchange_memberships
            .lock()
            .map_err(|_| {
                ViewError::detailed(
                    RefusalDetail::CachePoisoned,
                    "exchange membership cache is poisoned",
                )
            })?
            .insert(key, Arc::clone(&membership));
        Ok(membership)
    }

    fn read_exchange_membership(
        &self,
        plans: &[String],
        admitted: &ViewSourceSnapshot,
        membership: &mut ExchangeMembership,
    ) -> Result<(), ViewError> {
        self.read_checkpoint()?;
        for plan in plans {
            let owner = MetadataAddress::parse_logical(&format!(
                "{}.{plan}",
                NodeKind::ExchangePlan.as_str()
            ))
            .map_err(|error| {
                ViewError::detailed(RefusalDetail::SourceUnreadable, error.to_string())
            })?;
            // Повреждённое описание плана оставляет признак неизвестным, но
            // не отнимает у ветви её объекты; отмена и срок — свои отказы.
            match self.verify_registered_owner(&owner, admitted) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.code(),
                        RefusalCode::Cancelled | RefusalCode::DeadlineExceeded
                    ) =>
                {
                    return Err(error)
                }
                Err(error) => {
                    membership
                        .unread
                        .push(format!("план {} не прочитан: {error}", owner.as_str()));
                    continue;
                }
            }
            match self
                .read
                .exchange_plan_content(plan, &mut || self.read_checkpoint())?
            {
                Ok(Some(content)) => {
                    for item in content.items {
                        let mut parts = item.target.as_str().splitn(3, '.');
                        if let (Some(kind), Some(name)) = (parts.next(), parts.next()) {
                            membership
                                .members
                                .insert(ExchangeMembership::key(kind, name));
                        }
                    }
                    if let Some(error) = content.errors.first() {
                        membership.unread.push(format!(
                            "состав {} прочитан не целиком: {error}",
                            owner.as_str()
                        ));
                    }
                }
                // Платформа не пишет `Content.xml` пустого состава.
                Ok(None) => {}
                Err(error) => membership
                    .unread
                    .push(format!("состав {} не прочитан: {error}", owner.as_str())),
            }
        }
        Ok(())
    }
}

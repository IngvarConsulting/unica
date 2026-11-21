//! Признак `inExchangePlan` у элементов ветвей видов: входит ли объект
//! хотя бы в один план обмена набора (#3217).
//!
//! Состав всех планов собирается тем же чтением `Ext/Content.xml`, что и
//! `Relation` плана, и сохраняется между вызовами, пока не изменился ни один
//! файл, из которых он собран: страницы ветвей разных видов его не повторяют. Конструктору запросов
//! признак приходит в тех же страницах ветвей, которые он уже читает для
//! дерева источников, — без отдельного обхода составов планов.

use super::*;

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
        let membership = self.exchange_membership(admitted)?;
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
        let membership = match self.read.retained_exchange_membership() {
            Some(membership) => membership,
            None => {
                let plans = self.registered_kind_names(NodeKind::ExchangePlan, admitted)?;
                // Стампы снимаются до чтения: изменение во время сборки
                // делает сохранённое недействительным.
                let stamps = self.read.exchange_membership_stamps(&plans);
                let membership = Arc::new(self.build_exchange_membership(&plans, admitted)?);
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

    fn build_exchange_membership(
        &self,
        plans: &[String],
        admitted: &ViewSourceSnapshot,
    ) -> Result<ExchangeMembership, ViewError> {
        let mut membership = ExchangeMembership::default();
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
        Ok(membership)
    }

    /// Имена зарегистрированных объектов вида `kind` — по потоковому
    /// индексу регистраций или по разобранному корню, как у ветви вида.
    fn registered_kind_names(
        &self,
        kind: NodeKind,
        admitted: &ViewSourceSnapshot,
    ) -> Result<Vec<String>, ViewError> {
        if self.read.configuration_xml_requires_streaming() {
            let index = self.streamed_configuration_registration_index(admitted)?;
            return index.names_for_kind(kind.as_str(), &|| self.read_checkpoint());
        }
        let payload = self.configuration_payload(admitted)?;
        Ok(payload
            .get("registeredObjects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item.get("kind").and_then(Value::as_str) == Some(kind.as_str()))
            .filter_map(|item| item.get("name").and_then(Value::as_str))
            .map(str::to_string)
            .collect())
    }
}

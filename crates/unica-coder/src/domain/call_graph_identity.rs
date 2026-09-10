//! Перевод логического адреса метода в личность узла графа и обратно.
//!
//! Анализатор называет метод не адресом Unica, а своей личностью, и форм у неё
//! четыре. Замер 10.09.2026 на `bsl-analyzer 0.2.67`:
//!
//! | Где метод | Адрес Unica | Личность анализатора |
//! |---|---|---|
//! | общий модуль | `main:CommonModule.Общий.Method.Утилита` | `method/common/Общий/Утилита` |
//! | модуль объекта | `main:Catalog.Валюты.Module.Object.Method.ПриЗаписи` | `method/object/Catalog/Валюты/ПриЗаписи` |
//! | модуль менеджера | `main:Catalog.Валюты.Module.Manager.Method.Одноимённый` | `method/manager/Catalog/Валюты/Одноимённый` |
//! | модуль формы | `main:Catalog.Валюты.Form.Форма.Module.Form.Method.ПриОткрытии` | `method/file/…/Module.bsl::ПриОткрытии` |
//!
//! Вторым сегментом идёт роль модуля в нижнем регистре. Роли, которых замер не
//! видел, **не угадываются**: перевод отказывает и называет роль. Догадка здесь
//! уже обходилась дорого — квалифицированное имя `Модуль.Метод` анализатор не
//! разрешает вовсе, и мост, построенный на догадке, молча получал бы пустоту.
//!
//! Четвёртая форма содержит путь к файлу. Наружу путь не отдаётся — решение
//! «путь не появляется в `view` и `search`» в силе, — поэтому перевод берёт его
//! у вызывающего, который уже разрешил адрес в файл, а не строит сам.

use crate::domain::address::{NodeKind, QualifiedAddress};

/// Личность узла графа вызовов.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallGraphIdentity {
    /// Метод модуля, адресуемого логически: общий модуль, модуль объекта,
    /// модуль менеджера.
    Logical {
        role: CallGraphModuleRole,
        /// Вид владельца у модуля объекта и менеджера; у общего модуля его нет.
        owner_kind: Option<String>,
        owner: String,
        method: String,
    },
    /// Метод модуля, который анализатор называет файлом: форма, команда.
    File { path: String, method: String },
}

/// Роль модуля в личности анализатора. Закрытый набор: роль, которой замер не
/// видел, переводом не обслуживается.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallGraphModuleRole {
    Common,
    Object,
    Manager,
}

impl CallGraphModuleRole {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Object => "object",
            Self::Manager => "manager",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "common" => Some(Self::Common),
            "object" => Some(Self::Object),
            "manager" => Some(Self::Manager),
            _ => None,
        }
    }

    /// Роль модуля по написанию в адресе Unica.
    fn from_address_role(value: &str) -> Option<Self> {
        match value {
            "Object" => Some(Self::Object),
            "Manager" => Some(Self::Manager),
            _ => None,
        }
    }

    const fn address_role(self) -> Option<&'static str> {
        match self {
            Self::Common => None,
            Self::Object => Some("Object"),
            Self::Manager => Some("Manager"),
        }
    }
}

/// Почему перевод не состоялся. Каждый случай назван: молчаливая пустота здесь
/// хуже отказа, потому что читатель примет её за «вызовов нет».
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallGraphIdentityError {
    /// Адрес не называет метод.
    NotAMethod,
    /// Модуль живёт в файле, а путь вызывающий не дал.
    PathRequired,
    /// Роль модуля замером не покрыта.
    UnsupportedRole(String),
    /// Личность не разбирается: форма незнакома.
    Malformed(String),
}

impl CallGraphIdentity {
    /// Собрать личность из логического адреса метода.
    ///
    /// `path` — физический путь модуля, уже разрешённый вызывающим. Он нужен
    /// только формам и командам; для остальных ролей не запрашивается.
    pub(crate) fn from_address(
        address: &QualifiedAddress,
        path: Option<&str>,
    ) -> Result<Self, CallGraphIdentityError> {
        let segments = address.segments();
        let Some(method) = segments.last() else {
            return Err(CallGraphIdentityError::NotAMethod);
        };
        if method.kind() != NodeKind::Method {
            return Err(CallGraphIdentityError::NotAMethod);
        }
        let Some(method_name) = method.name() else {
            return Err(CallGraphIdentityError::NotAMethod);
        };
        let head = &segments[..segments.len() - 1];
        match head {
            // Общий модуль — сам себе модуль: сегмента `Module` у него нет.
            [owner] if owner.kind() == NodeKind::CommonModule => {
                let Some(owner_name) = owner.name() else {
                    return Err(CallGraphIdentityError::NotAMethod);
                };
                Ok(Self::Logical {
                    role: CallGraphModuleRole::Common,
                    owner_kind: None,
                    owner: owner_name.to_string(),
                    method: method_name.to_string(),
                })
            }
            [owner, module] if module.kind() == NodeKind::Module => {
                let Some(role) = module
                    .name()
                    .and_then(CallGraphModuleRole::from_address_role)
                else {
                    return Err(CallGraphIdentityError::UnsupportedRole(
                        module.name().unwrap_or("none").to_string(),
                    ));
                };
                let Some(owner_name) = owner.name() else {
                    return Err(CallGraphIdentityError::NotAMethod);
                };
                Ok(Self::Logical {
                    role,
                    owner_kind: Some(owner.kind().as_str().to_string()),
                    owner: owner_name.to_string(),
                    method: method_name.to_string(),
                })
            }
            // Модуль формы и команды анализатор называет файлом.
            _ => {
                let Some(path) = path else {
                    return Err(CallGraphIdentityError::PathRequired);
                };
                Ok(Self::File {
                    path: path.to_string(),
                    method: method_name.to_string(),
                })
            }
        }
    }

    /// Личность в том написании, которое принимает анализатор.
    pub(crate) fn to_analyzer_id(&self) -> String {
        match self {
            Self::Logical {
                role,
                owner_kind,
                owner,
                method,
            } => match owner_kind {
                Some(kind) => format!("method/{}/{kind}/{owner}/{method}", role.as_str()),
                None => format!("method/{}/{owner}/{method}", role.as_str()),
            },
            Self::File { path, method } => format!("method/file/{path}::{method}"),
        }
    }

    /// Разобрать личность, которую анализатор вернул в странице ветви.
    pub(crate) fn parse_analyzer_id(id: &str) -> Result<Self, CallGraphIdentityError> {
        let Some(rest) = id.strip_prefix("method/") else {
            return Err(CallGraphIdentityError::Malformed(id.to_string()));
        };
        if let Some(rest) = rest.strip_prefix("file/") {
            let Some((path, method)) = rest.split_once("::") else {
                return Err(CallGraphIdentityError::Malformed(id.to_string()));
            };
            return Ok(Self::File {
                path: path.to_string(),
                method: method.to_string(),
            });
        }
        let parts = rest.split('/').collect::<Vec<_>>();
        let Some(role) = parts
            .first()
            .and_then(|role| CallGraphModuleRole::parse(role))
        else {
            return Err(CallGraphIdentityError::UnsupportedRole(
                parts.first().copied().unwrap_or_default().to_string(),
            ));
        };
        match (role, parts.as_slice()) {
            (CallGraphModuleRole::Common, [_, owner, method]) => Ok(Self::Logical {
                role,
                owner_kind: None,
                owner: (*owner).to_string(),
                method: (*method).to_string(),
            }),
            (_, [_, kind, owner, method]) => Ok(Self::Logical {
                role,
                owner_kind: Some((*kind).to_string()),
                owner: (*owner).to_string(),
                method: (*method).to_string(),
            }),
            _ => Err(CallGraphIdentityError::Malformed(id.to_string())),
        }
    }

    /// Логический адрес метода в названном наборе исходников.
    ///
    /// `address_for_path` переводит путь в адрес и нужен только личности,
    /// названной файлом: путь наружу не уходит, значит и в ответе ветви его
    /// быть не может.
    pub(crate) fn to_address(
        &self,
        source_set: &str,
        address_for_path: impl FnOnce(&str) -> Option<QualifiedAddress>,
    ) -> Result<QualifiedAddress, CallGraphIdentityError> {
        let raw = match self {
            Self::Logical {
                role,
                owner_kind,
                owner,
                method,
            } => match (role.address_role(), owner_kind) {
                (None, _) => format!("{source_set}:CommonModule.{owner}.Method.{method}"),
                (Some(address_role), Some(kind)) => {
                    format!("{source_set}:{kind}.{owner}.Module.{address_role}.Method.{method}")
                }
                (Some(_), None) => {
                    return Err(CallGraphIdentityError::Malformed(self.to_analyzer_id()))
                }
            },
            Self::File { path, method } => {
                let Some(module) = address_for_path(path) else {
                    return Err(CallGraphIdentityError::PathRequired);
                };
                format!("{module}.Method.{method}")
            }
        };
        QualifiedAddress::parse(&raw)
            .map_err(|error| CallGraphIdentityError::Malformed(format!("{raw}: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::{CallGraphIdentity, CallGraphIdentityError, CallGraphModuleRole};
    use crate::domain::address::QualifiedAddress;

    fn address(raw: &str) -> QualifiedAddress {
        QualifiedAddress::parse(raw).expect("измеренный адрес разбирается")
    }

    /// Соответствия взяты из замера, а не из догадки.
    ///
    /// Фикстура `tests/fixtures/bsl_analyzer/graph-identities-0.2.67.json`
    /// держит те же пары, снятые с живого анализатора: общий модуль, модуль
    /// объекта, модуль менеджера и модуль, названный файлом.
    #[test]
    fn the_four_measured_identities_translate_both_ways() {
        let measured = [
            (
                "main:CommonModule.Общий.Method.Утилита",
                "method/common/Общий/Утилита",
            ),
            (
                "main:Catalog.Валюты.Module.Object.Method.ПриЗаписи",
                "method/object/Catalog/Валюты/ПриЗаписи",
            ),
            (
                "main:Catalog.Валюты.Module.Manager.Method.Одноимённый",
                "method/manager/Catalog/Валюты/Одноимённый",
            ),
        ];
        for (raw, expected) in measured {
            let identity = CallGraphIdentity::from_address(&address(raw), None)
                .unwrap_or_else(|error| panic!("{raw}: {error:?}"));
            assert_eq!(identity.to_analyzer_id(), expected, "{raw}");
            // Обратный перевод возвращает тот же адрес: личность и адрес —
            // два написания одного предмета, а не два разных предмета.
            let parsed = CallGraphIdentity::parse_analyzer_id(expected)
                .unwrap_or_else(|error| panic!("{expected}: {error:?}"));
            assert_eq!(parsed, identity, "{expected}");
            assert_eq!(
                parsed
                    .to_address("main", |_| None)
                    .unwrap_or_else(|error| panic!("{expected}: {error:?}"))
                    .to_string(),
                raw
            );
        }
    }

    /// Модуль формы анализатор называет файлом, и путь берётся у вызывающего.
    ///
    /// Наружу путь не уходит: решение «путь не появляется в `view` и `search`»
    /// в силе, поэтому обратный перевод требует резолвера, а без него
    /// отказывает названным случаем, а не подставляет путь в ответ.
    #[test]
    fn the_file_identity_needs_the_resolver_and_never_leaks_the_path() {
        let form_method = address("main:Catalog.Валюты.Form.Форма.Module.Form.Method.ПриОткрытии");
        assert_eq!(
            CallGraphIdentity::from_address(&form_method, None),
            Err(CallGraphIdentityError::PathRequired),
            "без пути личность формы не собирается"
        );
        let identity = CallGraphIdentity::from_address(
            &form_method,
            Some("Catalogs/Валюты/Forms/Форма/Ext/Form/Module.bsl"),
        )
        .expect("с путём собирается");
        assert_eq!(
            identity.to_analyzer_id(),
            "method/file/Catalogs/Валюты/Forms/Форма/Ext/Form/Module.bsl::ПриОткрытии"
        );

        let parsed = CallGraphIdentity::parse_analyzer_id(&identity.to_analyzer_id())
            .expect("личность файла разбирается");
        assert_eq!(
            parsed.to_address("main", |_| None),
            Err(CallGraphIdentityError::PathRequired),
            "без резолвера адрес не выдумывается"
        );
        assert_eq!(
            parsed
                .to_address("main", |path| {
                    assert_eq!(path, "Catalogs/Валюты/Forms/Форма/Ext/Form/Module.bsl");
                    Some(address("main:Catalog.Валюты.Form.Форма.Module.Form"))
                })
                .expect("с резолвером адрес собирается")
                .to_string(),
            "main:Catalog.Валюты.Form.Форма.Module.Form.Method.ПриОткрытии"
        );
    }

    /// Роль, которой замер не видел, не угадывается.
    ///
    /// У регистров есть модуль набора записей, у перечислений — менеджера
    /// значений; как анализатор называет их, не мерено. Перевод отказывает и
    /// называет роль, потому что догадка здесь уже обходилась молчаливой
    /// пустотой.
    #[test]
    fn an_unmeasured_role_refuses_instead_of_guessing() {
        assert_eq!(
            CallGraphIdentity::from_address(
                &address("main:InformationRegister.Курсы.Module.RecordSet.Method.ПриЗаписи"),
                None
            ),
            Err(CallGraphIdentityError::UnsupportedRole(
                "RecordSet".to_string()
            ))
        );
        assert_eq!(
            CallGraphIdentity::parse_analyzer_id("method/recordset/InformationRegister/Курсы/X"),
            Err(CallGraphIdentityError::UnsupportedRole(
                "recordset".to_string()
            ))
        );
        assert_eq!(
            CallGraphIdentity::from_address(&address("main:CommonModule.Общий"), None),
            Err(CallGraphIdentityError::NotAMethod)
        );
        assert!(matches!(
            CallGraphIdentity::parse_analyzer_id("module/common/Общий"),
            Err(CallGraphIdentityError::Malformed(_))
        ));
        assert_eq!(CallGraphModuleRole::Common.as_str(), "common");
    }
}

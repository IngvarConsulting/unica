/// Заимствование объекта расширением, как его пишет платформа.
///
/// `extends` — `ExtendedConfigurationObject`, UUID объекта родителя как есть.
/// `overrides` — список свойств из `xr:PropertyState`, а не флаг: платформа
/// перечисляет перекрытые свойства поимённо, и заимствованный объект без
/// перекрытий `PropertyState` не несёт вовсе.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetaBorrowing {
    pub extends: Option<String>,
    pub overrides: Vec<String>,
}

use serde_json::{Map, Value};

/// Read-only argument view retained for inspection operations and legacy
/// writer unit fixtures. Production mutations receive closed core commands
/// and opaque source bindings instead.
pub(crate) trait ArgumentAccess {
    fn get(&self, key: &str) -> Option<&Value>;

    fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    fn keys<'a>(&'a self) -> Box<dyn Iterator<Item = &'a str> + 'a>;
}

impl ArgumentAccess for Map<String, Value> {
    fn get(&self, key: &str) -> Option<&Value> {
        Map::get(self, key)
    }

    fn keys<'a>(&'a self) -> Box<dyn Iterator<Item = &'a str> + 'a> {
        Box::new(Map::keys(self).map(String::as_str))
    }
}

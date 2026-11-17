//! Diagnostic-only URL rendering; request URLs retain their authentication data.
use url::Url;

pub(crate) fn url(raw: &str) -> String {
    let mut url = match Url::parse(raw) {
        Ok(url) if url.host_str().is_some() => url,
        _ => return "<invalid URL>".to_owned(),
    };
    if url.username().is_empty() && url.password().is_none() {
        return raw.to_owned();
    }
    if url.set_username("").is_err() || url.set_password(None).is_err() {
        return "<invalid URL>".to_owned();
    }
    url.into()
}

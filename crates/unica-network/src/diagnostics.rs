//! Представление адреса в диагностике, не меняющее адрес запроса.

pub(crate) fn url(raw: &str) -> String {
    let Ok(mut url) = url::Url::parse(raw) else {
        return "<invalid URL>".to_owned();
    };
    if !url.has_host() {
        return "<invalid URL>".to_owned();
    }
    if (!url.username().is_empty() || url.password().is_some())
        && (url.set_username("").is_err() || url.set_password(None).is_err())
    {
        return "<invalid URL>".to_owned();
    }
    url.into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostic_url_hides_only_userinfo_without_changing_the_request() {
        let raw = "https://User2566:Pass2566@example.test/path?public=value#section";
        let request = url::Url::parse(raw).unwrap();
        assert_eq!(
            super::url(request.as_str()),
            "https://example.test/path?public=value#section"
        );
        assert_eq!(request.username(), "User2566");
        assert_eq!(request.password(), Some("Pass2566"));
        assert_eq!(request.as_str(), raw);
        for invalid in [
            "http://User2566:Pass2566@example.test:bad/path",
            "User2566:Pass2566@example.test/path",
        ] {
            assert_eq!(super::url(invalid), "<invalid URL>");
        }
    }
}

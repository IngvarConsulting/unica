use std::{fmt, num::NonZeroU16};

/// The exact HTTP origin shared by the debugger and a thin client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DebugServerUrl(String);

impl DebugServerUrl {
    /// Require the canonical spelling sent back by v8-runner, including port 80.
    /// In particular, a URL parser must never rewrite the caller's chosen host.
    pub fn parse(raw: &str) -> Option<Self> {
        let authority = raw.strip_prefix("http://")?;
        let (host, port) = authority.rsplit_once(':')?;
        let port = port.parse::<NonZeroU16>().ok()?;
        let host = match url::Host::parse(host).ok()? {
            url::Host::Ipv4(ip) => ip.to_string(),
            url::Host::Ipv6(ip) => format!("[{ip}]"),
            url::Host::Domain(name) => {
                if name.len() > 253 || !name.split('.').all(dns_label) {
                    return None;
                }
                name
            }
        };
        let canonical = format!("http://{host}:{port}");
        (raw == canonical).then_some(Self(canonical))
    }
}

fn dns_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

impl fmt::Display for DebugServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::DebugServerUrl;

    #[test]
    fn debug_server_accepts_the_exact_origin_the_client_will_use() {
        for url in [
            "http://127.0.0.1:1",
            "http://192.0.2.5:1561",
            "http://dbgs-1.example.test:80",
            "http://[2001:db8::7]:65535",
        ] {
            assert_eq!(DebugServerUrl::parse(url).unwrap().to_string(), url);
        }
    }

    #[test]
    fn debug_server_rejects_ambiguous_or_extended_origins() {
        for url in [
            "https://test:80",
            "http://test",
            "http://test:0",
            "http://test:65536",
            "http://test:080",
            "http://test:80/",
            "http://test:80/a",
            "http://test:80?x",
            "http://test:80#x",
            "http://user@test:80",
            "http://test\\redirect:80",
            "http://test:80\n",
            "http://test\0:80",
            "http://::1:80",
            "http://0x7f000001:80",
            "http://127.1:80",
            "http://010.0.0.1:80",
            "http://%31%32%37.0.0.1:80",
            "http://db..test:80",
            "http://-dbgs.test:80",
        ] {
            assert_eq!(DebugServerUrl::parse(url), None, "{url:?}");
        }
    }
}

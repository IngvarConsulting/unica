use std::fmt;

use serde_json::Value;
use unica_source_contract::debug_source::DebugServerUrl;

#[derive(Debug)]
pub(crate) enum DebugServerError {
    Invalid(&'static str),
    Unavailable(reqwest::Error),
}

impl DebugServerError {
    pub fn id(&self) -> u32 {
        match self {
            Self::Invalid(_) => 32601,
            Self::Unavailable(_) => 32602,
        }
    }
}

impl fmt::Display for DebugServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Unavailable(_) => f.write_str("the connection to the selected dbgs failed"),
        }
    }
}

impl std::error::Error for DebugServerError {}

pub(crate) fn server_from_cli(args: &[String]) -> Result<Option<DebugServerUrl>, &'static str> {
    match args {
        [] => Ok(None),
        [flag, value] if flag == "--debug-server-url" => DebugServerUrl::parse(value)
            .map(Some)
            .ok_or("invalid --debug-server-url; expected http://<host>:<port>"),
        _ => Err("usage: unica dap [--debug-server-url http://<host>:<port>]"),
    }
}

pub(crate) fn attach_server(
    args: &Value,
    selected: Option<&DebugServerUrl>,
) -> Result<DebugServerUrl, DebugServerError> {
    use DebugServerError::Invalid;

    if let Some(raw) = args.get("debugServerUrl") {
        if args.get("debugServerHost").is_some() || args.get("debugServerPort").is_some() {
            return Err(Invalid(
                "debugServerUrl conflicts with debugServerHost and debugServerPort",
            ));
        }
        let url = raw.as_str().and_then(DebugServerUrl::parse).ok_or(Invalid(
            "invalid debugServerUrl; expected http://<host>:<port>",
        ))?;
        if Some(&url) != selected {
            return Err(Invalid(
                "debugServerUrl must match the adapter's --debug-server-url",
            ));
        }
        return Ok(url);
    }
    if selected.is_some() {
        return Err(Invalid(
            "attach must confirm --debug-server-url with debugServerUrl",
        ));
    }

    if args.get("debugServerHost").is_some_and(|host| {
        !host.is_null() && !matches!(host.as_str(), Some("127.0.0.1" | "localhost"))
    }) {
        return Err(Invalid("debugServerHost must be localhost or 127.0.0.1"));
    }
    let port = match args.get("debugServerPort").filter(|value| !value.is_null()) {
        Some(port) => port
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| *n != 0)
            .ok_or(Invalid(
                "debugServerPort must be an integer from 1 to 65535",
            ))?,
        None => 1550,
    };
    DebugServerUrl::parse(&format!("http://127.0.0.1:{port}"))
        .ok_or(Invalid("invalid local debug server URL"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cli_selects_one_explicit_server() {
        let flag = "--debug-server-url".to_owned();
        let url = "http://dbgs.example.test:19550".to_owned();
        assert!(server_from_cli(&[]).unwrap().is_none());
        let pinned = server_from_cli(&[flag.clone(), url.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(pinned.to_string(), url);
        for args in [
            vec![flag.clone()],
            vec![url.clone()],
            vec![flag.clone(), "https://dbgs.example.test:19550".into()],
            vec![flag.clone(), url.clone(), flag, url],
        ] {
            assert!(server_from_cli(&args).is_err());
        }
    }

    #[test]
    fn explicit_attach_confirms_the_endpoint_selected_by_the_editor() {
        let url = "http://dbgs.example.test:19550";
        let pinned = DebugServerUrl::parse(url).unwrap();
        assert_eq!(
            attach_server(&json!({"debugServerUrl": url}), Some(&pinned))
                .unwrap()
                .to_string(),
            url
        );
        for args in [
            json!({}),
            json!({"debugServerPort": 19550}),
            json!({"debugServerUrl": "http://dbgs.example.test:80"}),
            json!({"debugServerUrl": "http://other.example.test:19550"}),
            json!({"debugServerUrl": url, "debugServerPort": 19550}),
            json!({"debugServerUrl": url, "debugServerHost": "dbgs.example.test"}),
            json!({"debugServerUrl": null}),
        ] {
            assert!(attach_server(&args, Some(&pinned)).is_err());
        }
        assert!(attach_server(&json!({"debugServerUrl": url}), None).is_err());
    }

    #[test]
    fn legacy_attach_stays_on_loopback_and_rejects_invalid_addresses() {
        assert_eq!(
            attach_server(&json!({}), None).unwrap().to_string(),
            "http://127.0.0.1:1550"
        );
        for host in ["127.0.0.1", "localhost"] {
            assert_eq!(
                attach_server(
                    &json!({"debugServerHost":host, "debugServerPort":19550}),
                    None
                )
                .unwrap()
                .to_string(),
                "http://127.0.0.1:19550"
            );
        }
        for args in [
            json!({"debugServerHost": "dbgs.example.test"}),
            json!({"debugServerHost": 42}),
            json!({"debugServerPort": "19550"}),
            json!({"debugServerPort": 0}),
            json!({"debugServerPort": 65536}),
        ] {
            assert!(attach_server(&args, None).is_err());
        }
    }
}

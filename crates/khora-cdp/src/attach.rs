//! Joining a Chrome that khora did not launch (`khora attach`).
//!
//! The browser is the user's: they started it with `--remote-debugging-port`,
//! so khora has no PID to watch and no process to stop. Everything here goes
//! through Chrome's HTTP debugging endpoint instead — `/json/version` for the
//! browser websocket, `/json/list` for the tabs, and a successful
//! `/json/version` as the liveness signal.
//!
//! The HTTP is a hand-rolled blocking `GET` over `std::net`: the endpoint is a
//! loopback server that answers `HTTP/1.0` with a `Content-Length`, and
//! `load_and_verify` (the other caller) is synchronous.

use khora_core::error::{KhoraError, KhoraResult};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// Default remote-debugging port, Chrome's own convention.
pub const DEFAULT_PORT: u16 = 9222;

/// Bound on each HTTP probe. The endpoint is normally local, so a miss this
/// long means it is not coming back.
const HTTP_TIMEOUT: Duration = Duration::from_secs(2);

/// Where `attach` was told to find the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A full browser websocket URL (`ws://host:port/devtools/browser/<id>`).
    Ws(String),
    /// A `host:port` whose `/json/version` yields the websocket URL.
    HostPort(String),
}

/// Parse the `attach` endpoint argument: `ws://...`, `http://host:port`,
/// `host:port`, or a bare port number (taken as localhost).
pub fn parse_endpoint(input: &str) -> KhoraResult<Endpoint> {
    let s = input.trim();
    let bad = || {
        KhoraError::EndpointUnreachable(format!(
            "invalid endpoint '{input}' (expected a port, host:port, or ws:// URL)"
        ))
    };
    if s.starts_with("ws://") || s.starts_with("wss://") {
        return if host_port_of_ws(s).is_some() {
            Ok(Endpoint::Ws(s.to_string()))
        } else {
            Err(bad())
        };
    }
    let rest = s.strip_prefix("http://").unwrap_or(s).trim_end_matches('/');
    if rest.is_empty() || rest.contains('/') {
        return Err(bad());
    }
    if let Ok(port) = rest.parse::<u16>() {
        return Ok(Endpoint::HostPort(format!("127.0.0.1:{port}")));
    }
    match rest.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok() => {
            Ok(Endpoint::HostPort(rest.to_string()))
        }
        _ => Err(bad()),
    }
}

/// The `host:port` of a websocket URL, which is also where Chrome serves its
/// HTTP debugging endpoint.
pub fn host_port_of_ws(ws_url: &str) -> Option<String> {
    let rest = ws_url
        .strip_prefix("ws://")
        .or_else(|| ws_url.strip_prefix("wss://"))?;
    let authority = rest.split('/').next()?;
    let (host, port) = authority.rsplit_once(':')?;
    (!host.is_empty() && port.parse::<u16>().is_ok()).then(|| authority.to_string())
}

/// Blocking `GET http://<host_port><path>`, returning the body of a 200.
fn http_get(host_port: &str, path: &str) -> KhoraResult<String> {
    let unreachable = |why: String| KhoraError::EndpointUnreachable(format!("{host_port}: {why}"));
    let addrs = host_port
        .to_socket_addrs()
        .map_err(|e| unreachable(e.to_string()))?;
    let mut last = String::from("no addresses");
    let mut stream = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, HTTP_TIMEOUT) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = e.to_string(),
        }
    }
    let mut stream = stream.ok_or_else(|| unreachable(last))?;
    let _ = stream.set_read_timeout(Some(HTTP_TIMEOUT));
    let _ = stream.set_write_timeout(Some(HTTP_TIMEOUT));
    // HTTP/1.1: Chrome's server answers an HTTP/1.0 request with nothing. It
    // may also keep the socket open despite `Connection: close`, so the body is
    // read by `Content-Length` rather than to EOF.
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| unreachable(e.to_string()))?;
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    let (head, body) = loop {
        let n = stream
            .read(&mut chunk)
            .map_err(|e| unreachable(e.to_string()))?;
        raw.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&raw).to_string();
        if let Some((head, body)) = text.split_once("\r\n\r\n") {
            let want = head.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().ok())?
            });
            if n == 0 || want.is_none_or(|w| raw.len() - head.len() - 4 >= w) {
                break (head.to_string(), body.to_string());
            }
        } else if n == 0 {
            return Err(unreachable("malformed HTTP response".into()));
        }
    };
    let chunked = head.lines().any(|l| {
        l.split_once(':').is_some_and(|(k, v)| {
            k.eq_ignore_ascii_case("transfer-encoding")
                && v.to_ascii_lowercase().contains("chunked")
        })
    });
    if chunked
        && !head
            .lines()
            .any(|l| l.to_ascii_lowercase().starts_with("content-length"))
    {
        return Err(unreachable(
            "response uses chunked transfer encoding, which khora's client does not support".into(),
        ));
    }
    let status = head.lines().next().unwrap_or_default();
    if !status.split_whitespace().nth(1).is_some_and(|c| c == "200") {
        return Err(unreachable(format!("{path} answered '{status}'")));
    }
    Ok(body.to_string())
}

/// Resolve an endpoint to the browser websocket URL.
pub fn resolve_ws_url(endpoint: &Endpoint) -> KhoraResult<String> {
    match endpoint {
        Endpoint::Ws(url) => Ok(url.clone()),
        Endpoint::HostPort(host_port) => {
            let body = http_get(host_port, "/json/version")?;
            let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                KhoraError::EndpointUnreachable(format!(
                    "{host_port}: /json/version is not JSON: {e}"
                ))
            })?;
            v["webSocketDebuggerUrl"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| {
                    KhoraError::EndpointUnreachable(format!(
                        "{host_port}: /json/version has no webSocketDebuggerUrl"
                    ))
                })
        }
    }
}

/// Is the debugging endpoint behind `ws_url` still answering? This is an
/// attached session's liveness check, standing in for the PID check.
pub fn endpoint_alive(ws_url: &str) -> bool {
    host_port_of_ws(ws_url).is_some_and(|hp| http_get(&hp, "/json/version").is_ok())
}

/// Page-type targets (tabs) in Chrome's `/json/list` order, which is most
/// recently active first.
pub fn list_page_ids(ws_url: &str) -> KhoraResult<Vec<String>> {
    let host_port = host_port_of_ws(ws_url)
        .ok_or_else(|| KhoraError::EndpointUnreachable(format!("bad websocket URL: {ws_url}")))?;
    let body = http_get(&host_port, "/json/list")?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        KhoraError::EndpointUnreachable(format!("{host_port}: /json/list is not JSON: {e}"))
    })?;
    Ok(v.as_array()
        .map(|a| {
            a.iter()
                .filter(|t| t["type"] == "page")
                .filter_map(|t| t["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Serve `responses` (path prefix, status line, body) on a loopback port,
    /// one connection per entry, and return the port.
    fn serve(routes: Vec<(&'static str, &'static str, String)>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 1024];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, body) = routes
                    .iter()
                    .find(|(p, _, _)| *p == path)
                    .map(|(_, s, b)| (*s, b.clone()))
                    .unwrap_or(("404 Not Found", String::new()));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length:{}\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        port
    }

    #[test]
    fn parse_bare_port() {
        assert_eq!(
            parse_endpoint("9333").unwrap(),
            Endpoint::HostPort("127.0.0.1:9333".into())
        );
    }

    #[test]
    fn parse_host_port_and_http() {
        assert_eq!(
            parse_endpoint("localhost:9222").unwrap(),
            Endpoint::HostPort("localhost:9222".into())
        );
        assert_eq!(
            parse_endpoint("http://10.0.0.5:9222/").unwrap(),
            Endpoint::HostPort("10.0.0.5:9222".into())
        );
    }

    #[test]
    fn parse_ws_url_kept_verbatim() {
        let ws = "ws://127.0.0.1:9222/devtools/browser/abc";
        assert_eq!(parse_endpoint(ws).unwrap(), Endpoint::Ws(ws.into()));
    }

    #[test]
    fn parse_rejects_garbage() {
        for bad in [
            "",
            "localhost",
            "host:notaport",
            "http://h:1/json",
            "ws://nohostport",
        ] {
            assert!(
                matches!(parse_endpoint(bad), Err(KhoraError::EndpointUnreachable(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn host_port_of_ws_extracts_authority() {
        assert_eq!(
            host_port_of_ws("ws://127.0.0.1:9222/devtools/browser/x").as_deref(),
            Some("127.0.0.1:9222")
        );
        assert_eq!(host_port_of_ws("http://127.0.0.1:9222"), None);
    }

    #[test]
    fn resolve_reads_websocket_url_from_json_version() {
        let port = serve(vec![(
            "/json/version",
            "200 OK",
            r#"{"Browser":"Chrome","webSocketDebuggerUrl":"ws://127.0.0.1:1/devtools/browser/z"}"#
                .into(),
        )]);
        let ep = Endpoint::HostPort(format!("127.0.0.1:{port}"));
        assert_eq!(
            resolve_ws_url(&ep).unwrap(),
            "ws://127.0.0.1:1/devtools/browser/z"
        );
    }

    #[test]
    fn resolve_unreachable_port_is_endpoint_error() {
        // Bind then drop to get a port nothing listens on.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = resolve_ws_url(&Endpoint::HostPort(format!("127.0.0.1:{port}"))).unwrap_err();
        assert!(matches!(err, KhoraError::EndpointUnreachable(_)));
        assert_eq!(err.exit_code(), 5);
    }

    #[test]
    fn endpoint_alive_tracks_json_version() {
        let port = serve(vec![("/json/version", "200 OK", "{}".into())]);
        assert!(endpoint_alive(&format!(
            "ws://127.0.0.1:{port}/devtools/browser/a"
        )));
        let dead = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(!endpoint_alive(&format!(
            "ws://127.0.0.1:{dead}/devtools/browser/a"
        )));
    }

    #[test]
    fn list_page_ids_keeps_pages_in_order() {
        let port = serve(vec![(
            "/json/list",
            "200 OK",
            r#"[{"id":"b","type":"page"},{"id":"w","type":"service_worker"},{"id":"a","type":"page"}]"#
                .into(),
        )]);
        let ids = list_page_ids(&format!("ws://127.0.0.1:{port}/devtools/browser/q")).unwrap();
        assert_eq!(ids, vec!["b", "a"]);
    }
}

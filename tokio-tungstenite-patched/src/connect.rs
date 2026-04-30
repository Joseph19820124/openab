//! Connection helper.
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use tungstenite::{
    error::{Error, UrlError},
    handshake::client::{Request, Response},
    protocol::WebSocketConfig,
};

use crate::{domain, stream::MaybeTlsStream, Connector, IntoClientRequest, WebSocketStream};

/// Connect to a given URL.
pub async fn connect_async<R>(
    request: R,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    connect_async_with_config(request, None, false).await
}

/// The same as `connect_async()` but the one can specify a websocket configuration.
/// Please refer to `connect_async()` for more details. `disable_nagle` specifies if
/// the Nagle's algorithm must be disabled, i.e. `set_nodelay(true)`. If you don't know
/// what the Nagle's algorithm is, better leave it set to `false`.
pub async fn connect_async_with_config<R>(
    request: R,
    config: Option<WebSocketConfig>,
    disable_nagle: bool,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    connect(request.into_client_request()?, config, disable_nagle, None).await
}

/// The same as `connect_async()` but the one can specify a websocket configuration,
/// and a TLS connector to use. Please refer to `connect_async()` for more details.
/// `disable_nagle` specifies if the Nagle's algorithm must be disabled, i.e.
/// `set_nodelay(true)`. If you don't know what the Nagle's algorithm is, better
/// leave it to `false`.
#[cfg(any(feature = "native-tls", feature = "__rustls-tls"))]
pub async fn connect_async_tls_with_config<R>(
    request: R,
    config: Option<WebSocketConfig>,
    disable_nagle: bool,
    connector: Option<Connector>,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error>
where
    R: IntoClientRequest + Unpin,
{
    connect(request.into_client_request()?, config, disable_nagle, connector).await
}

/// Detect an HTTP(S) proxy from environment variables.
/// Returns `Some((proxy_host, proxy_port))` if a usable HTTP proxy is found.
fn detect_http_proxy() -> Option<(String, u16)> {
    for var in &["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        if let Ok(val) = std::env::var(var) {
            if val.is_empty() {
                continue;
            }
            // Support both "http://host:port" and "host:port"
            let url_str = if val.starts_with("http://") || val.starts_with("https://") {
                val.clone()
            } else if val.starts_with("socks") {
                // Skip SOCKS-only entries; try next var
                continue;
            } else {
                format!("http://{val}")
            };
            // Minimal URL parsing without adding a url dependency
            if let Some(authority) = url_str.strip_prefix("http://").or_else(|| url_str.strip_prefix("https://")) {
                let authority = authority.trim_end_matches('/');
                let (host, port) = if let Some(idx) = authority.rfind(':') {
                    let port_str = &authority[idx + 1..];
                    if let Ok(p) = port_str.parse::<u16>() {
                        (authority[..idx].to_string(), p)
                    } else {
                        (authority.to_string(), 1080)
                    }
                } else {
                    (authority.to_string(), 1080)
                };
                // Check NO_PROXY — not implemented for simplicity; the caller is
                // connecting to external hosts (Discord, Slack, etc.)
                log::debug!("using HTTP CONNECT proxy {}:{} (from {var})", host, port);
                return Some((host, port));
            }
        }
    }
    None
}

/// Establish a TCP tunnel through an HTTP proxy using the CONNECT method.
async fn connect_via_http_proxy(
    proxy_host: &str,
    proxy_port: u16,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream, std::io::Error> {
    let proxy_addr = format!("{proxy_host}:{proxy_port}");
    let mut socket = TcpStream::connect(&proxy_addr).await?;

    let connect_req = format!(
        "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\n\r\n"
    );
    socket.write_all(connect_req.as_bytes()).await?;

    // Read the proxy response (expecting "HTTP/1.x 200 ...")
    let mut buf = vec![0u8; 4096];
    let mut total = 0usize;
    loop {
        let n = socket.read(&mut buf[total..]).await?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "proxy closed connection before completing CONNECT handshake",
            ));
        }
        total += n;
        // Look for the end of the HTTP response headers
        if let Some(pos) = find_header_end(&buf[..total]) {
            let response_line = String::from_utf8_lossy(&buf[..pos]);
            if !response_line.contains("200") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("HTTP CONNECT proxy rejected: {}", response_line.lines().next().unwrap_or("")),
                ));
            }
            log::debug!("HTTP CONNECT tunnel established to {target_host}:{target_port}");
            return Ok(socket);
        }
        if total >= buf.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "HTTP CONNECT response too large",
            ));
        }
    }
}

/// Find the `\r\n\r\n` boundary in HTTP headers.
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

async fn connect(
    request: Request,
    config: Option<WebSocketConfig>,
    disable_nagle: bool,
    connector: Option<Connector>,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, Response), Error> {
    let domain = domain(&request)?;
    let port = request
        .uri()
        .port_u16()
        .or_else(|| match request.uri().scheme_str() {
            Some("wss") => Some(443),
            Some("ws") => Some(80),
            _ => None,
        })
        .ok_or(Error::Url(UrlError::UnsupportedUrlScheme))?;

    let socket = if let Some((proxy_host, proxy_port)) = detect_http_proxy() {
        connect_via_http_proxy(&proxy_host, proxy_port, &domain, port)
            .await
            .map_err(Error::Io)?
    } else {
        let addr = format!("{domain}:{port}");
        TcpStream::connect(addr).await.map_err(Error::Io)?
    };

    if disable_nagle {
        socket.set_nodelay(true)?;
    }

    crate::tls::client_async_tls_with_config(request, socket, config, connector).await
}

//! Transport abstraction for ACP-style JSON-RPC.
//!
//! The same event contract can travel over a native stdio pipe or over a
//! socket used by a Wasm bridge. This module only describes and validates
//! endpoints; it never connects, spawns processes, or performs network I/O.
//!
//! Validation is offline and never echoes addresses or credentials: error
//! messages use fixed text so secrets cannot leak through [`TransportError`].

/// How an ACP-style endpoint is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// A child process speaking JSON-RPC over stdin and stdout.
    Stdio,
    /// A socket endpoint (`ws`/`wss`/`http`/`https`) used by a bridge.
    Socket,
}

/// A validated endpoint description for ACP-style JSON-RPC.
///
/// Implementors must be sendable across threads; validation is offline only
/// and performs no connecting or process spawning.
pub trait Transport: Send {
    /// Which mechanism this endpoint uses.
    fn kind(&self) -> TransportKind;
    /// Non-sensitive hint for logs or UI, without credentials.
    fn address_hint(&self) -> Option<String>;
    /// Check endpoint shape without connecting or spawning.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the endpoint shape is invalid.
    fn validate(&self) -> Result<(), TransportError>;
}

/// Transport validation failure without sensitive data.
///
/// Messages are fixed strings; they never echo programs, URLs, or tokens.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum TransportError {
    /// The endpoint address shape is invalid.
    #[error("invalid transport address: {0}")]
    InvalidAddress(&'static str),
    /// This transport kind is not supported in the current context.
    #[error("unsupported transport: {0}")]
    Unsupported(&'static str),
    /// The transport is already closed.
    #[error("transport is closed")]
    Closed,
}

/// Stdio endpoint: an explicit program plus arguments.
///
/// No shell parsing is involved; the program must be non-empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdioTransport {
    /// Explicit executable path or name; never a shell string.
    pub program: String,
    /// Arguments passed directly to the executable.
    pub args: Vec<String>,
}

impl StdioTransport {
    /// Create a stdio endpoint with an explicit program and arguments.
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }
}

impl Transport for StdioTransport {
    /// Report [`TransportKind::Stdio`].
    fn kind(&self) -> TransportKind {
        TransportKind::Stdio
    }

    /// The program name when non-empty, without arguments.
    fn address_hint(&self) -> Option<String> {
        let program = self.program.trim();
        if program.is_empty() {
            None
        } else {
            Some(program.to_owned())
        }
    }

    /// Require a non-empty program; arguments need no validation.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::InvalidAddress`] when the program is empty.
    fn validate(&self) -> Result<(), TransportError> {
        if self.program.trim().is_empty() {
            return Err(TransportError::InvalidAddress(
                "stdio program must not be empty",
            ));
        }
        Ok(())
    }
}

/// Socket endpoint: a URL string validated offline, never dialed.
///
/// Accepted schemes are `ws`, `wss`, `http`, and `https` with a non-empty
/// host. Validation checks shape only and never connects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketTransport {
    /// Endpoint URL; validated for shape only, never connected.
    pub url: String,
}

impl SocketTransport {
    /// Create a socket endpoint; call [`Transport::validate`] to check shape.
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

impl Transport for SocketTransport {
    /// Report [`TransportKind::Socket`].
    fn kind(&self) -> TransportKind {
        TransportKind::Socket
    }

    /// The URL when non-empty; callers must redact userinfo before display
    /// when the URL may carry credentials.
    fn address_hint(&self) -> Option<String> {
        if self.url.trim().is_empty() {
            None
        } else {
            Some(self.url.clone())
        }
    }

    /// Validate scheme and host presence without connecting.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::InvalidAddress`] when the scheme is not
    /// `ws`/`wss`/`http`/`https` or when no host is present.
    fn validate(&self) -> Result<(), TransportError> {
        validate_socket_url(&self.url)
    }
}

/// Check socket URL shape without connecting or echoing the URL in errors.
fn validate_socket_url(url: &str) -> Result<(), TransportError> {
    if url.trim().is_empty() {
        return Err(TransportError::InvalidAddress(
            "socket url must not be empty",
        ));
    }
    if url != url.trim() || url.chars().any(char::is_whitespace) {
        return Err(TransportError::InvalidAddress(
            "socket url must not contain whitespace",
        ));
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(TransportError::InvalidAddress(
            "socket url must include a scheme",
        ));
    };
    match scheme.to_ascii_lowercase().as_str() {
        "ws" | "wss" | "http" | "https" => {}
        _ => {
            return Err(TransportError::InvalidAddress(
                "socket url scheme must be ws, wss, http, or https",
            ));
        }
    }
    if rest.is_empty() {
        return Err(TransportError::InvalidAddress(
            "socket url must include a host",
        ));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err(TransportError::InvalidAddress(
            "socket url must include a host",
        ));
    }
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        let Some(end) = bracketed.find(']') else {
            return Err(TransportError::InvalidAddress(
                "socket url must include a host",
            ));
        };
        &bracketed[..end]
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        return Err(TransportError::InvalidAddress(
            "socket url must include a host",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn transports_are_send() {
        assert_send::<StdioTransport>();
        assert_send::<SocketTransport>();
        assert_send::<TransportKind>();
        assert_send::<TransportError>();
    }

    #[test]
    fn stdio_accepts_explicit_program() {
        let transport = StdioTransport::new("agent", vec!["--flag".into()]);
        assert_eq!(transport.kind(), TransportKind::Stdio);
        assert_eq!(transport.address_hint().as_deref(), Some("agent"));
        transport.validate().unwrap();
    }

    #[test]
    fn stdio_rejects_empty_program() {
        for program in ["", "   ", "\t\n"] {
            let transport = StdioTransport::new(program, Vec::new());
            assert_eq!(transport.address_hint(), None);
            assert_eq!(
                transport.validate().unwrap_err(),
                TransportError::InvalidAddress("stdio program must not be empty")
            );
        }
    }

    #[test]
    fn socket_accepts_supported_schemes_with_host() {
        for url in [
            "ws://bridge.local/session",
            "wss://bridge.example.com:443/session",
            "http://localhost:8080/bridge",
            "https://example.com",
            "ws://127.0.0.1:9000",
            "wss://[::1]:9000/session",
            "https://example.com/path?query=1#fragment",
        ] {
            let transport = SocketTransport::new(url);
            assert_eq!(transport.kind(), TransportKind::Socket);
            assert_eq!(transport.address_hint().as_deref(), Some(url));
            transport.validate().unwrap();
        }
    }

    #[test]
    fn socket_scheme_match_is_case_insensitive() {
        SocketTransport::new("WSS://bridge.example.com/session")
            .validate()
            .unwrap();
    }

    #[test]
    fn socket_rejects_missing_or_unsupported_address() {
        for url in [
            "",
            "   ",
            "bridge.example.com/session",
            "ftp://bridge.example.com/session",
            "file:///tmp/socket",
            "ws://",
            "wss://",
            "https://",
            "ws:///path-only",
            "https:///path-only",
            "ws://?query-only",
            "ws://#fragment-only",
            "ws://:8080/no-host",
            "ws://bridge.example.com:8080/pa th",
            " ws://bridge.example.com",
            "ws://bridge.example.com ",
        ] {
            let transport = SocketTransport::new(url);
            assert!(
                transport.validate().is_err(),
                "expected rejection for {url:?}"
            );
        }
    }

    #[test]
    fn error_messages_never_echo_addresses() {
        let secret = "super-secret-token-9f8e7d6c";
        let transport = SocketTransport::new(format!("ftp://bridge.example.com/{secret}"));
        let error = transport.validate().unwrap_err();
        assert_eq!(
            error,
            TransportError::InvalidAddress("socket url scheme must be ws, wss, http, or https")
        );
        let rendered = format!("{error} {error:?}");
        assert!(!rendered.contains(secret));
        assert!(!rendered.contains("ftp://"));

        let stdio_error = StdioTransport::new("", Vec::new()).validate().unwrap_err();
        assert!(!format!("{stdio_error}").contains(secret));
    }

    #[test]
    fn closed_and_unsupported_variants_stay_static() {
        let closed = TransportError::Closed;
        let unsupported = TransportError::Unsupported("socket bridge");
        assert_eq!(format!("{closed}"), "transport is closed");
        assert_eq!(
            format!("{unsupported}"),
            "unsupported transport: socket bridge"
        );
    }

    #[test]
    fn socket_hint_is_none_for_blank_address() {
        for url in ["", "   ", "\t\n"] {
            assert_eq!(SocketTransport::new(url).address_hint(), None);
        }
    }

    #[test]
    fn stdio_hint_trims_program_name() {
        let transport = StdioTransport::new("  agent  ", Vec::new());
        assert_eq!(transport.address_hint().as_deref(), Some("agent"));
    }
}

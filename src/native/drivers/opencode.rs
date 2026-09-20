//! API-key login through a native OpenCode control server.
//!
//! The driver starts `program serve` on loopback with an ephemeral port and
//! a random server password, stores the user key through the vendor auth
//! endpoint, and confirms presence through the vendor provider list. The
//! password lives only in the child environment; the user key lives only in
//! request bodies. Neither value appears in errors, logs, or debug output.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::time::{Instant, sleep, timeout};

use super::{AccountInfo, DriverError, LoginDriver, LoginState};

/// Upper bound for one control-plane reply body.
const MAX_BODY: usize = 1_048_576;

/// Vendor provider id reported as the display label on success.
const PROVIDER_ID: &str = "opencode-go";

/// Key login against an ephemeral OpenCode control server.
///
/// The server password is random per attempt and passed only through the
/// child environment. The user key is supplied by the application and sent
/// only in the vendor `PUT` body. The child is stopped on drop and on
/// cancellation.
pub struct OpenCodeKeyDriver {
    program: PathBuf,
    lead_args: Vec<String>,
    api_key: String,
    operation_timeout: Duration,
    readiness_timeout: Duration,
    helper: Option<RunningHelper>,
    cancelled: bool,
}

/// Running control server for one attempt.
struct RunningHelper {
    child: Child,
    port: u16,
    password: String,
}

impl OpenCodeKeyDriver {
    /// Create a driver for one user-supplied key.
    ///
    /// The program is the OpenCode executable; `serve --hostname 127.0.0.1
    /// --port <ephemeral>` is appended at spawn time. Fixture tests prepend
    /// interpreter arguments with [`OpenCodeKeyDriver::with_lead_args`].
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::InvalidOptions`] when the program or the key
    /// is empty.
    pub fn new(program: impl Into<PathBuf>, api_key: String) -> Result<Self, DriverError> {
        let program = program.into();
        if program.as_os_str().is_empty() {
            return Err(DriverError::InvalidOptions(
                "opencode program must not be empty",
            ));
        }
        if api_key.trim().is_empty() {
            return Err(DriverError::InvalidOptions(
                "opencode key must not be empty",
            ));
        }
        Ok(Self {
            program,
            lead_args: Vec::new(),
            api_key,
            operation_timeout: Duration::from_secs(10),
            readiness_timeout: Duration::from_secs(15),
            helper: None,
            cancelled: false,
        })
    }

    /// Prepend arguments before the appended `serve` token.
    ///
    /// Production use leaves this empty; fixture tests pass an interpreter
    /// preamble such as `-u -c <code> <mode>` here.
    pub fn with_lead_args(mut self, args: Vec<String>) -> Self {
        self.lead_args = args;
        self
    }

    /// Set the deadline for one HTTP round trip.
    pub fn with_operation_timeout(mut self, limit: Duration) -> Self {
        self.operation_timeout = limit;
        self
    }

    /// Set how long `start` waits for `/global/health` to succeed.
    pub fn with_readiness_timeout(mut self, limit: Duration) -> Self {
        self.readiness_timeout = limit;
        self
    }

    /// Whether a control server is currently running.
    pub fn has_helper(&self) -> bool {
        self.helper.is_some()
    }

    /// Stop the helper without exposing secrets.
    fn shutdown(&mut self) {
        if let Some(mut live) = self.helper.take() {
            let _ = live.child.start_kill();
        }
    }
}

impl std::fmt::Debug for OpenCodeKeyDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenCodeKeyDriver")
            .field(
                "phase",
                if self.cancelled {
                    &"cancelled"
                } else if self.helper.is_some() {
                    &"running"
                } else {
                    &"idle"
                },
            )
            .field("has_helper", &self.helper.is_some())
            .finish()
    }
}

impl Drop for OpenCodeKeyDriver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Standard base64 alphabet for basic authentication.
const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes as base64 without external crates.
fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let mut block: u32 = 0;
        for (slot, byte) in chunk.iter().enumerate() {
            block |= (*byte as u32) << (16 - 8 * slot);
        }
        let pad = 3 - chunk.len();
        for slot in 0..4 - pad {
            let sextet = ((block >> (18 - 6 * slot)) & 0x3F) as usize;
            out.push(B64_TABLE[sextet] as char);
        }
        for _ in 0..pad {
            out.push('=');
        }
    }
    out
}

/// Basic header value for the fixed `opencode` user.
fn auth_value(password: &str) -> String {
    base64_encode(format!("opencode:{password}").as_bytes())
}

/// Random ephemeral password without new dependencies.
///
/// Prefers operating-system randomness; falls back to a time-seeded mix so
/// tests on constrained hosts still get a unique value per attempt.
fn ephemeral_password() -> String {
    let mut raw = [0u8; 24];
    let mut filled = false;
    if let Ok(mut source) = std::fs::File::open("/dev/urandom") {
        use std::io::Read as _;
        if source.read_exact(&mut raw).is_ok() {
            filled = true;
        }
    }
    if !filled {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|span| span.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id() as u128;
        let mut mix = nanos ^ ((pid << 64) | 0x9e37_79b9_7f4a_7c15);
        for slot in raw.iter_mut() {
            mix ^= mix >> 29;
            mix = mix.wrapping_mul(0xbf58_476d_1ce4_e5b9);
            mix ^= mix >> 32;
            *slot = (mix & 0xFF) as u8;
        }
    }
    let mut text = String::with_capacity(raw.len() * 2);
    for byte in raw {
        text.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        text.push(char::from_digit((byte & 0x0F) as u32, 16).unwrap_or('0'));
    }
    text
}

/// Claim an ephemeral loopback port by binding, then release it.
async fn free_port(limit: Duration) -> Result<u16, DriverError> {
    let listener = timeout(limit, TcpListener::bind("127.0.0.1:0"))
        .await
        .map_err(|_| DriverError::Timeout)?
        .map_err(|_| DriverError::Spawn("control port was unavailable"))?;
    let port = listener
        .local_addr()
        .map_err(|_| DriverError::Spawn("control port was unavailable"))?
        .port();
    drop(listener);
    if port == 0 {
        return Err(DriverError::Spawn("control port was unavailable"));
    }
    Ok(port)
}

/// Spawn the control server with a cleared minimal environment.
fn launch(
    program: &PathBuf,
    lead: &[String],
    port: u16,
    password: &str,
) -> Result<Child, DriverError> {
    if program.as_os_str().is_empty() {
        return Err(DriverError::InvalidOptions(
            "opencode program must not be empty",
        ));
    }
    let home = std::env::var("HOME")
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    // Resolve bare names against the parent PATH before clearing it, so
    // fixtures work where the interpreter lives outside the minimal set.
    let resolved = super::resolve_program(program);
    let mut spawn = Command::new(&resolved);
    spawn
        .args(lead)
        .args([
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", home)
        .env("OPENCODE_SERVER_PASSWORD", password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    spawn
        .spawn()
        .map_err(|_| DriverError::Spawn("control server could not be launched"))
}

/// One hand-rolled HTTP round trip over loopback.
///
/// Uses `Connection: close` so the reply ends at EOF; the body is capped.
async fn http_call(
    method: &str,
    path: &str,
    port: u16,
    password: &str,
    body: Option<&[u8]>,
    limit: Duration,
) -> Result<(u16, Vec<u8>), DriverError> {
    let mut stream = timeout(limit, TcpStream::connect(("127.0.0.1", port)))
        .await
        .map_err(|_| DriverError::Timeout)?
        .map_err(|_| DriverError::Closed)?;
    let auth = auth_value(password);
    let length = body.map_or(0, <[u8]>::len);
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Basic {auth}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    );
    timeout(limit, stream.write_all(head.as_bytes()))
        .await
        .map_err(|_| DriverError::Timeout)?
        .map_err(|_| DriverError::Closed)?;
    if let Some(payload) = body {
        timeout(limit, stream.write_all(payload))
            .await
            .map_err(|_| DriverError::Timeout)?
            .map_err(|_| DriverError::Closed)?;
    }
    timeout(limit, stream.flush())
        .await
        .map_err(|_| DriverError::Timeout)?
        .map_err(|_| DriverError::Closed)?;
    let mut raw = Vec::new();
    let mut chunk = [0u8; 8_192];
    let outcome: Result<(), DriverError> = timeout(limit, async {
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    if raw.len() + count > MAX_BODY + 8_192 {
                        return Err(DriverError::Protocol(-32700));
                    }
                    raw.extend_from_slice(&chunk[..count]);
                }
                Err(_) => return Err(DriverError::Closed),
            }
        }
    })
    .await
    .map_err(|_| DriverError::Timeout)?;
    outcome?;
    if raw.len() > MAX_BODY + 8_192 {
        return Err(DriverError::Protocol(-32700));
    }
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status_line = head.lines().next().unwrap_or_default();
    let mut parts = status_line.split_whitespace();
    let _version = parts.next();
    let code_text = parts.next().unwrap_or_default();
    let code: u16 = code_text
        .parse()
        .map_err(|_| DriverError::Protocol(-32700))?;
    let mut payload = body.as_bytes().to_vec();
    if payload.len() > MAX_BODY {
        return Err(DriverError::Protocol(-32700));
    }
    // Prefer Content-Length when the server keeps framing exact.
    if let Some(wanted) = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        && wanted <= MAX_BODY
    {
        payload.truncate(wanted.min(payload.len()));
    }
    Ok((code, payload))
}

/// Wait until `/global/health` answers 2xx or the helper exits.
async fn await_ready(helper: &mut RunningHelper, limit: Duration) -> Result<(), DriverError> {
    if limit.is_zero() {
        return Err(DriverError::InvalidOptions(
            "driver deadlines must be positive",
        ));
    }
    let start = Instant::now();
    loop {
        if start.elapsed() >= limit {
            return Err(DriverError::Timeout);
        }
        match helper.child.try_wait() {
            Ok(Some(_)) => {
                return Err(DriverError::Spawn(
                    "control server stopped before readiness",
                ));
            }
            Ok(None) => {}
            Err(_) => {
                return Err(DriverError::Spawn(
                    "control server stopped before readiness",
                ));
            }
        }
        let attempt = (limit - start.elapsed()).min(Duration::from_secs(2));
        match http_call(
            "GET",
            "/global/health",
            helper.port,
            &helper.password,
            None,
            attempt,
        )
        .await
        {
            Ok((code, _)) if (200..300).contains(&code) => return Ok(()),
            Ok(_) => {}
            Err(DriverError::Closed) => {}
            Err(DriverError::Timeout) => {}
            Err(other) => return Err(other),
        }
        sleep(Duration::from_millis(50)).await;
    }
}

/// Whether the provider reply lists the expected vendor id.
fn is_connected(reply: &[u8]) -> Result<bool, DriverError> {
    let value: Value = serde_json::from_slice(reply).map_err(|_| DriverError::Protocol(-32700))?;
    let listed = value
        .get("connected")
        .and_then(Value::as_array)
        .ok_or(DriverError::Protocol(-32603))?;
    Ok(listed
        .iter()
        .any(|entry| entry.as_str() == Some(PROVIDER_ID)))
}

impl LoginDriver for OpenCodeKeyDriver {
    /// Start the server, store the key, and confirm presence.
    async fn start(&mut self) -> Result<LoginState, DriverError> {
        if self.cancelled {
            return Err(DriverError::Cancelled);
        }
        self.shutdown();
        if self.operation_timeout.is_zero() || self.readiness_timeout.is_zero() {
            return Err(DriverError::InvalidOptions(
                "driver deadlines must be positive",
            ));
        }
        let port = free_port(self.operation_timeout).await?;
        let password = ephemeral_password();
        let child = launch(&self.program, &self.lead_args, port, &password)?;
        let mut live = RunningHelper {
            child,
            port,
            password,
        };
        await_ready(&mut live, self.readiness_timeout)
            .await
            .inspect_err(|_| {
                let _ = live.child.start_kill();
            })?;
        let payload = serde_json::json!({"type": "api", "key": self.api_key.clone()});
        let bytes = serde_json::to_vec(&payload).map_err(|_| DriverError::Protocol(-32700))?;
        let (stored, _) = http_call(
            "PUT",
            "/auth/opencode-go",
            live.port,
            &live.password,
            Some(&bytes),
            self.operation_timeout,
        )
        .await
        .inspect_err(|_| {
            let _ = live.child.start_kill();
        })?;
        if !(200..300).contains(&stored) {
            let _ = live.child.start_kill();
            return Ok(LoginState::Failed);
        }
        let (code, reply) = http_call(
            "GET",
            "/provider",
            live.port,
            &live.password,
            None,
            self.operation_timeout,
        )
        .await
        .inspect_err(|_| {
            let _ = live.child.start_kill();
        })?;
        if !(200..300).contains(&code) {
            let _ = live.child.start_kill();
            return Err(DriverError::Protocol(code as i32));
        }
        let connected = is_connected(&reply).inspect_err(|_| {
            let _ = live.child.start_kill();
        })?;
        if !connected {
            let _ = live.child.start_kill();
            return Ok(LoginState::Failed);
        }
        self.helper = Some(live);
        Ok(LoginState::Authenticated(AccountInfo::new(Some(
            PROVIDER_ID.to_owned(),
        ))))
    }

    /// Re-read provider presence on the running server.
    async fn poll(&mut self) -> Result<LoginState, DriverError> {
        if self.cancelled {
            return Err(DriverError::Cancelled);
        }
        let live = self.helper.as_mut().ok_or(DriverError::Closed)?;
        let (code, reply) = http_call(
            "GET",
            "/provider",
            live.port,
            &live.password,
            None,
            self.operation_timeout,
        )
        .await?;
        if !(200..300).contains(&code) {
            return Err(DriverError::Protocol(code as i32));
        }
        if is_connected(&reply)? {
            Ok(LoginState::Authenticated(AccountInfo::new(Some(
                PROVIDER_ID.to_owned(),
            ))))
        } else {
            Ok(LoginState::Failed)
        }
    }

    /// Re-read provider presence as display-only state.
    async fn account(&mut self) -> Result<AccountInfo, DriverError> {
        if self.cancelled {
            return Err(DriverError::Cancelled);
        }
        let live = self.helper.as_mut().ok_or(DriverError::Closed)?;
        let (code, reply) = http_call(
            "GET",
            "/provider",
            live.port,
            &live.password,
            None,
            self.operation_timeout,
        )
        .await?;
        if !(200..300).contains(&code) {
            return Err(DriverError::Protocol(code as i32));
        }
        if is_connected(&reply)? {
            Ok(AccountInfo::new(Some(PROVIDER_ID.to_owned())))
        } else {
            Ok(AccountInfo::signed_out())
        }
    }

    /// Stop the server; later steps report cancellation.
    async fn cancel(&mut self) -> Result<(), DriverError> {
        self.cancelled = true;
        self.shutdown();
        Ok(())
    }
}

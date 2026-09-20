//! ChatGPT device-code login through a native Codex helper.
//!
//! The driver spawns the vendor `app-server` over stdio, requests a device
//! challenge, waits for browser approval, and projects the vendor presence.
//! No secret bytes are read or stored; only the user-visible URL, code, and
//! display label leave the helper.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{Instant, timeout};

use super::{AccountInfo, Challenge, DriverError, LoginDriver, LoginState};

/// Maximum accepted line length for helper frames.
const MAX_FRAME: u64 = 1_048_576;

/// Maximum buffered vendor notifications while waiting for a reply.
const MAX_QUEUED_NOTES: usize = 64;

/// Default approval host for the ChatGPT device step.
const DEFAULT_HOST: &str = "auth.openai.com";

/// Native Codex device-code handshake.
///
/// Spawns `program app-server` with a cleared environment containing only a
/// minimal `PATH` and `HOME`. The helper owns its credential files; this
/// driver only relays the challenge and the projected presence.
pub struct CodexDeviceDriver {
    program: PathBuf,
    lead_args: Vec<String>,
    allowed_hosts: Vec<String>,
    call_timeout: Duration,
    approval_deadline: Duration,
    handshake: Option<ActiveHandshake>,
    cancelled: bool,
}

/// Live helper state for one login attempt.
struct ActiveHandshake {
    child: Child,
    intake: ChildStdin,
    outtake: BufReader<ChildStdout>,
    next_id: u64,
    queued: VecDeque<Value>,
    login_id: String,
    challenge: Challenge,
}

impl CodexDeviceDriver {
    /// Create a driver with the default approval host.
    ///
    /// The program is the Codex executable; `app-server` is appended at
    /// spawn time. Extra leading arguments for fixtures can be added with
    /// [`CodexDeviceDriver::with_lead_args`].
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            lead_args: Vec::new(),
            allowed_hosts: vec![DEFAULT_HOST.to_owned()],
            call_timeout: Duration::from_secs(15),
            approval_deadline: Duration::from_secs(300),
            handshake: None,
            cancelled: false,
        }
    }

    /// Create a driver with an explicit approval-host allowlist.
    ///
    /// Every host must be a bare DNS name without scheme, path, or
    /// whitespace. An empty list rejects every challenge. Matching is
    /// case-insensitive and exact; subdomains are not implied.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::InvalidOptions`] when the program is empty,
    /// the list is empty, or any host has an unsupported shape.
    pub fn with_allowed_hosts(
        program: impl Into<PathBuf>,
        allowed: Vec<String>,
    ) -> Result<Self, DriverError> {
        if allowed.is_empty() {
            return Err(DriverError::InvalidOptions(
                "approval host allowlist must not be empty",
            ));
        }
        for host in &allowed {
            if !is_bare_host(host) {
                return Err(DriverError::InvalidOptions(
                    "approval host has an unsupported shape",
                ));
            }
        }
        let program = program.into();
        if program.as_os_str().is_empty() {
            return Err(DriverError::InvalidOptions(
                "codex program must not be empty",
            ));
        }
        Ok(Self {
            program,
            lead_args: Vec::new(),
            allowed_hosts: allowed,
            call_timeout: Duration::from_secs(15),
            approval_deadline: Duration::from_secs(300),
            handshake: None,
            cancelled: false,
        })
    }

    /// Add leading arguments before the appended `app-server` token.
    ///
    /// Production use leaves this empty; fixture tests pass an interpreter
    /// preamble such as `-u -c <code> <mode>` here.
    pub fn with_lead_args(mut self, args: Vec<String>) -> Self {
        self.lead_args = args;
        self
    }

    /// Set the per-call deadline for helper requests.
    ///
    /// Applies to the challenge request and the presence read. Polling for
    /// browser approval uses [`CodexDeviceDriver::with_approval_deadline`].
    pub fn with_call_timeout(mut self, limit: Duration) -> Self {
        self.call_timeout = limit;
        self
    }

    /// Set how long [`LoginDriver::poll`] waits for browser approval.
    pub fn with_approval_deadline(mut self, limit: Duration) -> Self {
        self.approval_deadline = limit;
        self
    }

    /// Current approval-host allowlist.
    pub fn allowed_hosts(&self) -> &[String] {
        &self.allowed_hosts
    }

    /// Whether a helper is currently running.
    pub fn has_helper(&self) -> bool {
        self.handshake.is_some()
    }

    /// Spawn the helper with a cleared minimal environment.
    fn spawn_helper(&self) -> Result<ActiveHandshake, DriverError> {
        if self.program.as_os_str().is_empty() {
            return Err(DriverError::InvalidOptions(
                "codex program must not be empty",
            ));
        }
        if self.call_timeout.is_zero() || self.approval_deadline.is_zero() {
            return Err(DriverError::InvalidOptions(
                "driver deadlines must be positive",
            ));
        }
        let home = std::env::var("HOME")
            .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
        // Resolve bare names against the parent PATH before clearing it, so
        // fixtures work where the interpreter lives outside the minimal set.
        let resolved = super::resolve_program(&self.program);
        let mut spawn = Command::new(&resolved);
        spawn
            .args(&self.lead_args)
            .arg("app-server")
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = spawn
            .spawn()
            .map_err(|_| DriverError::Spawn("codex helper could not be launched"))?;
        let intake = child
            .stdin
            .take()
            .ok_or(DriverError::Spawn("codex helper input was unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or(DriverError::Spawn("codex helper output was unavailable"))?;
        Ok(ActiveHandshake {
            child,
            intake,
            outtake: BufReader::new(output),
            next_id: 0,
            queued: VecDeque::new(),
            login_id: String::new(),
            challenge: Challenge::new(String::new(), String::new()),
        })
    }

    /// Stop the helper without reporting secrets.
    fn shutdown(&mut self) {
        if let Some(mut live) = self.handshake.take() {
            let _ = live.child.start_kill();
        }
    }

    /// Short label for redacted debug output.
    fn phase_label(&self) -> &'static str {
        if self.cancelled {
            "cancelled"
        } else if let Some(live) = &self.handshake {
            if live.login_id.is_empty() {
                "starting"
            } else {
                "awaiting-approval"
            }
        } else {
            "idle"
        }
    }
}

impl std::fmt::Debug for CodexDeviceDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodexDeviceDriver")
            .field("phase", &self.phase_label())
            .field("has_helper", &self.handshake.is_some())
            .finish()
    }
}

impl Drop for CodexDeviceDriver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Whether a string is a bare host without scheme or path.
fn is_bare_host(candidate: &str) -> bool {
    if candidate.is_empty() || candidate.chars().any(char::is_whitespace) {
        return false;
    }
    if candidate.contains("://") || candidate.contains('/') {
        return false;
    }
    candidate
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
        && candidate.bytes().any(|byte| byte.is_ascii_alphanumeric())
}

/// Extract the lowercased host from an `https` URL.
fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.split('@').next_back().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// Read one capped line-delimited frame from the helper.
async fn read_frame(outtake: &mut BufReader<ChildStdout>) -> Result<Value, DriverError> {
    let mut buf = Vec::new();
    let taken = outtake
        .take(MAX_FRAME)
        .read_until(b'\n', &mut buf)
        .await
        .map_err(|_| DriverError::Closed)?;
    let _ = taken;
    if buf.is_empty() {
        return Err(DriverError::Closed);
    }
    if buf.last() != Some(&b'\n') {
        return Err(DriverError::Protocol(-32700));
    }
    serde_json::from_slice(&buf).map_err(|_| DriverError::Protocol(-32700))
}

/// Send one frame to the helper.
async fn write_frame(intake: &mut ChildStdin, frame: &Value) -> Result<(), DriverError> {
    let mut bytes = serde_json::to_vec(frame).map_err(|_| DriverError::Protocol(-32700))?;
    bytes.push(b'\n');
    intake
        .write_all(&bytes)
        .await
        .map_err(|_| DriverError::Closed)?;
    intake.flush().await.map_err(|_| DriverError::Closed)?;
    Ok(())
}

/// Call one helper method and wait for the matching reply.
async fn round_trip(
    live: &mut ActiveHandshake,
    method: &str,
    params: Value,
    limit: Duration,
) -> Result<Value, DriverError> {
    live.next_id = live.next_id.wrapping_add(1);
    let id = live.next_id;
    write_frame(
        &mut live.intake,
        &serde_json::json!({"id": id, "method": method, "params": params}),
    )
    .await?;
    timeout(limit, async {
        loop {
            let frame = read_frame(&mut live.outtake).await?;
            if frame.get("id") == Some(&Value::from(id)) {
                if let Some(err) = frame.get("error") {
                    let code = err.get("code").and_then(Value::as_i64).unwrap_or(-32603) as i32;
                    return Err(DriverError::Protocol(code));
                }
                if let Some(result) = frame.get("result") {
                    return Ok(result.clone());
                }
                return Err(DriverError::Protocol(-32603));
            }
            if frame.get("method").is_some() {
                if live.queued.len() >= MAX_QUEUED_NOTES {
                    return Err(DriverError::Protocol(-32603));
                }
                live.queued.push_back(frame);
            }
        }
    })
    .await
    .map_err(|_| DriverError::Timeout)?
}

/// Fetch one queued or fresh helper notification.
async fn next_note(live: &mut ActiveHandshake, limit: Duration) -> Result<Value, DriverError> {
    if let Some(note) = live.queued.pop_front() {
        return Ok(note);
    }
    timeout(limit, read_frame(&mut live.outtake))
        .await
        .map_err(|_| DriverError::Timeout)?
}

/// Pull one string field, accepting camelCase or snake_case keys.
fn pick_text(source: &Value, camel: &str, snake: &str) -> Option<String> {
    for key in [camel, snake] {
        if let Some(text) = source.get(key).and_then(Value::as_str) {
            return Some(text.to_owned());
        }
    }
    None
}

/// Project the login-start reply into an id plus a validated challenge.
fn project_start(reply: &Value, allowed: &[String]) -> Result<(String, Challenge), DriverError> {
    let login_id = pick_text(reply, "loginId", "login_id").ok_or(DriverError::InvalidOptions(
        "helper challenge missed its login id",
    ))?;
    let url = pick_text(reply, "verificationUrl", "verification_url").ok_or(
        DriverError::InvalidOptions("helper challenge missed its URL"),
    )?;
    let code = pick_text(reply, "userCode", "user_code").ok_or(DriverError::InvalidOptions(
        "helper challenge missed its code",
    ))?;
    if login_id.is_empty() || login_id.len() > 128 {
        return Err(DriverError::InvalidOptions(
            "helper challenge missed its login id",
        ));
    }
    let challenge = Challenge::new(url, code);
    challenge.validate()?;
    let host = host_of(&challenge.verification_url).ok_or(DriverError::InvalidOptions(
        "challenge URL must include a host",
    ))?;
    let permitted = allowed
        .iter()
        .any(|entry| entry.to_ascii_lowercase() == host);
    if !permitted {
        return Err(DriverError::InvalidOptions("challenge host is not allowed"));
    }
    Ok((login_id, challenge))
}

/// Project the account-read reply into display-only presence.
fn project_presence(reply: &Value) -> AccountInfo {
    let node = reply.get("account").unwrap_or(&Value::Null);
    let kind = node.get("type").and_then(Value::as_str).unwrap_or_default();
    if kind != "chatgpt" {
        return AccountInfo::signed_out();
    }
    let label = node
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| node.get("label").and_then(Value::as_str))
        .unwrap_or_default();
    if label.is_empty() || label.len() > 320 {
        return AccountInfo::new(None);
    }
    AccountInfo::new(Some(label.to_owned()))
}

impl LoginDriver for CodexDeviceDriver {
    /// Spawn the helper and request a ChatGPT device challenge.
    async fn start(&mut self) -> Result<LoginState, DriverError> {
        if self.cancelled {
            return Err(DriverError::Cancelled);
        }
        self.shutdown();
        let mut live = self.spawn_helper()?;
        let reply = round_trip(
            &mut live,
            "account/login/start",
            serde_json::json!({"type": "chatgptDeviceCode"}),
            self.call_timeout,
        )
        .await
        .inspect_err(|_| {
            let _ = live.child.start_kill();
        })?;
        let (login_id, challenge) =
            project_start(&reply, &self.allowed_hosts).inspect_err(|_| {
                let _ = live.child.start_kill();
            })?;
        live.login_id = login_id;
        live.challenge = challenge.clone();
        self.handshake = Some(live);
        Ok(LoginState::ChallengeRequired(challenge))
    }

    /// Wait for browser approval, then project the connected account.
    async fn poll(&mut self) -> Result<LoginState, DriverError> {
        if self.cancelled {
            return Err(DriverError::Cancelled);
        }
        let deadline = Instant::now() + self.approval_deadline;
        if self.approval_deadline.is_zero() {
            return Err(DriverError::InvalidOptions(
                "driver deadlines must be positive",
            ));
        }
        let live = self.handshake.as_mut().ok_or(DriverError::Closed)?;
        let wanted = live.login_id.clone();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(DriverError::Timeout);
            }
            let wait = remaining.min(self.call_timeout);
            let event = next_note(live, wait).await?;
            let method = event.get("method").and_then(Value::as_str).unwrap_or("");
            if method != "account/login/completed" {
                continue;
            }
            let params = event.get("params").unwrap_or(&Value::Null);
            let seen = params
                .get("loginId")
                .and_then(Value::as_str)
                .or_else(|| params.get("login_id").and_then(Value::as_str))
                .unwrap_or_default();
            if seen != wanted {
                continue;
            }
            let ok = params
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !ok {
                return Ok(LoginState::Failed);
            }
            let reply = round_trip(
                live,
                "account/read",
                serde_json::json!({"refreshToken": false}),
                self.call_timeout,
            )
            .await?;
            let info = project_presence(&reply);
            if info.account.is_none() {
                return Ok(LoginState::Failed);
            }
            return Ok(LoginState::Authenticated(info));
        }
    }

    /// Re-read vendor presence without a new device challenge.
    async fn account(&mut self) -> Result<AccountInfo, DriverError> {
        if self.cancelled {
            return Err(DriverError::Cancelled);
        }
        let live = self.handshake.as_mut().ok_or(DriverError::Closed)?;
        let reply = round_trip(
            live,
            "account/read",
            serde_json::json!({"refreshToken": false}),
            self.call_timeout,
        )
        .await?;
        Ok(project_presence(&reply))
    }

    /// Stop the helper; later steps report cancellation.
    async fn cancel(&mut self) -> Result<(), DriverError> {
        self.cancelled = true;
        self.shutdown();
        Ok(())
    }
}

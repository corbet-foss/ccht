//! Native ACP sessions driven by the upstream Apache-2.0 Rust SDK.
//!
//! The SDK owns process spawning, protocol dispatch, session routing, and process
//! cleanup. Applications own agent installation, subscription login, workspace
//! isolation, and authorization policy. Denying permission requests is not a
//! sandbox: an agent can have tools that do not ask the client for permission.

mod client;
pub mod drivers;
mod env;
pub mod pool;
mod session;
pub mod well_known;

#[cfg(test)]
mod tests;

pub use client::NativeClient;
pub use env::EnvProfile;
pub use pool::{PoolEvent, SessionKey, SessionPool};
pub use session::{NativeSession, SessionHandle};

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::acp::{McpServer, RequestPermissionOutcome};

/// An explicitly installed ACP executable; no implicit shell or installer is used.
#[derive(Clone)]
pub struct AgentCommand {
    /// Executable path or name resolved by the operating system.
    pub program: PathBuf,
    /// Arguments passed directly to the executable.
    pub args: Vec<String>,
    /// Additional child environment. Values are excluded from debug output.
    pub env: BTreeMap<String, String>,
}

impl AgentCommand {
    /// Select an existing executable without arguments or environment overrides.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    /// Set arguments without shell parsing.
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Start in a directory through an installed Linux or macOS `env` launcher.
    ///
    /// This changes the process startup directory, independently of ACP session
    /// `cwd`. The SDK still owns the process and its cleanup. Linux resolves
    /// `env` through the caller's PATH and requires GNU `--chdir`; macOS uses
    /// `/usr/bin/env -C`. An unsupported launcher fails before executing the
    /// agent. No shell parsing is involved. Windows has no implementation here.
    pub fn with_working_directory(self, cwd: impl AsRef<std::path::Path>) -> Result<Self> {
        if !cfg!(any(target_os = "linux", target_os = "macos")) {
            return Err(NativeError::Unsupported(
                "a startup-directory launcher on this platform",
            ));
        }
        let cwd = cwd.as_ref();
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(NativeError::InvalidOptions(
                "startup cwd must be an existing absolute directory",
            ));
        }
        let directory = cwd
            .to_str()
            .ok_or(NativeError::InvalidOptions("startup cwd must be UTF-8"))?;
        let program = self
            .program
            .to_str()
            .ok_or(NativeError::InvalidOptions("executable must be UTF-8"))?;
        let (launcher, mut args) = if cfg!(target_os = "macos") {
            (
                "/usr/bin/env",
                vec!["-C".into(), directory.into(), "--".into(), program.into()],
            )
        } else {
            (
                "env",
                vec![format!("--chdir={directory}"), "--".into(), program.into()],
            )
        };
        args.extend(self.args);
        Ok(Self {
            program: launcher.into(),
            args,
            env: self.env,
        })
    }

    /// Describe this spawn command as a stdio transport and validate it.
    ///
    /// Types and validation only; the spawn itself stays with
    /// [`NativeClient::connect`]. An empty program is rejected here, matching
    /// the connect-time `InvalidOptions` for the same shape, so valid
    /// commands see no behavior change. Sockets never appear on this path:
    /// native connectors spawn a local agent speaking ACP over stdio.
    ///
    /// # Errors
    ///
    /// Returns [`crate::TransportError`] when the program is empty or the
    /// address has an unsupported shape.
    pub fn validate_stdio(&self) -> std::result::Result<(), crate::TransportError> {
        use crate::{StdioTransport, Transport as _};
        let program = self.program.to_string_lossy().into_owned();
        StdioTransport::new(program, self.args.clone()).validate()
    }

    /// Seed missing entries from the parent process environment, then filter
    /// through `profile` in place.
    ///
    /// [`NativeClient::connect`] inherits the host environment and only blanks
    /// known API credential variables on its own; seeding every parent
    /// variable first and then blanking non-allowlisted entries enforces the
    /// allowlist (for example `GITHUB_TOKEN`, which the SDK alone would
    /// inherit, becomes `""`). Explicit entries already on the command win
    /// over the parent snapshot.
    pub fn seal_parent_env(&mut self, profile: EnvProfile) {
        for (key, value) in std::env::vars() {
            self.env.entry(key).or_insert(value);
        }
        profile.apply_to_command(self);
    }

    /// Builder variant of [`AgentCommand::seal_parent_env`].
    #[must_use]
    pub fn with_sealed_parent_env(mut self, profile: EnvProfile) -> Self {
        self.seal_parent_env(profile);
        self
    }
}

impl std::fmt::Debug for AgentCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentCommand")
            .field("program", &self.program)
            .field("argument_count", &self.args.len())
            .field("environment_keys", &self.env.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// How requests that reach the client are handled; this does not sandbox an agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PermissionPolicy {
    /// Cancel every permission request without granting authority.
    #[default]
    Deny,
    /// Emit a permission event and await an explicit, valid response.
    Ask,
}

/// A permission response uses ACP's exact selected-option or cancellation type.
pub type PermissionDecision = RequestPermissionOutcome;

/// Resource limits and permission policy for a native connection.
#[derive(Debug, Clone)]
pub struct NativeOptions {
    /// Limit for initialization, session creation/restoration, and configuration.
    pub operation_timeout: Duration,
    /// Maximum duration of a prompt, including permission waits.
    pub prompt_timeout: Duration,
    /// Maximum time an unanswered permission request may remain pending.
    pub permission_timeout: Duration,
    /// Grace period for cancellation and SDK connection shutdown.
    pub shutdown_timeout: Duration,
    /// Maximum queued application events per session.
    pub event_capacity: usize,
    /// Policy applied to every incoming permission request.
    pub permissions: PermissionPolicy,
}

impl Default for NativeOptions {
    fn default() -> Self {
        Self {
            operation_timeout: Duration::from_secs(30),
            prompt_timeout: Duration::from_secs(600),
            permission_timeout: Duration::from_secs(120),
            shutdown_timeout: Duration::from_secs(5),
            event_capacity: 256,
            permissions: PermissionPolicy::Deny,
        }
    }
}

/// Application-owned workspace and optional agent session configuration.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// Absolute existing directory supplied to the agent; not an isolation boundary.
    pub cwd: PathBuf,
    /// Exact advertised model option value, or the agent's default when absent.
    pub model: Option<String>,
    /// Agent-advertised configuration values, applied in order after model selection.
    pub configuration: Vec<(String, crate::acp::SessionConfigOptionValue)>,
    /// Application-owned MCP server descriptors passed to the agent.
    pub mcp_servers: Vec<McpServer>,
}

impl SessionOptions {
    /// Use an explicit workspace and the agent's default model without MCP servers.
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            model: None,
            configuration: Vec::new(),
            mcp_servers: Vec::new(),
        }
    }
}

/// Native errors deliberately exclude raw agent messages, stderr, and error data.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum NativeError {
    /// A Tokio runtime is required to drive the connection.
    #[error("a Tokio runtime is required for native ACP sessions")]
    RuntimeUnavailable,
    /// Configuration is incomplete or inconsistent.
    #[error("invalid native configuration: {0}")]
    InvalidOptions(&'static str),
    /// The operation exceeded its configured deadline.
    #[error("native ACP operation timed out")]
    Timeout,
    /// The connection or session has closed.
    #[error("native ACP connection or session is closed")]
    Closed,
    /// A session already has an active prompt.
    #[error("this session already has an active prompt")]
    Busy,
    /// The agent did not advertise the requested operation or content type.
    #[error("the agent does not support {0}")]
    Unsupported(&'static str),
    /// Agent-owned authentication is required; no alternate billing path is tried.
    #[error("authenticate using the selected agent's native login, then reconnect")]
    AuthenticationRequired,
    /// A protocol error code, without possibly sensitive server text or data.
    #[error("the agent returned ACP error {0}")]
    Protocol(i32),
    /// An application failed to keep up with the bounded event stream.
    #[error("session event buffer is full; the connection was closed")]
    EventBufferFull,
    /// The session's sole event receiver has already been taken.
    #[error("the session event stream has already been taken")]
    EventsTaken,
    /// No pending permission request has the supplied identifier.
    #[error("permission request is absent, expired, or already answered")]
    UnknownPermission,
    /// A selected permission option was not offered by the agent.
    #[error("the selected permission option was not offered")]
    InvalidPermissionOption,
}

impl NativeError {
    /// Stable category suitable for a product's structured error event.
    pub fn code(&self) -> &'static str {
        match self {
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::InvalidOptions(_) => "invalid_options",
            Self::Timeout => "timeout",
            Self::Closed => "closed",
            Self::Busy => "busy",
            Self::Unsupported(_) => "unsupported",
            Self::AuthenticationRequired => "authentication_required",
            Self::Protocol(_) => "protocol",
            Self::EventBufferFull => "event_buffer_full",
            Self::EventsTaken => "events_taken",
            Self::UnknownPermission => "unknown_permission",
            Self::InvalidPermissionOption => "invalid_permission_option",
        }
    }

    /// Shared login state for this failure.
    ///
    /// Only [`NativeError::AuthenticationRequired`] carries login
    /// information; every other failure maps to
    /// [`crate::AuthState::Unknown`]. This is the call-error side of the same
    /// convergence [`crate::auth_state_from_code`] provides for wire codes,
    /// so applications never compare the `"authentication_required"` literal.
    #[must_use]
    pub fn auth_state(&self) -> crate::AuthState {
        if *self == Self::AuthenticationRequired {
            crate::AuthState::Unauthenticated
        } else {
            crate::AuthState::Unknown
        }
    }
}

impl From<agent_client_protocol::Error> for NativeError {
    fn from(error: agent_client_protocol::Error) -> Self {
        if error.code == crate::acp::ErrorCode::AuthRequired {
            Self::AuthenticationRequired
        } else {
            Self::Protocol(error.code.into())
        }
    }
}

/// Result returned by native ACP operations.
pub type Result<T> = std::result::Result<T, NativeError>;

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

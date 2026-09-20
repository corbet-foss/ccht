//! Native client connection to one ACP agent process.
//!
//! The upstream SDK owns process spawning, protocol dispatch, and cleanup.
//! This module only validates options, clears ambient credential variables,
//! and negotiates stable ACP v1 before handing sessions to [`super::session`].

use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{AcpAgent, AcpAgentConfig, ActiveSession, Agent, Client, ConnectionTo};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::acp::{
    AuthMethod, AuthenticateRequest, InitializeRequest, InitializeResponse, LoadSessionRequest,
    McpServer, NewSessionRequest, ResumeSessionRequest,
};

use super::{
    AgentCommand, NativeError, NativeOptions, NativeSession, Result, SessionOptions, lock,
};

/// A connection to one explicitly selected, externally authenticated ACP agent.
///
/// Clones and active sessions share this connection. Closing the client closes
/// every session on it. No credentials are stored and no API fallback is attempted.
#[derive(Clone)]
pub struct NativeClient {
    pub(super) inner: Arc<ClientLease>,
}

pub(super) struct ClientLease {
    pub(super) state: Arc<ClientState>,
    pub(super) connection: ConnectionTo<Agent>,
    pub(super) info: InitializeResponse,
    pub(super) options: NativeOptions,
}

pub(super) struct ClientState {
    shutdown: watch::Sender<bool>,
    pub(super) closed: watch::Sender<bool>,
    failure: Mutex<Option<NativeError>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl ClientState {
    pub(super) fn abort(&self) {
        self.shutdown.send_replace(true);
        self.closed.send_replace(true);
        if let Some(task) = lock(&self.task).as_ref() {
            task.abort();
        }
    }

    pub(super) fn error(&self) -> NativeError {
        lock(&self.failure).clone().unwrap_or(NativeError::Closed)
    }

    pub(super) fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }
}

impl Drop for ClientLease {
    fn drop(&mut self) {
        self.state.abort();
    }
}

struct Connecting(Option<Arc<ClientState>>);

impl Drop for Connecting {
    fn drop(&mut self) {
        if let Some(state) = &self.0 {
            state.abort();
        }
    }
}

impl NativeClient {
    /// Spawn the configured agent through the SDK and negotiate stable ACP v1.
    ///
    /// Requires a caller-owned Tokio runtime. No terminal or filesystem client
    /// capabilities are advertised. Authentication remains with the agent.
    pub async fn connect(command: AgentCommand, options: NativeOptions) -> Result<Self> {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| NativeError::RuntimeUnavailable)?;
        if command.program.as_os_str().is_empty()
            || options.event_capacity == 0
            || options.operation_timeout.is_zero()
            || options.prompt_timeout.is_zero()
            || options.permission_timeout.is_zero()
            || options.shutdown_timeout.is_zero()
        {
            return Err(NativeError::InvalidOptions(
                "executable and positive limits are required",
            ));
        }
        if command
            .env
            .iter()
            .any(|(key, value)| api_credential_variable(key) && !value.is_empty())
        {
            return Err(NativeError::InvalidOptions(
                "API credential environment overrides are not accepted",
            ));
        }
        let mut config = AcpAgentConfig::new(command.program)
            .args(command.args)
            .envs(command.env);
        // The SDK inherits the host environment. Empty known API credential
        // variables so an ambient key cannot silently replace native agent login.
        // The agent's own saved credentials and billing settings remain its concern.
        for (key, _) in std::env::vars_os() {
            if let Some(key) = key.to_str().filter(|key| api_credential_variable(key)) {
                config = config.env(key, "");
            }
        }
        let agent = AcpAgent::new(config);
        let (ready_tx, ready_rx) = oneshot::channel();
        let (shutdown, mut shutdown_rx) = watch::channel(false);
        let (closed, _) = watch::channel(false);
        let state = Arc::new(ClientState {
            shutdown,
            closed,
            failure: Mutex::new(None),
            task: Mutex::new(None),
        });
        let mut guard = Connecting(Some(state.clone()));
        let worker_state = Arc::downgrade(&state);
        let task = runtime.spawn(async move {
            let outcome = Client
                .builder()
                .connect_with(agent, async move |connection| {
                    let info = connection
                        .send_request(
                            InitializeRequest::new(ProtocolVersion::V1).client_capabilities(
                                crate::acp::ClientCapabilities::new().session(
                                    crate::acp::ClientSessionCapabilities::new().config_options(
                                        crate::acp::SessionConfigOptionsCapabilities::new()
                                            .boolean(
                                                crate::acp::BooleanConfigOptionCapabilities::new(),
                                            ),
                                    ),
                                ),
                            ),
                        )
                        .block_task()
                        .await?;
                    if info.protocol_version != ProtocolVersion::V1 {
                        return Err(agent_client_protocol::Error::new(
                            -32600,
                            "unsupported ACP version",
                        ));
                    }
                    if ready_tx.send((connection.clone(), info)).is_err() {
                        return Ok(());
                    }
                    tokio::select! {
                        _ = shutdown_rx.changed() => {},
                        _ = connection.incoming_closed() => {},
                    }
                    Ok(())
                })
                .await;
            if let Some(state) = worker_state.upgrade() {
                if let Err(error) = outcome {
                    *lock(&state.failure) = Some(error.into());
                }
                state.closed.send_replace(true);
            }
        });
        *lock(&state.task) = Some(task);
        let (connection, info) = match timeout(options.operation_timeout, ready_rx).await {
            Ok(Ok(ready)) => ready,
            Ok(Err(_)) => {
                let mut closed = state.closed.subscribe();
                if !state.is_closed() {
                    let _ = timeout(options.shutdown_timeout, closed.changed()).await;
                }
                return Err(state.error());
            }
            Err(_) => return Err(NativeError::Timeout),
        };
        guard.0 = None;
        Ok(Self {
            inner: Arc::new(ClientLease {
                state,
                connection,
                info,
                options,
            }),
        })
    }

    /// The agent's exact negotiated capabilities, implementation, and auth methods.
    pub fn info(&self) -> &InitializeResponse {
        &self.inner.info
    }

    /// Invoke an explicitly selected agent-owned authentication method.
    ///
    /// Terminal authentication is left to the application or the agent's CLI.
    /// The method must have been advertised; none is selected automatically.
    pub async fn authenticate(&self, method_id: &str) -> Result<()> {
        match self
            .info()
            .auth_methods
            .iter()
            .find(|method| method.id().0.as_ref() == method_id)
        {
            Some(AuthMethod::Agent(_)) => {}
            Some(_) => {
                return Err(NativeError::Unsupported(
                    "terminal authentication through this client",
                ));
            }
            None => {
                return Err(NativeError::Unsupported(
                    "the selected authentication method",
                ));
            }
        }
        self.operation(
            self.inner
                .connection
                .send_request(AuthenticateRequest::new(method_id.to_owned()))
                .block_task(),
        )
        .await?;
        Ok(())
    }

    /// Create an independent conversation in an application-selected directory.
    pub async fn new_session(&self, options: SessionOptions) -> Result<NativeSession> {
        self.validate_session(&options)?;
        let request = NewSessionRequest::new(&options.cwd).mcp_servers(options.mcp_servers.clone());
        let session = self
            .operation(
                self.inner
                    .connection
                    .build_session_from(request)
                    .block_task()
                    .start_session(),
            )
            .await?;
        self.finish_session(session, options).await
    }

    /// Restore a known agent session, preserving the agent's replayed history events.
    pub async fn load_session(
        &self,
        agent_session_id: &str,
        options: SessionOptions,
    ) -> Result<NativeSession> {
        if !self.info().agent_capabilities.load_session {
            return Err(NativeError::Unsupported("session history loading"));
        }
        self.validate_session(&options)?;
        let request = LoadSessionRequest::new(agent_session_id.to_owned(), &options.cwd)
            .mcp_servers(options.mcp_servers.clone());
        let restored = self
            .operation(
                self.inner
                    .connection
                    .load_session_from(request)
                    .block_task()
                    .start_session(),
            )
            .await?;
        self.finish_session(restored.into_session(), options).await
    }

    /// Resume a known session without replay, only when the agent advertises support.
    pub async fn resume_session(
        &self,
        agent_session_id: &str,
        options: SessionOptions,
    ) -> Result<NativeSession> {
        if self
            .info()
            .agent_capabilities
            .session_capabilities
            .resume
            .is_none()
        {
            return Err(NativeError::Unsupported(
                "session resumption without replay",
            ));
        }
        self.validate_session(&options)?;
        let request = ResumeSessionRequest::new(agent_session_id.to_owned(), &options.cwd)
            .mcp_servers(options.mcp_servers.clone());
        let restored = self
            .operation(
                self.inner
                    .connection
                    .resume_session_from(request)
                    .block_task()
                    .start_session(),
            )
            .await?;
        self.finish_session(restored.into_session(), options).await
    }

    async fn finish_session(
        &self,
        session: ActiveSession<'static, Agent>,
        options: SessionOptions,
    ) -> Result<NativeSession> {
        let session = NativeSession::start(self.inner.clone(), session)?;
        let handle = session.handle();
        if let Some(model) = options.model {
            handle.set_model(&model).await?;
        }
        for (id, value) in options.configuration {
            handle.set_config_option(&id, value).await?;
        }
        Ok(session)
    }

    fn validate_session(&self, options: &SessionOptions) -> Result<()> {
        if self.inner.state.is_closed() {
            return Err(self.inner.state.error());
        }
        if !options.cwd.is_absolute() || !options.cwd.is_dir() {
            return Err(NativeError::InvalidOptions(
                "session cwd must be an existing absolute directory",
            ));
        }
        let capabilities = &self.info().agent_capabilities.mcp_capabilities;
        for server in &options.mcp_servers {
            match server {
                McpServer::Stdio(_) => {}
                McpServer::Http(_) if capabilities.http => {}
                McpServer::Sse(_) if capabilities.sse => {}
                _ => return Err(NativeError::Unsupported("the requested MCP transport")),
            }
        }
        Ok(())
    }

    async fn operation<T>(
        &self,
        future: impl std::future::Future<Output = std::result::Result<T, agent_client_protocol::Error>>,
    ) -> Result<T> {
        if self.inner.state.is_closed() {
            return Err(self.inner.state.error());
        }
        match timeout(self.inner.options.operation_timeout, future).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                if error.code == crate::acp::ErrorCode::InternalError {
                    tokio::task::yield_now().await;
                }
                if self.inner.state.is_closed() {
                    Err(self.inner.state.error())
                } else {
                    Err(error.into())
                }
            }
            Err(_) => {
                self.inner.state.abort();
                Err(NativeError::Timeout)
            }
        }
    }

    /// Close every session and wait a bounded time for the SDK to clean up its child.
    pub async fn close(&self) -> Result<()> {
        self.inner.state.shutdown.send_replace(true);
        let task = lock(&self.inner.state.task).take();
        if let Some(mut task) = task
            && timeout(self.inner.options.shutdown_timeout, &mut task)
                .await
                .is_err()
        {
            task.abort();
            self.inner.state.closed.send_replace(true);
            return Err(NativeError::Timeout);
        }
        self.inner.state.closed.send_replace(true);
        Ok(())
    }
}

fn api_credential_variable(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.ends_with("_API_KEY")
        || matches!(
            upper.as_str(),
            "ANTHROPIC_AUTH_TOKEN"
                | "AZURE_OPENAI_KEY"
                | "AWS_BEARER_TOKEN_BEDROCK"
                | "GOOGLE_APPLICATION_CREDENTIALS"
                | "AWS_ACCESS_KEY_ID"
                | "AWS_SECRET_ACCESS_KEY"
                | "AWS_SESSION_TOKEN"
        )
}

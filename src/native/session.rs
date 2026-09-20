//! Native conversation session with one ordered event stream.
//!
//! The worker owns prompt routing, permission validation, and timeout handling.
//! Applications consume events through the single receiver and control the turn
//! through [`SessionHandle`]; dropping an active prompt closes the connection.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::util::MatchDispatch;
use agent_client_protocol::{ActiveSession, Agent, Dispatch, Responder, SessionMessage};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, interval, timeout};

use crate::acp::{
    CancelNotification, CloseSessionRequest, ContentBlock, PromptRequest, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, SessionId,
    SessionNotification, StopReason,
};
use crate::{Event, Prompt, SessionConfiguration, SessionEvent};

use super::client::{ClientLease, ClientState};
use super::{NativeError, PermissionDecision, PermissionPolicy, Result, lock};

static NEXT_PERMISSION: AtomicU64 = AtomicU64::new(1);

/// A native session and its single ordered event stream.
///
/// Take the receiver once, then retain a handle while consuming it. A cloned
/// handle keeps the session alive independently of this value.
pub struct NativeSession {
    handle: SessionHandle,
    events: Option<mpsc::Receiver<SessionEvent>>,
}

/// Cloneable, thread-safe control of a native conversation.
#[derive(Clone)]
pub struct SessionHandle {
    lease: Arc<SessionLease>,
}

struct SessionLease {
    state: Arc<SessionState>,
    commands: mpsc::Sender<PromptCommand>,
    client: Arc<ClientLease>,
}

struct SessionState {
    id: SessionId,
    client: Arc<ClientState>,
    connection: agent_client_protocol::ConnectionTo<Agent>,
    turn: Mutex<TurnState>,
    activity: watch::Sender<bool>,
    closed: watch::Sender<bool>,
    failure: Mutex<Option<NativeError>>,
    events: Mutex<Option<mpsc::Sender<SessionEvent>>>,
    permissions: Mutex<BTreeMap<String, PendingPermission>>,
    configuration: Mutex<SessionConfiguration>,
    configuring: tokio::sync::Mutex<()>,
    task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
struct TurnState {
    reserved: Option<String>,
    current: Option<String>,
    cancelled: bool,
}

struct PendingPermission {
    request: RequestPermissionRequest,
    responder: Responder<RequestPermissionResponse>,
    expires: Instant,
}

struct PromptCommand {
    prompt: Prompt,
    result: oneshot::Sender<Result<StopReason>>,
}

struct PromptGuard(Option<(Arc<SessionState>, String)>);

impl Drop for PromptGuard {
    fn drop(&mut self) {
        let Some((state, request_id)) = &self.0 else {
            return;
        };
        // Release the turn lock before cleanup takes it again.
        let active = lock(&state.turn).reserved.as_ref() == Some(request_id);
        if active {
            // Abandoning a prompt cannot leave unattended agent work running.
            state.close(NativeError::Closed, true);
        }
    }
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        let active = lock(&self.state.turn).reserved.is_some();
        self.state.close(NativeError::Closed, active);
    }
}

impl SessionState {
    fn is_closed(&self) -> bool {
        *self.closed.borrow() || self.client.is_closed()
    }

    fn error(&self) -> NativeError {
        lock(&self.failure)
            .clone()
            .unwrap_or_else(|| self.client.error())
    }

    fn emit(&self, event: Event) -> Result<()> {
        if let Event::Update { update } = &event {
            lock(&self.configuration).apply(update);
        }
        let request_id = lock(&self.turn).current.clone();
        let result = lock(&self.events)
            .as_ref()
            .ok_or(NativeError::Closed)?
            .try_send(SessionEvent { request_id, event });
        match result {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.close(NativeError::EventBufferFull, true);
                Err(NativeError::EventBufferFull)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.close(NativeError::Closed, true);
                Err(NativeError::Closed)
            }
        }
    }

    fn deny_permissions(&self) {
        for (_, pending) in std::mem::take(&mut *lock(&self.permissions)) {
            let _ = pending.responder.respond(RequestPermissionResponse::new(
                RequestPermissionOutcome::Cancelled,
            ));
        }
    }

    fn close(&self, error: NativeError, stop_connection: bool) {
        if !*self.closed.borrow() {
            *lock(&self.failure) = Some(error.clone());
            let _ = self
                .connection
                .send_notification(CancelNotification::new(self.id.clone()));
            self.deny_permissions();
            let request_id = lock(&self.turn).current.clone();
            if let Some(events) = lock(&self.events).take()
                && (request_id.is_some() || error != NativeError::Closed)
            {
                let _ = events.try_send(SessionEvent {
                    request_id,
                    event: Event::Error {
                        code: error.code().into(),
                        message: error.to_string(),
                    },
                });
            }
            self.closed.send_replace(true);
            self.activity.send_replace(false);
            if let Some(task) = lock(&self.task).as_ref() {
                task.abort();
            }
        }
        if stop_connection {
            self.client.abort();
        }
    }

    fn finish(&self, outcome: &Result<StopReason>) -> Result<()> {
        self.deny_permissions();
        let event = match outcome {
            Ok(stop_reason) => Event::Completed {
                stop_reason: *stop_reason,
            },
            Err(error) => Event::Error {
                code: error.code().into(),
                message: error.to_string(),
            },
        };
        self.emit(event)?;
        *lock(&self.turn) = TurnState::default();
        self.activity.send_replace(false);
        Ok(())
    }
}

impl NativeSession {
    pub(super) fn start(
        client: Arc<ClientLease>,
        session: ActiveSession<'static, Agent>,
    ) -> Result<Self> {
        let (events_tx, events_rx) = mpsc::channel(client.options.event_capacity);
        let (commands_tx, commands_rx) = mpsc::channel(1);
        let (activity, _) = watch::channel(false);
        let (closed, _) = watch::channel(false);
        let state = Arc::new(SessionState {
            id: session.session_id().clone(),
            client: client.state.clone(),
            connection: session.connection().clone(),
            turn: Mutex::new(TurnState::default()),
            activity,
            closed,
            failure: Mutex::new(None),
            events: Mutex::new(Some(events_tx)),
            permissions: Mutex::new(BTreeMap::new()),
            configuration: Mutex::new(SessionConfiguration {
                options: session.config_options().unwrap_or_default().to_vec(),
                modes: session.modes().cloned(),
            }),
            configuring: tokio::sync::Mutex::new(()),
            task: Mutex::new(None),
        });
        let worker_state = state.clone();
        let options = client.options.clone();
        let task = tokio::spawn(async move {
            if let Err(error) =
                run_session(session, commands_rx, worker_state.clone(), options).await
            {
                worker_state.close(error, true);
            }
        });
        *lock(&state.task) = Some(task);
        Ok(Self {
            handle: SessionHandle {
                lease: Arc::new(SessionLease {
                    state,
                    commands: commands_tx,
                    client,
                }),
            },
            events: Some(events_rx),
        })
    }

    /// Clone the control handle; it can be used while the receiver is polled elsewhere.
    pub fn handle(&self) -> SessionHandle {
        self.handle.clone()
    }

    /// Take the sole stream, including any history queued during session restoration.
    pub fn take_events(&mut self) -> Result<mpsc::Receiver<SessionEvent>> {
        self.events.take().ok_or(NativeError::EventsTaken)
    }
}

impl SessionHandle {
    /// Latest agent-advertised controls, including dependent changes and notifications.
    pub fn configuration(&self) -> SessionConfiguration {
        lock(&self.lease.state.configuration).clone()
    }

    /// Select an exact model advertised by this session, without any fallback.
    pub async fn set_model(&self, model: &str) -> Result<SessionConfiguration> {
        let id = self
            .configuration()
            .model_option()
            .map(|option| option.id.to_string())
            .ok_or(NativeError::Unsupported("model selection"))?;
        self.set_config_option(&id, model.into()).await
    }

    /// Change an advertised select or boolean control and retain the full response.
    /// Unknown controls and values fail before sending a request to the agent.
    pub async fn set_config_option(
        &self,
        id: &str,
        value: crate::acp::SessionConfigOptionValue,
    ) -> Result<SessionConfiguration> {
        let state = &self.lease.state;
        let _serial = state.configuring.lock().await;
        if state.is_closed() {
            return Err(state.error());
        }
        if !self.configuration().accepts(id, &value) {
            return Err(NativeError::Unsupported("the selected configuration value"));
        }
        let request =
            crate::acp::SetSessionConfigOptionRequest::new(state.id.clone(), id.to_owned(), value);
        let response = match timeout(
            self.lease.client.options.operation_timeout,
            state.connection.send_request(request).block_task(),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                state.close(NativeError::Timeout, true);
                return Err(NativeError::Timeout);
            }
        };
        state.emit(Event::Update {
            update: crate::acp::SessionUpdate::ConfigOptionUpdate(
                crate::acp::ConfigOptionUpdate::new(response.config_options),
            ),
        })?;
        Ok(self.configuration())
    }

    /// The agent-owned session identifier, suitable for explicit load or resume.
    pub fn id(&self) -> &str {
        self.lease.state.id.0.as_ref()
    }

    /// Send one prompt and await its terminal result while events stream independently.
    ///
    /// A concurrent prompt fails with `Busy`. Dropping this future after submission
    /// closes the connection, ensuring the abandoned turn cannot keep running.
    pub async fn prompt(&self, prompt: Prompt) -> Result<StopReason> {
        let state = &self.lease.state;
        if state.is_closed() {
            return Err(state.error());
        }
        if prompt.request_id.is_empty() || prompt.content.is_empty() {
            return Err(NativeError::InvalidOptions(
                "a prompt needs a request id and content",
            ));
        }
        let capabilities = &self
            .lease
            .client
            .info
            .agent_capabilities
            .prompt_capabilities;
        for content in &prompt.content {
            match content {
                ContentBlock::Text(_) | ContentBlock::ResourceLink(_) => {}
                ContentBlock::Image(_) if capabilities.image => {}
                ContentBlock::Audio(_) if capabilities.audio => {}
                ContentBlock::Resource(_) if capabilities.embedded_context => {}
                _ => return Err(NativeError::Unsupported("the requested prompt content")),
            }
        }
        {
            let mut turn = lock(&state.turn);
            if turn.reserved.is_some() {
                return Err(NativeError::Busy);
            }
            turn.reserved = Some(prompt.request_id.clone());
            turn.cancelled = false;
            state.activity.send_replace(true);
        }
        let mut guard = PromptGuard(Some((state.clone(), prompt.request_id.clone())));
        let (result_tx, result_rx) = oneshot::channel();
        self.lease
            .commands
            .send(PromptCommand {
                prompt,
                result: result_tx,
            })
            .await
            .map_err(|_| state.error())?;
        let mut closed = state.closed.subscribe();
        let mut connection_closed = state.client.closed.subscribe();
        let result = timeout(self.lease.client.options.prompt_timeout, async {
            if state.is_closed() {
                return Err(state.error());
            }
            tokio::select! {
                result = result_rx => result.unwrap_or_else(|_| Err(state.error())),
                _ = closed.changed() => Err(state.error()),
                _ = connection_closed.changed() => Err(state.error()),
            }
        })
        .await;
        match result {
            Ok(outcome) => {
                guard.0 = None;
                outcome
            }
            Err(_) => {
                state.close(NativeError::Timeout, true);
                guard.0 = None;
                Err(NativeError::Timeout)
            }
        }
    }

    /// Request cancellation and wait a bounded time for the active turn to finish.
    ///
    /// If the agent ignores cancellation, the SDK connection is closed. This also
    /// closes other sessions sharing that agent process.
    pub async fn cancel(&self) -> Result<()> {
        let state = &self.lease.state;
        if state.is_closed() {
            return Err(state.error());
        }
        {
            let mut turn = lock(&state.turn);
            if turn.reserved.is_none() {
                return Ok(());
            }
            turn.cancelled = true;
        }
        state.deny_permissions();
        state
            .connection
            .send_notification(CancelNotification::new(state.id.clone()))?;
        let mut activity = state.activity.subscribe();
        if timeout(self.lease.client.options.shutdown_timeout, async {
            while *activity.borrow_and_update() {
                if activity.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .is_err()
        {
            state.close(NativeError::Timeout, true);
            return Err(NativeError::Timeout);
        }
        if state.is_closed() {
            Err(state.error())
        } else {
            Ok(())
        }
    }

    /// Answer one pending permission request using an option the agent offered.
    ///
    /// Invalid selections leave the request pending so the application can correct
    /// its response. Expired or previously answered requests fail explicitly.
    pub async fn respond_permission(
        &self,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<()> {
        let state = &self.lease.state;
        if state.is_closed() {
            return Err(state.error());
        }
        let pending = {
            let mut permissions = lock(&state.permissions);
            let pending = permissions
                .get(request_id)
                .ok_or(NativeError::UnknownPermission)?;
            if pending.expires <= Instant::now() || pending.responder.cancellation().is_cancelled()
            {
                return Err(NativeError::UnknownPermission);
            }
            match &decision {
                RequestPermissionOutcome::Cancelled => {}
                RequestPermissionOutcome::Selected(selected)
                    if pending
                        .request
                        .options
                        .iter()
                        .any(|option| option.option_id == selected.option_id) => {}
                _ => return Err(NativeError::InvalidPermissionOption),
            }
            permissions
                .remove(request_id)
                .ok_or(NativeError::UnknownPermission)?
        };
        pending
            .responder
            .respond(RequestPermissionResponse::new(decision))?;
        Ok(())
    }

    /// Cancel work, detach local routing, and close the remote session when supported.
    ///
    /// Without `session/close`, persisted agent history remains available for an
    /// explicit future load; this method never claims to delete remote history.
    pub async fn close(&self) -> Result<()> {
        let state = &self.lease.state;
        if state.is_closed() {
            return Ok(());
        }
        self.cancel().await?;
        if self
            .lease
            .client
            .info
            .agent_capabilities
            .session_capabilities
            .close
            .is_some()
        {
            let response = timeout(
                self.lease.client.options.shutdown_timeout,
                state
                    .connection
                    .send_request(CloseSessionRequest::new(state.id.clone()))
                    .block_task(),
            )
            .await;
            match response {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    state.close(error.clone().into(), false);
                    return Err(error.into());
                }
                Err(_) => {
                    state.close(NativeError::Timeout, true);
                    return Err(NativeError::Timeout);
                }
            }
        }
        state.close(NativeError::Closed, false);
        Ok(())
    }
}

async fn run_session(
    mut session: ActiveSession<'static, Agent>,
    mut commands: mpsc::Receiver<PromptCommand>,
    state: Arc<SessionState>,
    options: super::NativeOptions,
) -> Result<()> {
    let (completion_tx, mut completion_rx) = mpsc::unbounded_channel::<Result<StopReason>>();
    let mut active: Option<oneshot::Sender<Result<StopReason>>> = None;
    let mut closed = state.closed.subscribe();
    let mut connection_closed = state.client.closed.subscribe();
    let mut expiry = interval(
        options
            .permission_timeout
            .min(std::time::Duration::from_secs(1)),
    );
    loop {
        if state.is_closed() {
            return Err(state.error());
        }
        tokio::select! {
            biased;
            _ = closed.changed() => return Ok(()),
            _ = connection_closed.changed() => return Err(state.client.error()),
            message = session.read_update() => {
                match message? {
                    SessionMessage::SessionMessage(dispatch) => {
                        MatchDispatch::new(dispatch)
                            .if_notification(async |notification: SessionNotification| {
                                state.emit(Event::Update { update: notification.update })
                                    .map_err(|_| agent_client_protocol::Error::internal_error())
                            }).await
                            .if_request(async |request: RequestPermissionRequest, responder| {
                                if options.permissions == PermissionPolicy::Deny
                                    || lock(&state.turn).cancelled
                                    || lock(&state.permissions).len() >= options.event_capacity
                                {
                                    return responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled));
                                }
                                let request_id = format!("permission-{}", NEXT_PERMISSION.fetch_add(1, Ordering::Relaxed));
                                lock(&state.permissions).insert(request_id.clone(), PendingPermission {
                                    request: request.clone(), responder,
                                    expires: Instant::now() + options.permission_timeout,
                                });
                                state.emit(Event::Permission { request_id, request })
                                    .map_err(|_| agent_client_protocol::Error::internal_error())
                            }).await
                            .otherwise(async |dispatch| {
                                match dispatch {
                                    Dispatch::Request(_, responder) => responder.respond_with_result(Err(agent_client_protocol::Error::method_not_found())),
                                    _ => Ok(()),
                                }
                            }).await?;
                    }
                    SessionMessage::StopReason(stop_reason) => {
                        let _ = completion_tx.send(Ok(stop_reason));
                    }
                    _ => return Err(NativeError::Unsupported("the received SDK session event")),
                }
            }
            Some(outcome) = completion_rx.recv(), if active.is_some() => {
                state.finish(&outcome)?;
                if let Some(result) = active.take() { let _ = result.send(outcome); }
            }
            Some(command) = commands.recv(), if active.is_none() => {
                {
                    let mut turn = lock(&state.turn);
                    turn.current = Some(command.prompt.request_id.clone());
                }
                if lock(&state.turn).cancelled || command.result.is_closed() {
                    let outcome = Ok(StopReason::Cancelled);
                    state.finish(&outcome)?;
                    let _ = command.result.send(outcome);
                    continue;
                }
                active = Some(command.result);
                let tx = completion_tx.clone();
                session.connection().send_request(PromptRequest::new(state.id.clone(), command.prompt.content))
                    .on_receiving_result(async move |result: std::result::Result<PromptResponse, agent_client_protocol::Error>| {
                        // Earlier notifications are already on ActiveSession's queue.
                        // The biased reader drains them before publishing completion.
                        let _ = tx.send(result.map(|response| response.stop_reason).map_err(NativeError::from));
                        Ok(())
                    })?;
            }
            _ = expiry.tick() => {
                let expired: Vec<_> = lock(&state.permissions).iter()
                    .filter(|(_, pending)| pending.expires <= Instant::now() || pending.responder.cancellation().is_cancelled())
                    .map(|(id, _)| id.clone()).collect();
                for id in expired {
                    if let Some(pending) = lock(&state.permissions).remove(&id) {
                        let _ = pending.responder.respond(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled));
                    }
                }
            }
        }
    }
}

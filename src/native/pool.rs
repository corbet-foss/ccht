//! Keyed pool of native agent sessions sharing one client.
//!
//! Workspaces, projects or roles map to pool keys; each key owns exactly one
//! session, created lazily on first use. Turns in different keys run in
//! parallel automatically, while each key serializes its own turns. Events
//! arrive tagged so late answers still land where they were asked.
//!
//! Every turn gets a unique request identity (`ccht-pool-{key}-{n}` with a
//! per-key counter): conversation snapshots key turns by request id, so a
//! reused id would collide with the finished turn and be rejected. Never
//! construct turns with a fixed id per key.
//!
//! Rendering recipe: read buffered per-turn text and status back from
//! [`SessionPool::conversation`] (`Conversation::turns`) instead of
//! hand-buffering message chunks. Every streamed event is already applied
//! to its key's snapshot before it is re-emitted tagged.
//!
//! The pool owns session mechanics only: applications own prompts, permission
//! decisions, display and storage, and read conversation snapshots back for
//! replay or persistence.

use std::collections::HashMap;

use tokio::sync::mpsc;

use super::{
    AgentCommand, NativeClient, NativeError, NativeOptions, SessionHandle, SessionOptions,
};
use crate::{Conversation, SessionEvent, WireEvent};

/// Application-chosen session key: a workspace, project or role name.
pub type SessionKey = String;

/// Streamed pool activity. Raw session events keep their full detail; the
/// application maps them to display, storage and permission decisions.
#[derive(Debug)]
pub enum PoolEvent {
    /// A session event, tagged with its key.
    Session {
        /// Application-chosen session key.
        key: SessionKey,
        /// Raw session event with full detail (boxed: the common case
        /// dwarfs the terminal variant).
        event: Box<SessionEvent>,
    },
    /// The forwarder for a key drained: its session stream closed and no
    /// further events follow for that key. This carries no turn result;
    /// clear any busy indicator for the key and tell the user.
    Ended {
        /// Application-chosen session key.
        key: SessionKey,
    },
}

struct Session {
    handle: SessionHandle,
    conv: Conversation,
    busy: bool,
    /// Per-turn event sequence numbers. Snapshots number events per turn,
    /// so one shared counter would gap every turn after the first
    /// turn-less event (history/config updates carry no request id).
    seqs: HashMap<SessionKey, u64>,
    /// Started turns on this key. Supplies the per-turn request identity;
    /// see the module docs for why ids must be unique per turn, not per key.
    turns: u64,
}

struct Inner {
    client: Option<NativeClient>,
    sessions: HashMap<SessionKey, Session>,
    app_events: mpsc::UnboundedSender<PoolEvent>,
    fwd: Option<mpsc::UnboundedSender<(SessionKey, Option<SessionEvent>)>>,
}

/// Keyed native sessions over one shared client connection.
#[derive(Clone)]
pub struct SessionPool {
    inner: std::sync::Arc<tokio::sync::Mutex<Inner>>,
}

impl SessionPool {
    /// Connect the shared client and return the pool plus its event stream.
    /// A failed connection still returns a pool: every operation then
    /// reports the outage instead of panicking, so applications degrade
    /// gracefully (e.g. keep local tools working while the agent is down).
    pub async fn connect(
        command: AgentCommand,
        options: NativeOptions,
    ) -> (Self, mpsc::UnboundedReceiver<PoolEvent>) {
        let (app_tx, app_rx) = mpsc::unbounded_channel();
        let (fwd_tx, fwd_rx) = mpsc::unbounded_channel::<(SessionKey, Option<SessionEvent>)>();
        let client = NativeClient::connect(command, options).await.ok();
        let pool = Self {
            inner: std::sync::Arc::new(tokio::sync::Mutex::new(Inner {
                client,
                sessions: HashMap::new(),
                app_events: app_tx,
                fwd: Some(fwd_tx),
            })),
        };
        let router = pool.clone();
        tokio::spawn(async move {
            Self::route(router, fwd_rx).await;
        });
        (pool, app_rx)
    }

    /// Send a prompt to a key's session, creating it from `new` on first
    /// use. Resolves when the turn ends; stream events (including the
    /// terminal `Completed`) arrive separately, tagged with the key.
    /// Each turn carries a fresh `ccht-pool-{key}-{n}` request id.
    /// A second prompt while the key is busy fails with [`NativeError::Busy`].
    pub async fn prompt(
        &self,
        key: &str,
        new: SessionOptions,
        text: String,
    ) -> Result<(), NativeError> {
        let (handle, request_id) = {
            let mut inner = self.inner.lock().await;
            Self::ensure(&mut inner, key, new).await?;
            let session = inner.sessions.get_mut(key).expect("just ensured");
            if session.busy {
                return Err(NativeError::Busy);
            }
            session.busy = true;
            session.turns += 1;
            let request_id = format!("ccht-pool-{key}-{}", session.turns);
            (session.handle.clone(), request_id)
        };
        let result = handle.prompt(crate::Prompt::text(request_id, text)).await;
        // Busy clears on the terminal stream event; a failed call has none,
        // so release the slot here to avoid wedging the key.
        if result.is_err()
            && let Some(session) = self.inner.lock().await.sessions.get_mut(key)
        {
            session.busy = false;
        }
        result.map(|_| ())
    }

    /// Cancel the active turn of one key. Missing keys are a no-op success.
    pub async fn cancel(&self, key: &str) -> Result<(), NativeError> {
        let handle = {
            let inner = self.inner.lock().await;
            match inner.sessions.get(key) {
                Some(session) => session.handle.clone(),
                None => return Ok(()),
            }
        };
        handle.cancel().await
    }

    /// Switch the model of an existing session, reporting the agent's
    /// resulting configuration. Unknown keys fail without creating sessions:
    /// model selection needs no session of its own.
    pub async fn set_model(
        &self,
        key: &str,
        model: &str,
    ) -> Result<crate::SessionConfiguration, NativeError> {
        let handle = {
            let inner = self.inner.lock().await;
            match inner.sessions.get(key) {
                Some(session) => session.handle.clone(),
                None => return Err(NativeError::Closed),
            }
        };
        handle.set_model(model).await
    }

    /// Answer a pending permission request of one key's session.
    pub async fn respond_permission(
        &self,
        key: &str,
        request_id: &str,
        decision: crate::acp::RequestPermissionOutcome,
    ) -> Result<(), NativeError> {
        let handle = {
            let inner = self.inner.lock().await;
            match inner.sessions.get(key) {
                Some(session) => session.handle.clone(),
                None => return Err(NativeError::Closed),
            }
        };
        handle.respond_permission(request_id, decision).await
    }

    /// Drop one key's session and end it agent-side. Unknown keys are a no-op.
    pub async fn close(&self, key: &str) {
        let handle = {
            let mut inner = self.inner.lock().await;
            inner.sessions.remove(key).map(|s| s.handle)
        };
        if let Some(handle) = handle {
            let _ = handle.close().await;
        }
    }

    /// Whether the key holds a live session with a turn in flight.
    pub async fn is_busy(&self, key: &str) -> bool {
        self.inner
            .lock()
            .await
            .sessions
            .get(key)
            .is_some_and(|s| s.busy)
    }

    /// Whether the key holds a live session at all.
    pub async fn has_session(&self, key: &str) -> bool {
        self.inner.lock().await.sessions.contains_key(key)
    }

    /// A snapshot of one key's conversation for replay or persistence.
    /// Applications own storage; the pool only lends the state.
    pub async fn conversation(&self, key: &str) -> Option<Conversation> {
        self.inner
            .lock()
            .await
            .sessions
            .get(key)
            .map(|s| s.conv.clone())
    }

    /// The live session configuration of one key, if it holds a session.
    /// Unlike the conversation snapshot (which only learns configuration
    /// from streamed updates), this reports what the agent advertised at
    /// session creation, so applications can name the session's model
    /// before the first configuration update arrives.
    pub async fn session_configuration(&self, key: &str) -> Option<crate::SessionConfiguration> {
        let handle = {
            self.inner
                .lock()
                .await
                .sessions
                .get(key)
                .map(|s| s.handle.clone())
        };
        handle.map(|h| h.configuration())
    }

    /// Shut every session down and close the shared client.
    pub async fn shutdown(&self) {
        let mut inner = self.inner.lock().await;
        let keys: Vec<String> = inner.sessions.keys().cloned().collect();
        for key in keys {
            if let Some(session) = inner.sessions.remove(&key) {
                let _ = session.handle.close().await;
            }
        }
        if let Some(client) = inner.client.take() {
            let _ = client.close().await;
        }
    }

    async fn ensure(inner: &mut Inner, key: &str, new: SessionOptions) -> Result<(), NativeError> {
        if inner.sessions.contains_key(key) {
            return Ok(());
        }
        let Some(client) = &inner.client else {
            return Err(NativeError::Closed);
        };
        let mut session = client.new_session(new).await?;
        let handle = session.handle();
        let mut events = session.take_events().map_err(|_| NativeError::Closed)?;
        drop(session);
        let key_owned = key.to_string();
        let fwd = inner
            .fwd
            .clone()
            .expect("forwarder channel installed at connect");
        tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                if fwd.send((key_owned.clone(), Some(ev))).is_err() {
                    break;
                }
            }
            let _ = fwd.send((key_owned, None));
        });
        inner.sessions.insert(
            key.to_string(),
            Session {
                handle,
                conv: Conversation::new(format!("pool-{key}")),
                busy: false,
                seqs: HashMap::new(),
                turns: 0,
            },
        );
        Ok(())
    }

    /// Single router: applies every streamed event to its key's conversation
    /// (clearing finished turns) and re-emits it tagged for the application.
    async fn route(
        pool: Self,
        mut fwd: mpsc::UnboundedReceiver<(SessionKey, Option<SessionEvent>)>,
    ) {
        while let Some((key, ev)) = fwd.recv().await {
            let mut inner = pool.inner.lock().await;
            let Some(session) = inner.sessions.get_mut(&key) else {
                continue;
            };
            let Some(se) = ev else {
                session.busy = false;
                let _ = inner.app_events.send(PoolEvent::Ended { key });
                continue;
            };
            let rid = se.request_id.clone().unwrap_or_default();
            let SessionEvent { event, .. } = &se;
            if rid.is_empty() {
                // Turn-less events (history/config updates with no active
                // request) carry no turn identity: forward them, but keep
                // them out of the snapshot. Applying them would fail, and
                // spending a sequence number on them would gap the turn.
                let _ = inner.app_events.send(PoolEvent::Session {
                    key,
                    event: Box::new(se),
                });
                continue;
            }
            let seq = session.seqs.entry(rid.clone()).or_insert(0);
            *seq += 1;
            let conv_id = session.conv.id().to_string();
            let _ = session
                .conv
                .apply(WireEvent::new(conv_id, rid.clone(), *seq, event.clone()));
            if matches!(
                &event,
                crate::Event::Completed { .. } | crate::Event::Error { .. }
            ) {
                session.busy = false;
                session.seqs.remove(&rid);
            }
            let _ = inner.app_events.send(PoolEvent::Session {
                key,
                event: Box::new(se),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Event;
    use crate::acp::{ContentBlock, SessionUpdate};
    use std::time::Duration;
    use tokio::time::timeout;

    fn options() -> NativeOptions {
        NativeOptions {
            operation_timeout: Duration::from_secs(5),
            prompt_timeout: Duration::from_secs(5),
            permission_timeout: Duration::from_secs(2),
            shutdown_timeout: Duration::from_secs(2),
            ..NativeOptions::default()
        }
    }

    fn command(mode: &str) -> AgentCommand {
        AgentCommand::new("python3").args(["-u", "-c", include_str!("fixture.py"), mode])
    }

    fn session_options() -> SessionOptions {
        SessionOptions::new(std::env::temp_dir())
    }

    fn text(event: &SessionEvent) -> Option<&str> {
        match &event.event {
            Event::Update {
                update: SessionUpdate::AgentMessageChunk(chunk),
            } => {
                if let ContentBlock::Text(content) = &chunk.content {
                    Some(&content.text)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Collect streamed text for one key until its turn terminates.
    /// Other keys' events are parked and replayed to the caller in order.
    async fn collect_two(rx: &mut mpsc::UnboundedReceiver<PoolEvent>) -> (String, String) {
        let (mut a, mut b) = (String::new(), String::new());
        let (mut done_a, mut done_b) = (false, false);
        while !done_a || !done_b {
            let ev = timeout(Duration::from_secs(15), rx.recv())
                .await
                .expect("event in time")
                .expect("stream open");
            if let PoolEvent::Session { key, event } = ev {
                let terminal = matches!(event.event, Event::Completed { .. } | Event::Error { .. });
                if let Some(t) = text(&event) {
                    match key.as_str() {
                        "a" => a.push_str(t),
                        "b" => b.push_str(t),
                        _ => {}
                    }
                }
                if terminal {
                    match key.as_str() {
                        "a" => done_a = true,
                        "b" => done_b = true,
                        _ => {}
                    }
                }
            }
        }
        (a, b)
    }

    #[tokio::test]
    async fn parallel_keys_complete_independently() {
        let (pool, mut rx) = SessionPool::connect(command("normal"), options()).await;
        // Both turns run concurrently; a single shared session could not do this.
        let p1 = pool.clone();
        let t1 =
            tokio::spawn(
                async move { p1.prompt("a", session_options(), "first".to_string()).await },
            );
        let p2 = pool.clone();
        let t2 = tokio::spawn(async move {
            p2.prompt("b", session_options(), "second".to_string())
                .await
        });
        let (r1, r2, texts) = tokio::join!(t1, t2, collect_two(&mut rx));
        r1.expect("task a").expect("turn a");
        r2.expect("task b").expect("turn b");
        // Fixture answers every turn with the same stream.
        assert_eq!(
            texts,
            ("hello world".to_string(), "hello world".to_string())
        );
        assert!(!pool.is_busy("a").await);
        assert!(!pool.is_busy("b").await);
        // Conversations accumulated per key for replay.
        assert!(pool.conversation("a").await.is_some());
        assert!(pool.conversation("b").await.is_some());
        assert!(pool.conversation("ghost").await.is_none());
        pool.shutdown().await;
    }

    #[tokio::test]
    async fn sequential_turns_get_unique_request_ids() {
        let (pool, mut rx) = SessionPool::connect(command("normal"), options()).await;
        // Same key twice in a row: the second turn must not reuse the first
        // turn's request id, or the snapshot would reject its events as a
        // collision with a finished turn and freeze after turn one.
        for text in ["one", "two"] {
            pool.prompt("k", session_options(), text.to_string())
                .await
                .expect("turn completes");
        }
        let mut ids = Vec::new();
        let mut terminals = 0;
        while terminals < 2 {
            let ev = timeout(Duration::from_secs(15), rx.recv())
                .await
                .expect("event in time")
                .expect("stream open");
            if let PoolEvent::Session { key, event } = ev {
                assert_eq!(key, "k");
                let id = event.request_id.clone().unwrap_or_default();
                if !ids.contains(&id) {
                    ids.push(id);
                }
                if matches!(event.event, Event::Completed { .. } | Event::Error { .. }) {
                    terminals += 1;
                }
            }
        }
        assert_eq!(
            ids,
            vec!["ccht-pool-k-1".to_string(), "ccht-pool-k-2".to_string()]
        );
        // Both turns accumulated into the snapshot with their buffered text.
        let conv = pool.conversation("k").await.expect("snapshot");
        assert_eq!(conv.turns().len(), 2);
        for (turn, id) in conv.turns().iter().zip(&ids) {
            assert_eq!(&turn.request_id, id);
            assert_eq!(turn.text, "hello world");
            assert_eq!(turn.status, "completed");
        }
        assert!(!pool.is_busy("k").await);
        pool.shutdown().await;
    }

    #[tokio::test]
    async fn second_prompt_on_busy_key_fails_cleanly() {
        use crate::native::NativeError;

        let (pool, _rx) = SessionPool::connect(command("hang"), options()).await;
        // The hang fixture holds the turn open without answering, so the
        // overlapping prompt deterministically hits the busy guard.
        let p = pool.clone();
        let held = tokio::spawn(async move {
            // The hang fixture never answers, so the held turn stays open
            // until the prompt timeout fires; the busy transition in between
            // is what this test observes, not the outcome.
            let _ = p.prompt("k", session_options(), "one".to_string()).await;
        });
        let mut busy_seen = false;
        for _ in 0..100 {
            if pool.is_busy("k").await {
                busy_seen = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(busy_seen, "turn never went busy");
        let second = pool.prompt("k", session_options(), "two".to_string()).await;
        assert!(
            matches!(second, Err(NativeError::Busy)),
            "expected Busy, got {second:?}"
        );
        let _ = held.await;
        assert!(!pool.is_busy("k").await);
        pool.shutdown().await;
    }

    #[tokio::test]
    async fn missing_keys_fail_without_creating_sessions() {
        let (pool, _rx) = SessionPool::connect(command("normal"), options()).await;
        assert!(pool.set_model("ghost", "m").await.is_err());
        assert!(pool.cancel("ghost").await.is_ok());
        assert!(!pool.has_session("ghost").await);
        pool.close("ghost").await;
        pool.shutdown().await;
    }
}

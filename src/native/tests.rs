use super::*;
use crate::acp::{ContentBlock, SelectedPermissionOutcome, SessionUpdate, StopReason};
use crate::{Event, Prompt, SessionEvent};
use tokio::sync::mpsc;
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

fn prompt(id: &str) -> Prompt {
    Prompt {
        request_id: id.into(),
        content: vec!["test".into()],
    }
}

async fn session(
    mode: &str,
    options: NativeOptions,
) -> (
    NativeClient,
    NativeSession,
    SessionHandle,
    mpsc::Receiver<SessionEvent>,
) {
    let client = NativeClient::connect(command(mode), options).await.unwrap();
    let mut session = client
        .new_session(SessionOptions::new(std::env::temp_dir()))
        .await
        .unwrap();
    let handle = session.handle();
    let events = session.take_events().unwrap();
    (client, session, handle, events)
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

#[tokio::test]
async fn ordered_stream_and_multiple_turns_keep_one_session() {
    let (client, _session, handle, mut events) = session("normal", options()).await;
    for id in ["first", "second"] {
        assert_eq!(
            handle.prompt(prompt(id)).await.unwrap(),
            StopReason::EndTurn
        );
        let first = events.recv().await.unwrap();
        let second = events.recv().await.unwrap();
        let completed = events.recv().await.unwrap();
        assert_eq!(first.request_id.as_deref(), Some(id));
        assert_eq!(text(&first), Some("hello "));
        assert_eq!(text(&second), Some("world"));
        assert!(matches!(
            completed.event,
            Event::Completed {
                stop_reason: StopReason::EndTurn
            }
        ));
    }
    handle.close().await.unwrap();
    assert!(events.recv().await.is_none());
    client.close().await.unwrap();
}

#[tokio::test]
async fn default_permissions_deny_without_user_approval() {
    let (client, _session, handle, mut events) = session("deny", options()).await;
    handle.prompt(prompt("deny")).await.unwrap();
    assert_eq!(text(&events.recv().await.unwrap()), Some("cancelled"));
    assert!(matches!(
        events.recv().await.unwrap().event,
        Event::Completed { .. }
    ));
    client.close().await.unwrap();
}

#[tokio::test]
async fn permission_responses_are_validated_and_isolated_between_sessions() {
    let mut options = options();
    options.permissions = PermissionPolicy::Ask;
    let (client, _first, handle, mut events) = session("ask", options).await;
    let mut second = client
        .new_session(SessionOptions::new(std::env::temp_dir()))
        .await
        .unwrap();
    let other = second.handle();
    let mut other_events = second.take_events().unwrap();
    let first_prompt = tokio::spawn({
        let handle = handle.clone();
        async move { handle.prompt(prompt("first")).await }
    });
    let second_prompt = tokio::spawn({
        let other = other.clone();
        async move { other.prompt(prompt("second")).await }
    });
    let first_request = events.recv().await.unwrap();
    let second_request = other_events.recv().await.unwrap();
    let Event::Permission {
        request_id,
        request,
    } = first_request.event
    else {
        panic!("permission event expected")
    };
    let Event::Permission {
        request_id: second_id,
        request: second_request,
    } = second_request.event
    else {
        panic!("permission event expected")
    };
    assert_ne!(request.session_id, second_request.session_id);
    assert_ne!(request_id, second_id);
    assert_eq!(
        other
            .respond_permission(&request_id, PermissionDecision::Cancelled)
            .await
            .unwrap_err(),
        NativeError::UnknownPermission
    );
    let invalid = PermissionDecision::Selected(SelectedPermissionOutcome::new("not-offered"));
    assert_eq!(
        handle
            .respond_permission(&request_id, invalid)
            .await
            .unwrap_err(),
        NativeError::InvalidPermissionOption
    );
    handle
        .respond_permission(
            &request_id,
            PermissionDecision::Selected(SelectedPermissionOutcome::new("allow")),
        )
        .await
        .unwrap();
    assert_eq!(first_prompt.await.unwrap().unwrap(), StopReason::EndTurn);
    assert!(!second_prompt.is_finished());
    other
        .respond_permission(&second_id, PermissionDecision::Cancelled)
        .await
        .unwrap();
    assert_eq!(second_prompt.await.unwrap().unwrap(), StopReason::EndTurn);
    assert_eq!(
        handle
            .respond_permission(&request_id, PermissionDecision::Cancelled)
            .await
            .unwrap_err(),
        NativeError::UnknownPermission
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn active_prompt_rejects_overlap_and_cancels_cooperatively() {
    let (client, _session, handle, mut events) = session("hang", options()).await;
    let running = tokio::spawn({
        let handle = handle.clone();
        async move { handle.prompt(prompt("first")).await }
    });
    assert_eq!(text(&events.recv().await.unwrap()), Some("waiting"));
    assert_eq!(
        handle.prompt(prompt("overlap")).await.unwrap_err(),
        NativeError::Busy
    );
    handle.cancel().await.unwrap();
    assert_eq!(running.await.unwrap().unwrap(), StopReason::Cancelled);
    assert!(matches!(
        events.recv().await.unwrap().event,
        Event::Completed {
            stop_reason: StopReason::Cancelled
        }
    ));
    client.close().await.unwrap();
}

#[tokio::test]
async fn ignored_cancellation_and_abandoned_prompts_close_the_connection() {
    let mut limits = options();
    limits.shutdown_timeout = Duration::from_millis(100);
    let (client, _session, handle, mut events) = session("ignore_cancel", limits).await;
    let running = tokio::spawn({
        let handle = handle.clone();
        async move { handle.prompt(prompt("first")).await }
    });
    events.recv().await.unwrap();
    assert_eq!(handle.cancel().await.unwrap_err(), NativeError::Timeout);
    assert!(running.await.unwrap().is_err());
    assert!(
        client
            .new_session(SessionOptions::new(std::env::temp_dir()))
            .await
            .is_err()
    );

    let (client, _session, handle, mut events) = session("hang", options()).await;
    let running = tokio::spawn({
        let handle = handle.clone();
        async move { handle.prompt(prompt("abandoned")).await }
    });
    events.recv().await.unwrap();
    running.abort();
    let _ = running.await;
    assert!(
        client
            .new_session(SessionOptions::new(std::env::temp_dir()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn restoration_preserves_history_before_new_prompt_identity() {
    let client = NativeClient::connect(command("normal"), options())
        .await
        .unwrap();
    let mut restored = client
        .load_session("saved-session", SessionOptions::new(std::env::temp_dir()))
        .await
        .unwrap();
    assert_eq!(restored.handle().id(), "saved-session");
    let mut events = restored.take_events().unwrap();
    let history = events.recv().await.unwrap();
    assert_eq!(history.request_id, None);
    assert_eq!(text(&history), Some("restored history"));
    restored.handle().prompt(prompt("continued")).await.unwrap();
    assert_eq!(
        events.recv().await.unwrap().request_id.as_deref(),
        Some("continued")
    );
    client.close().await.unwrap();
}

#[tokio::test]
async fn auth_errors_exclude_peer_secrets_and_unknown_host_requests_fail() {
    let client = NativeClient::connect(command("auth_required"), options())
        .await
        .unwrap();
    let error = match client
        .new_session(SessionOptions::new(std::env::temp_dir()))
        .await
    {
        Ok(_) => panic!("authentication failure expected"),
        Err(error) => error,
    };
    assert_eq!(error, NativeError::AuthenticationRequired);
    assert!(!format!("{error:?} {error}").contains("SECRET"));

    let (client, _session, handle, mut events) =
        session("unsupported_host_request", options()).await;
    handle.prompt(prompt("unsupported")).await.unwrap();
    assert_eq!(text(&events.recv().await.unwrap()), Some("unsupported"));
    client.close().await.unwrap();
}

#[tokio::test]
async fn slow_consumers_fail_with_bounded_buffer_and_shutdown() {
    let mut limits = options();
    limits.event_capacity = 2;
    let (client, _session, handle, _events) = session("burst", limits).await;
    let result = timeout(Duration::from_secs(5), handle.prompt(prompt("overflow")))
        .await
        .unwrap();
    assert_eq!(result.unwrap_err(), NativeError::EventBufferFull);
    assert!(
        client
            .new_session(SessionOptions::new(std::env::temp_dir()))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn model_selection_is_explicit_and_api_key_overrides_are_rejected() {
    let client = NativeClient::connect(command("normal"), options())
        .await
        .unwrap();
    let mut selected = SessionOptions::new(std::env::temp_dir());
    selected.model = Some("test-model".into());
    let mut session = client.new_session(selected.clone()).await.unwrap();
    let _events = session.take_events().unwrap();
    session.handle().prompt(prompt("selected")).await.unwrap();
    selected.model = Some("missing-model".into());
    assert!(matches!(
        client.new_session(selected).await,
        Err(NativeError::Unsupported(_))
    ));
    client.close().await.unwrap();
    let mut keyed = command("normal");
    keyed
        .env
        .insert("OPENAI_API_KEY".into(), "not-a-real-key".into());
    assert!(matches!(
        NativeClient::connect(keyed, options()).await,
        Err(NativeError::InvalidOptions(_))
    ));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn installed_launcher_sets_the_actual_startup_directory() {
    let directory = std::env::temp_dir().canonicalize().unwrap();
    let launch = command("startup_cwd")
        .with_working_directory(&directory)
        .unwrap();
    let client = NativeClient::connect(launch, options()).await.unwrap();
    let mut session = client
        .new_session(SessionOptions::new(&directory))
        .await
        .unwrap();
    let mut events = session.take_events().unwrap();
    session.handle().prompt(prompt("cwd")).await.unwrap();
    assert_eq!(text(&events.recv().await.unwrap()), directory.to_str());
    client.close().await.unwrap();
}

#[cfg(target_os = "linux")]
fn pid_fixture(mode: &str) -> (AgentCommand, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "ccht-native-pids-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut command = command(mode);
    command.args.push(path.to_str().unwrap().into());
    (command, path)
}

#[cfg(target_os = "linux")]
async fn fixture_pids(path: &std::path::Path) -> Vec<u32> {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(bytes) = std::fs::read(path)
                && let Ok(pids) = serde_json::from_slice::<BTreeMap<String, u32>>(&bytes)
            {
                return pids.into_values().collect();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[cfg(target_os = "linux")]
async fn assert_processes_stopped(pids: &[u32]) {
    timeout(Duration::from_secs(5), async {
        loop {
            let running = pids.iter().any(|pid| {
                std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                    // A zombie is terminated; reaping an orphan belongs to the
                    // worker's init process, not to the client under test.
                    !stat
                        .rsplit_once(") ")
                        .is_some_and(|(_, fields)| fields.starts_with('Z'))
                })
            });
            if !running {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SDK must terminate the agent and its descendant");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn ignored_cancellation_really_terminates_the_sdk_process_group() {
    let (launch, path) = pid_fixture("ignore_cancel");
    let mut limits = options();
    limits.shutdown_timeout = Duration::from_millis(100);
    let client = NativeClient::connect(launch, limits).await.unwrap();
    let pids = fixture_pids(&path).await;
    let mut session = client
        .new_session(SessionOptions::new(std::env::temp_dir()))
        .await
        .unwrap();
    let handle = session.handle();
    let mut events = session.take_events().unwrap();
    let running = tokio::spawn({
        let handle = handle.clone();
        async move { handle.prompt(prompt("stop-processes")).await }
    });
    events.recv().await.unwrap();
    assert_eq!(handle.cancel().await.unwrap_err(), NativeError::Timeout);
    assert!(running.await.unwrap().is_err());
    assert_processes_stopped(&pids).await;
    let _ = std::fs::remove_file(path);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dropping_connect_during_initialization_really_terminates_its_children() {
    let (launch, path) = pid_fixture("slow_init");
    let connecting = tokio::spawn(NativeClient::connect(launch, options()));
    let pids = fixture_pids(&path).await;
    connecting.abort();
    let _ = connecting.await;
    assert_processes_stopped(&pids).await;
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn advertised_controls_preserve_dependent_changes_and_reject_invalid_values() {
    let (client, _session, handle, mut events) = session("normal", options()).await;
    assert!(
        handle
            .configuration()
            .accepts("model", &"test-model".into())
    );
    let changed = handle.set_model("second-model").await.unwrap();
    assert!(matches!(&changed.options[1].kind,
        crate::acp::SessionConfigKind::Boolean(value) if value.current_value));
    let event = events.recv().await.unwrap();
    assert!(event.request_id.is_none());
    assert!(matches!(
        event.event,
        Event::Update {
            update: SessionUpdate::ConfigOptionUpdate(_)
        }
    ));
    assert!(
        handle
            .set_config_option("thinking", false.into())
            .await
            .is_ok()
    );
    assert!(matches!(
        handle.set_model("not-offered").await,
        Err(NativeError::Unsupported(_))
    ));
    assert!(matches!(
        handle.set_config_option("thinking", "false".into()).await,
        Err(NativeError::Unsupported(_))
    ));
    client.close().await.unwrap();
}

fn codex_fixture(mode: &str) -> super::drivers::CodexDeviceDriver {
    use std::time::Duration;
    super::drivers::CodexDeviceDriver::new("python3")
        .with_lead_args(vec![
            "-u".to_owned(),
            "-c".to_owned(),
            include_str!("fixture.py").to_owned(),
            mode.to_owned(),
        ])
        .with_call_timeout(Duration::from_secs(5))
        .with_approval_deadline(Duration::from_secs(5))
}

fn opencode_fixture(mode: &str, key: &str) -> super::drivers::OpenCodeKeyDriver {
    use std::time::Duration;
    super::drivers::OpenCodeKeyDriver::new("python3", key.to_owned())
        .unwrap()
        .with_lead_args(vec![
            "-u".to_owned(),
            "-c".to_owned(),
            include_str!("fixture.py").to_owned(),
            mode.to_owned(),
        ])
        .with_operation_timeout(Duration::from_secs(5))
        .with_readiness_timeout(Duration::from_secs(5))
}

#[tokio::test]
async fn codex_challenge_round_trip_projects_presence() {
    use super::drivers::{LoginDriver, LoginState};
    let mut driver = codex_fixture("codex_ok");
    let state = driver.start().await.unwrap();
    let challenge = match state {
        LoginState::ChallengeRequired(challenge) => challenge,
        other => panic!("challenge expected, got {other:?}"),
    };
    assert_eq!(
        challenge.verification_url,
        "https://auth.openai.com/codex/device"
    );
    assert_eq!(challenge.user_code, "ABCD-1234");
    challenge.validate().unwrap();
    let state = driver.poll().await.unwrap();
    match state {
        LoginState::Authenticated(info) => {
            assert_eq!(info.account.as_deref(), Some("user@example.com"));
        }
        other => panic!("authenticated presence expected, got {other:?}"),
    }
    let info = driver.account().await.unwrap();
    assert_eq!(info.account.as_deref(), Some("user@example.com"));
}

#[tokio::test]
async fn codex_rejects_malformed_challenge_without_leak() {
    use super::drivers::{Challenge, DriverError, LoginDriver};
    for mode in ["codex_bad_url", "codex_bad_code"] {
        let mut driver = codex_fixture(mode);
        let error = driver.start().await.unwrap_err();
        assert!(
            matches!(error, DriverError::InvalidOptions(_)),
            "expected invalid options for {mode}, got {error:?}"
        );
        assert_eq!(error.code(), "invalid_options");
        let rendered = format!("{error} {error:?}");
        assert!(!rendered.contains("ABCD-1234"));
        assert!(!rendered.contains("evil.example.com"));
    }
    let bad = Challenge::new(
        "http://evil.example.com/x".to_owned(),
        "ABCD-1234".to_owned(),
    );
    assert!(matches!(
        bad.validate().unwrap_err(),
        DriverError::InvalidOptions(_)
    ));
    let short = Challenge::new(
        "https://auth.openai.com/codex/device".to_owned(),
        "x".to_owned(),
    );
    assert!(matches!(
        short.validate().unwrap_err(),
        DriverError::InvalidOptions(_)
    ));
    let custom = super::drivers::CodexDeviceDriver::with_allowed_hosts(
        "python3",
        vec!["example.com".to_owned()],
    )
    .unwrap()
    .with_lead_args(vec![
        "-u".to_owned(),
        "-c".to_owned(),
        include_str!("fixture.py").to_owned(),
        "codex_ok".to_owned(),
    ]);
    let mut custom = custom;
    let error = custom.start().await.unwrap_err();
    assert_eq!(
        error,
        DriverError::InvalidOptions("challenge host is not allowed")
    );
}

#[tokio::test]
async fn codex_poll_times_out_without_browser_approval() {
    use super::drivers::{DriverError, LoginDriver, LoginState};
    use std::time::Duration;
    let mut driver = codex_fixture("codex_never");
    driver = driver
        .with_call_timeout(Duration::from_millis(300))
        .with_approval_deadline(Duration::from_millis(400));
    let state = driver.start().await.unwrap();
    assert!(matches!(state, LoginState::ChallengeRequired(_)));
    let error = driver.poll().await.unwrap_err();
    assert_eq!(error, DriverError::Timeout);
    assert_eq!(error.code(), "timeout");
}

#[tokio::test]
async fn codex_cancel_stops_helper_and_reports_cancelled() {
    use super::drivers::{DriverError, LoginDriver, LoginState};
    let mut driver = codex_fixture("codex_never");
    let state = driver.start().await.unwrap();
    assert!(matches!(state, LoginState::ChallengeRequired(_)));
    assert!(driver.has_helper());
    driver.cancel().await.unwrap();
    assert!(!driver.has_helper());
    assert_eq!(driver.poll().await.unwrap_err(), DriverError::Cancelled);
    assert_eq!(driver.account().await.unwrap_err(), DriverError::Cancelled);
    assert_eq!(driver.start().await.unwrap_err(), DriverError::Cancelled);
}

#[tokio::test]
async fn codex_declined_approval_reports_failed() {
    use super::drivers::{LoginDriver, LoginState};
    let mut driver = codex_fixture("codex_declined");
    let state = driver.start().await.unwrap();
    assert!(matches!(state, LoginState::ChallengeRequired(_)));
    assert_eq!(driver.poll().await.unwrap(), LoginState::Failed);
}

#[tokio::test]
async fn opencode_key_login_confirms_connected() {
    use super::drivers::{LoginDriver, LoginState};
    let key = "ccht-fixture-key-9f8e7d6c5b4a";
    let mut driver = opencode_fixture("opencode_ok", key);
    let state = driver.start().await.unwrap();
    match state {
        LoginState::Authenticated(info) => {
            assert_eq!(info.account.as_deref(), Some("opencode-go"));
        }
        other => panic!("authenticated presence expected, got {other:?}"),
    }
    let state = driver.poll().await.unwrap();
    assert!(matches!(state, LoginState::Authenticated(_)));
    let info = driver.account().await.unwrap();
    assert_eq!(info.account.as_deref(), Some("opencode-go"));
    let rendered = format!("{driver:?}");
    assert!(!rendered.contains(key));
}

#[tokio::test]
async fn opencode_start_times_out_when_server_never_ready() {
    use super::drivers::{DriverError, LoginDriver};
    use std::time::Duration;
    let mut driver = opencode_fixture("opencode_never", "ccht-fixture-key-timeout");
    driver = driver
        .with_operation_timeout(Duration::from_millis(300))
        .with_readiness_timeout(Duration::from_millis(400));
    let error = driver.start().await.unwrap_err();
    assert_eq!(error, DriverError::Timeout);
}

#[tokio::test]
async fn driver_debug_and_errors_omit_key_material() {
    use super::drivers::{DriverError, LoginDriver};
    let key = "ccht-fixture-key-secret-12345";
    let driver = super::drivers::OpenCodeKeyDriver::new("python3", key.to_owned()).unwrap();
    let rendered = format!("{driver:?}");
    assert!(!rendered.contains(key));
    assert!(!rendered.contains("OPENCODE_SERVER_PASSWORD"));
    let codex = super::drivers::CodexDeviceDriver::new("python3");
    let codex_rendered = format!("{codex:?}");
    assert!(!codex_rendered.contains("ABCD-1234"));
    for error in [
        DriverError::InvalidOptions("bad shape"),
        DriverError::Timeout,
        DriverError::Closed,
        DriverError::Spawn("helper unavailable"),
        DriverError::Protocol(-32603),
        DriverError::Cancelled,
        DriverError::Unsupported("extra step"),
    ] {
        let text = format!("{error} {error:?} {}", error.code());
        assert!(!text.contains(key));
        assert!(!text.contains("ccht-fixture"));
    }
    let mut failing = opencode_fixture("opencode_fail", key);
    let state = failing.start().await.unwrap();
    assert_eq!(state, super::drivers::LoginState::Failed);
    let rendered = format!("{failing:?}");
    assert!(!rendered.contains(key));
}

#[test]
fn native_error_maps_to_shared_login_state() {
    use crate::AuthState;
    assert_eq!(
        NativeError::AuthenticationRequired.auth_state(),
        AuthState::Unauthenticated
    );
    assert_eq!(
        NativeError::AuthenticationRequired.code(),
        "authentication_required"
    );
    for error in [
        NativeError::Closed,
        NativeError::Timeout,
        NativeError::Busy,
        NativeError::Unsupported("extra step"),
        NativeError::Protocol(-32603),
    ] {
        assert_eq!(error.auth_state(), AuthState::Unknown);
        assert_ne!(error.code(), "authentication_required");
    }
}

#[test]
fn agent_command_validates_stdio_shape_without_spawning() {
    use crate::TransportError;
    let command = AgentCommand::new("codex-acp").args(["--acp"]);
    command.validate_stdio().unwrap();
    assert_eq!(command.args, vec!["--acp".to_owned()]);
    let empty = AgentCommand::new("");
    let error = empty.validate_stdio().unwrap_err();
    assert_eq!(
        error,
        TransportError::InvalidAddress("stdio program must not be empty")
    );
}

#[test]
fn seal_parent_env_keeps_explicit_entries_and_blanks_secrets() {
    let mut command = AgentCommand::new("agent");
    command.env.insert("PATH".into(), "/explicit/bin".into());
    command
        .env
        .insert("OPENAI_API_KEY".into(), "super-secret".into());
    command.seal_parent_env(EnvProfile::strict());
    assert_eq!(
        command.env.get("PATH").map(String::as_str),
        Some("/explicit/bin"),
        "explicit spawn overrides must survive parent seeding"
    );
    assert_eq!(
        command.env.get("OPENAI_API_KEY").map(String::as_str),
        Some(""),
        "secrets must be blanked even when set explicitly"
    );
    assert_eq!(command.program.to_str(), Some("agent"));
}

#[test]
fn sealed_parent_env_builder_matches_in_place_variant() {
    let mut inplace = AgentCommand::new("agent");
    inplace.env.insert("PATH".into(), "/explicit/bin".into());
    inplace.seal_parent_env(EnvProfile::strict());
    let mut built = AgentCommand::new("agent");
    built.env.insert("PATH".into(), "/explicit/bin".into());
    let built = built.with_sealed_parent_env(EnvProfile::strict());
    assert_eq!(built.env, inplace.env);
    assert_eq!(built.program, inplace.program);
}

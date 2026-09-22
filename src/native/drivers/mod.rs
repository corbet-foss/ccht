//! Native vendor login ceremonies behind one uniform trait.
//!
//! Drivers run the vendor handshake natively only; they are never compiled
//! for Wasm. A driver spawns the vendor helper, relays the user-visible
//! challenge, and projects the vendor reply into plain state. Drivers never
//! extract secret bytes, never persist anything, and never log credentials.
//! The application owns storage through [`crate::CredentialsProvider`]: it
//! keeps any secret, decides where the secret lives, and clears it on
//! sign-out. Drivers only report whether the vendor considers the account
//! connected and, when the vendor offers one, which display label to show.
//!
//! All failures use fixed messages so URLs, paths, tokens, and keys cannot
//! leak through [`DriverError`].

use std::path::PathBuf;

mod codex;
mod opencode;

pub use codex::CodexDeviceDriver;
pub use opencode::OpenCodeKeyDriver;

/// User-visible device challenge for a browser approval step.
///
/// The URL is opened by the person, the code is typed or pasted there.
/// Neither field is a secret, but both are validated before display so a
/// compromised helper cannot turn the application into an open redirect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    /// Secure page where the person approves the login.
    pub verification_url: String,
    /// Short code the person confirms on that page.
    pub user_code: String,
}

impl Challenge {
    /// Create a challenge without validation for transport purposes.
    ///
    /// Call [`Challenge::validate`] before display.
    pub fn new(verification_url: String, user_code: String) -> Self {
        Self {
            verification_url,
            user_code,
        }
    }

    /// Check shape without network access or persistence.
    ///
    /// The URL must use `https` and include a host; the code must be 4 to
    /// 32 characters of ASCII letters, digits, or `-`.
    ///
    /// # Errors
    ///
    /// Returns [`DriverError::InvalidOptions`] when either field has an
    /// unsupported shape.
    pub fn validate(&self) -> Result<(), DriverError> {
        if self.verification_url.is_empty() || self.user_code.is_empty() {
            return Err(DriverError::InvalidOptions(
                "challenge must include a URL and a code",
            ));
        }
        if self.verification_url.chars().any(char::is_whitespace) {
            return Err(DriverError::InvalidOptions(
                "challenge URL must not contain whitespace",
            ));
        }
        let Some(rest) = self.verification_url.strip_prefix("https://") else {
            return Err(DriverError::InvalidOptions("challenge URL must use https"));
        };
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        if host.is_empty() {
            return Err(DriverError::InvalidOptions(
                "challenge URL must include a host",
            ));
        }
        let host_ok = host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b':'));
        if !host_ok {
            return Err(DriverError::InvalidOptions(
                "challenge URL host has an unsupported shape",
            ));
        }
        let code = self.user_code.as_str();
        if !(4..=32).contains(&code.len()) {
            return Err(DriverError::InvalidOptions(
                "challenge code has an unsupported shape",
            ));
        }
        if !code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(DriverError::InvalidOptions(
                "challenge code has an unsupported shape",
            ));
        }
        Ok(())
    }
}

/// Projected account presence after a vendor `read` call.
///
/// The label is display-only provenance (for example an email). It is never
/// an authentication proof by itself and never carries secret bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountInfo {
    /// Display-only label when the vendor reports a connected account.
    pub account: Option<String>,
}

impl AccountInfo {
    /// Project a connected account with an optional display label.
    pub fn new(account: Option<String>) -> Self {
        Self { account }
    }

    /// Project a signed-out vendor state.
    pub fn signed_out() -> Self {
        Self { account: None }
    }

    /// Whether the vendor reports a connected account label.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.account
            .as_deref()
            .is_some_and(|label| !label.is_empty())
    }
}

/// Login driver failure without secrets, paths, or peer text.
///
/// Every message is fixed at compile time except for the numeric protocol
/// code, which carries no text or data from the vendor.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum DriverError {
    /// Driver configuration or a vendor challenge has an unsupported shape.
    #[error("invalid login driver configuration: {0}")]
    InvalidOptions(&'static str),
    /// The vendor did not finish within the configured deadline.
    #[error("login driver operation timed out")]
    Timeout,
    /// The helper exited or the driver was shut down before completion.
    #[error("login driver connection is closed")]
    Closed,
    /// The helper process could not start or exited early.
    #[error("login driver process could not start: {0}")]
    Spawn(&'static str),
    /// A numeric vendor protocol code without peer text or data.
    #[error("vendor returned protocol error {0}")]
    Protocol(i32),
    /// The application cancelled the attempt.
    #[error("login attempt was cancelled")]
    Cancelled,
    /// The vendor or driver does not implement the requested step.
    #[error("login driver does not support {0}")]
    Unsupported(&'static str),
}

impl DriverError {
    /// Stable category suitable for structured application events.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidOptions(_) => "invalid_options",
            Self::Timeout => "timeout",
            Self::Closed => "closed",
            Self::Spawn(_) => "spawn_failed",
            Self::Protocol(_) => "protocol",
            Self::Cancelled => "cancelled",
            Self::Unsupported(_) => "unsupported",
        }
    }
}

/// Observable outcome of one driver step.
///
/// Drivers return state; they never return secret bytes. The application
/// decides what to persist through its own credential store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginState {
    /// Show the URL and code, then call `poll` again after approval.
    ChallengeRequired(Challenge),
    /// The vendor reports a connected account.
    Authenticated(AccountInfo),
    /// The vendor declined or reports no connected account.
    Failed,
}

/// One vendor login ceremony.
///
/// Implementations are `Send` so applications can hold them across await
/// points on a Tokio runtime. The ceremony is native only and performs no
/// credential storage: `start` begins the handshake, `poll` waits for the
/// person to approve it, `account` re-reads the vendor presence, and
/// `cancel` stops the helper.
#[allow(async_fn_in_trait)]
pub trait LoginDriver: Send {
    /// Begin the handshake and report the first visible state.
    async fn start(&mut self) -> Result<LoginState, DriverError>;
    /// Wait for browser approval and project the vendor presence.
    async fn poll(&mut self) -> Result<LoginState, DriverError>;
    /// Re-read the vendor presence without starting a new handshake.
    async fn account(&mut self) -> Result<AccountInfo, DriverError>;
    /// Stop the helper; later steps report cancellation.
    async fn cancel(&mut self) -> Result<(), DriverError>;
}

/// Result returned by login driver operations.
pub type DriverResult<T> = Result<T, DriverError>;

/// Drive a device-code ceremony to its terminal state.
///
/// Calls `start`; when the vendor answers with [`LoginState::Authenticated`]
/// or [`LoginState::Failed`] that state is returned directly. When the vendor
/// answers with [`LoginState::ChallengeRequired`], the challenge is validated
/// with [`Challenge::validate`], handed to `on_challenge` for application
/// display, and then `poll` waits for browser approval whose outcome is
/// returned.
///
/// The application keeps all product copy and result mapping: it renders the
/// challenge inside `on_challenge` and translates the returned [`LoginState`]
/// into its own success/failure transitions. This helper only removes the
/// start/validate/poll sequencing every device-code consumer would otherwise
/// copy.
///
/// # Errors
///
/// Returns [`DriverError`] when `start` fails, the challenge has an
/// unsupported shape, or `poll` fails.
pub async fn complete_device_login<D: LoginDriver>(
    driver: &mut D,
    on_challenge: impl FnOnce(&Challenge),
) -> DriverResult<LoginState> {
    match driver.start().await? {
        state @ (LoginState::Authenticated(_) | LoginState::Failed) => Ok(state),
        LoginState::ChallengeRequired(challenge) => {
            challenge.validate()?;
            on_challenge(&challenge);
            driver.poll().await
        }
    }
}

/// Resolve a bare program name against the parent process `PATH`.
///
/// Drivers spawn helpers with a cleared minimal environment, so a relative
/// name like `python3` would otherwise resolve only against
/// `/usr/local/bin:/usr/bin:/bin`. Test fixtures and caller-supplied bare
/// names (for example a Nix-profile `python3` on CI workers) live outside
/// that minimal set. Absolute or slash-containing paths pass through
/// unchanged; bare names resolve to the first `PATH` match, falling back to
/// the original name so spawn still fails with the fixed `Spawn` message.
pub(crate) fn resolve_program(program: &PathBuf) -> PathBuf {
    let text = program.to_string_lossy();
    if text.is_empty() || text.contains('/') {
        return program.clone();
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(program);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    program.clone()
}

#[cfg(test)]
fn assert_send<T: Send>() {
    let _ = core::marker::PhantomData::<T>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_types_are_send() {
        assert_send::<Challenge>();
        assert_send::<AccountInfo>();
        assert_send::<DriverError>();
        assert_send::<LoginState>();
    }

    #[test]
    fn challenge_accepts_well_formed_input() {
        let challenge = Challenge::new(
            "https://auth.openai.com/codex/device".to_owned(),
            "ABCD-1234".to_owned(),
        );
        challenge.validate().unwrap();
    }

    #[test]
    fn challenge_rejects_bad_shapes() {
        for (url, code) in [
            ("", "ABCD-1234"),
            ("https://auth.openai.com/codex/device", ""),
            ("http://auth.openai.com/codex/device", "ABCD-1234"),
            ("https://", "ABCD-1234"),
            ("https:///path", "ABCD-1234"),
            ("https://auth.openai.com/codex/device", "abc"),
            (
                "https://auth.openai.com/codex/device",
                "this-code-is-far-too-long-for-a-device-step",
            ),
            ("https://auth.openai.com/codex/device", "bad code!"),
            ("https://auth.openai.com/code x/device", "ABCD-1234"),
        ] {
            let challenge = Challenge::new(url.to_owned(), code.to_owned());
            assert!(
                matches!(challenge.validate(), Err(DriverError::InvalidOptions(_))),
                "expected rejection for {url:?} {code:?}"
            );
        }
    }

    #[test]
    fn error_codes_stay_stable() {
        assert_eq!(DriverError::InvalidOptions("x").code(), "invalid_options");
        assert_eq!(DriverError::Timeout.code(), "timeout");
        assert_eq!(DriverError::Closed.code(), "closed");
        assert_eq!(DriverError::Spawn("x").code(), "spawn_failed");
        assert_eq!(DriverError::Protocol(-32603).code(), "protocol");
        assert_eq!(DriverError::Cancelled.code(), "cancelled");
        assert_eq!(DriverError::Unsupported("x").code(), "unsupported");
    }

    #[test]
    fn account_presence_requires_a_non_empty_label() {
        assert!(!AccountInfo::signed_out().connected());
        assert!(!AccountInfo::new(None).connected());
        assert!(!AccountInfo::new(Some(String::new())).connected());
        assert!(AccountInfo::new(Some("user@example.com".to_owned())).connected());
    }

    struct StubDriver {
        start: LoginState,
        poll: LoginState,
        polled: bool,
    }

    impl StubDriver {
        fn new(start: LoginState, poll: LoginState) -> Self {
            Self {
                start,
                poll,
                polled: false,
            }
        }
    }

    impl LoginDriver for StubDriver {
        async fn start(&mut self) -> DriverResult<LoginState> {
            Ok(self.start.clone())
        }

        async fn poll(&mut self) -> DriverResult<LoginState> {
            self.polled = true;
            Ok(self.poll.clone())
        }

        async fn account(&mut self) -> DriverResult<AccountInfo> {
            Ok(AccountInfo::signed_out())
        }

        async fn cancel(&mut self) -> DriverResult<()> {
            Ok(())
        }
    }

    fn valid_challenge() -> Challenge {
        Challenge::new(
            "https://auth.openai.com/codex/device".to_owned(),
            "ABCD-1234".to_owned(),
        )
    }

    #[tokio::test]
    async fn device_login_returns_immediate_states_without_a_challenge() {
        for state in [
            LoginState::Authenticated(AccountInfo::new(Some("user@example.com".to_owned()))),
            LoginState::Failed,
        ] {
            let mut driver = StubDriver::new(state.clone(), LoginState::Failed);
            let outcome = complete_device_login(&mut driver, |_| panic!("no challenge expected"))
                .await
                .expect("immediate states must succeed");
            assert_eq!(outcome, state);
            assert!(!driver.polled);
        }
    }

    #[tokio::test]
    async fn device_login_displays_then_polls_a_valid_challenge() {
        let challenge = valid_challenge();
        let authenticated =
            LoginState::Authenticated(AccountInfo::new(Some("user@example.com".to_owned())));
        let mut driver = StubDriver::new(
            LoginState::ChallengeRequired(challenge.clone()),
            authenticated.clone(),
        );
        let outcome = complete_device_login(&mut driver, |shown| {
            assert_eq!(shown, &challenge);
        })
        .await
        .expect("polled approval must succeed");
        assert_eq!(outcome, authenticated);
        assert!(driver.polled);
    }

    #[tokio::test]
    async fn device_login_rejects_a_bad_challenge_before_display_or_poll() {
        let mut driver = StubDriver::new(
            LoginState::ChallengeRequired(Challenge::new(
                "http://auth.openai.com/codex/device".to_owned(),
                "ABCD-1234".to_owned(),
            )),
            LoginState::Failed,
        );
        let error =
            complete_device_login(&mut driver, |_| panic!("bad challenge must not display"))
                .await
                .expect_err("bad challenge shape must fail");
        assert!(matches!(error, DriverError::InvalidOptions(_)));
        assert!(!driver.polled);
    }
}

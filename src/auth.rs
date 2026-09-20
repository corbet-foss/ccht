//! Provider login state and credential-store abstraction.
//!
//! Structural inspiration comes from the provider/auth split in Zed
//! (GPL-3.0): uniform per-provider auth state, an application-supplied
//! credential store, and key lifecycle with environment override. This
//! module is reimplemented from scratch for LGPL use; no Zed code is
//! contained here.
//!
//! Boundary, unchanged: the store sees opaque bytes keyed by service URL
//! and never learns what they unlock; applications own login ceremonies,
//! UI, and where the store persists (OS keychain natively, browser storage
//! on the web). Nothing here performs network I/O.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Login state of one provider as observed by the application.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    /// Not checked yet (for example, before the store was read).
    #[default]
    Unknown,
    /// Credentials exist. The account label is display-only provenance and
    /// must never be treated as an authentication proof by itself.
    Authenticated {
        /// Display-only account label (username, email, or subscription name).
        account: Option<String>,
    },
    /// No credentials and no other evidence of a login.
    Unauthenticated,
}

impl AuthState {
    /// Whether routine work may proceed without prompting for login first.
    #[must_use]
    pub fn authenticated(&self) -> bool {
        matches!(self, Self::Authenticated { .. })
    }
}

/// Storage failure of a [`CredentialsProvider`].
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum AuthError {
    /// The backing store failed (keychain locked, disk full, quota hit).
    #[error("credential store failed: {0}")]
    Store(String),
    /// The operation makes no sense for this store (for example, writing to
    /// a read-only native-agent home owned by the agent itself).
    #[error("operation not supported by this credential store")]
    NotSupported,
}

/// Opaque credential: an account label plus secret bytes.
///
/// Stores must treat both fields as opaque. The label exists so UIs can say
/// *who* is signed in without ever unlocking the secret.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Credential {
    /// Display-only account label (username, email, or subscription name).
    pub account: String,
    /// Secret bytes. Never logged, never embedded in errors or fixtures.
    pub secret: Vec<u8>,
}

/// Application-supplied credential persistence.
///
/// One method triple per service URL. Native applications back this with the
/// OS keychain; web applications with origin-scoped browser storage; tests
/// with [`MemoryCredentialsProvider`].
pub trait CredentialsProvider: Send + Sync {
    /// Read stored credentials, if any, for a service URL.
    fn read_credentials<'a>(
        &'a self,
        service: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Credential>, AuthError>> + Send + 'a>>;

    /// Persist credentials for a service URL, replacing any previous entry.
    fn write_credentials<'a>(
        &'a self,
        service: &'a str,
        credential: &'a Credential,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuthError>> + Send + 'a>>;

    /// Remove stored credentials for a service URL. Missing entries are not
    /// an error.
    fn delete_credentials<'a>(
        &'a self,
        service: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuthError>> + Send + 'a>>;
}

/// In-memory [`CredentialsProvider`] for tests and local development.
///
/// Never ships credentials anywhere; drop it when the test ends.
#[derive(Debug, Default)]
pub struct MemoryCredentialsProvider {
    entries: Mutex<HashMap<String, Credential>>,
}

impl MemoryCredentialsProvider {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialsProvider for MemoryCredentialsProvider {
    fn read_credentials<'a>(
        &'a self,
        service: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Credential>, AuthError>> + Send + 'a>> {
        let found = match self.entries.lock() {
            Ok(entries) => entries.get(service).cloned(),
            Err(_) => {
                return Box::pin(async move { Err(AuthError::Store("lock poisoned".into())) });
            }
        };
        Box::pin(async move { Ok(found) })
    }

    fn write_credentials<'a>(
        &'a self,
        service: &'a str,
        credential: &'a Credential,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuthError>> + Send + 'a>> {
        let result = self
            .entries
            .lock()
            .map(|mut entries| {
                entries.insert(service.to_owned(), credential.clone());
            })
            .map_err(|_| AuthError::Store("lock poisoned".into()));
        Box::pin(async move { result })
    }

    fn delete_credentials<'a>(
        &'a self,
        service: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), AuthError>> + Send + 'a>> {
        let result = self
            .entries
            .lock()
            .map(|mut entries| {
                entries.remove(service);
            })
            .map_err(|_| AuthError::Store("lock poisoned".into()));
        Box::pin(async move { result })
    }
}

/// Lifecycle of one provider's key: environment override wins, otherwise the
/// store decides. Mirrors the semantics applications need for a Zed-style
/// settings surface: `is_authenticated()` for badges, `store()` for the
/// save action, `reset()` for sign-out, `load_if_needed()` for lazy boot.
#[derive(Clone, Debug, Default)]
pub struct ApiKeyState {
    key: Option<String>,
    from_env_var: bool,
    // The environment override is unavailable on Wasm; the field is still set
    // through `new` on every target so construction stays uniform.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    env_var_name: Option<&'static str>,
    loaded_service: Option<String>,
}

impl ApiKeyState {
    /// Unloaded state, optionally honoring an environment variable override.
    #[must_use]
    pub fn new(env_var_name: Option<&'static str>) -> Self {
        Self {
            env_var_name,
            ..Self::default()
        }
    }

    /// Whether routine work may proceed: an in-memory key is present.
    #[must_use]
    pub fn has_key(&self) -> bool {
        self.key.as_deref().is_some_and(|key| !key.is_empty())
    }

    /// Whether the current key came from the environment (read-only: reset
    /// is disabled and the UI must say so instead of failing silently).
    #[must_use]
    pub fn is_from_env_var(&self) -> bool {
        self.from_env_var
    }

    /// The in-memory key, if any. Callers forward it to request signing;
    /// they never persist or display it.
    #[must_use]
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    /// Load from environment or store unless this exact service already was.
    /// Returns the resulting [`AuthState`].
    ///
    /// The environment override is unavailable on Wasm (browsers expose no
    /// process environment); there the store alone decides.
    pub async fn load_if_needed(
        &mut self,
        service: &str,
        store: &(dyn CredentialsProvider + Send + Sync),
    ) -> Result<AuthState, AuthError> {
        if self.loaded_service.as_deref() == Some(service) {
            return Ok(self.snapshot());
        }
        #[cfg(not(target_family = "wasm"))]
        if let Some(name) = self.env_var_name
            && let Ok(value) = std::env::var(name)
            && !value.trim().is_empty()
        {
            self.key = Some(value);
            self.from_env_var = true;
            self.loaded_service = Some(service.to_owned());
            return Ok(self.snapshot());
        }
        self.from_env_var = false;
        self.key = store
            .read_credentials(service)
            .await?
            .and_then(|credential| {
                String::from_utf8(credential.secret)
                    .ok()
                    .filter(|secret| !secret.trim().is_empty())
            });
        self.loaded_service = Some(service.to_owned());
        Ok(self.snapshot())
    }

    /// Forget a service change: the next `load_if_needed` reads again.
    pub fn handle_service_change(&mut self, service: &str) {
        if self.loaded_service.as_deref() != Some(service) {
            self.key = None;
            self.from_env_var = false;
            self.loaded_service = None;
        }
    }

    /// Persist a key (or `None` to sign out) through the store. Refuses while
    /// the key comes from the environment: callers must unset the variable.
    pub async fn store(
        &mut self,
        service: &str,
        key: Option<String>,
        store: &(dyn CredentialsProvider + Send + Sync),
    ) -> Result<AuthState, AuthError> {
        if self.from_env_var {
            return Err(AuthError::NotSupported);
        }
        match key
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
        {
            Some(key) => {
                store
                    .write_credentials(
                        service,
                        &Credential {
                            account: String::new(),
                            secret: key.into_bytes(),
                        },
                    )
                    .await?;
                self.key = store
                    .read_credentials(service)
                    .await?
                    .and_then(|credential| String::from_utf8(credential.secret).ok());
            }
            None => {
                store.delete_credentials(service).await?;
                self.key = None;
            }
        }
        self.loaded_service = Some(service.to_owned());
        Ok(self.snapshot())
    }

    fn snapshot(&self) -> AuthState {
        if self.has_key() {
            AuthState::Authenticated { account: None }
        } else {
            AuthState::Unauthenticated
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    /// Minimal single-threaded executor so core tests need no async runtime.
    fn block_on<F: Future>(mut future: F) -> F::Output {
        // SAFETY: all waker callbacks ignore the null data pointer.
        unsafe fn clone_raw(_: *const ()) -> RawWaker {
            RawWaker::new(std::ptr::null(), &VTABLE)
        }
        unsafe fn noop_raw(_: *const ()) {}
        static VTABLE: RawWakerVTable =
            RawWakerVTable::new(clone_raw, noop_raw, noop_raw, noop_raw);
        // SAFETY: the waker never dereferences its null data pointer.
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut context = Context::from_waker(&waker);
        // SAFETY: the future is never moved after pinning for the poll loop.
        let mut future = unsafe { Pin::new_unchecked(&mut future) };
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    async fn roundtrip(store: &MemoryCredentialsProvider) {
        let credential = Credential {
            account: "Pro".into(),
            secret: b"secret".to_vec(),
        };
        assert_eq!(store.read_credentials("svc").await.unwrap(), None);
        store.write_credentials("svc", &credential).await.unwrap();
        assert_eq!(
            store.read_credentials("svc").await.unwrap(),
            Some(credential)
        );
        store.delete_credentials("svc").await.unwrap();
        assert_eq!(store.read_credentials("svc").await.unwrap(), None);
        // Deleting a missing entry is not an error.
        store.delete_credentials("svc").await.unwrap();
    }

    #[test]
    fn memory_store_roundtrips() {
        block_on(roundtrip(&MemoryCredentialsProvider::new()));
    }

    #[test]
    fn api_key_state_starts_unauthenticated() {
        let state = ApiKeyState::new(None);
        assert!(!state.has_key());
        assert!(!state.is_from_env_var());
        assert_eq!(state.snapshot(), AuthState::Unauthenticated);
    }

    #[test]
    fn api_key_state_save_and_sign_out() {
        block_on(async {
            let store = MemoryCredentialsProvider::new();
            let mut state = ApiKeyState::new(None);
            let seen = state.load_if_needed("svc", &store).await.unwrap();
            assert_eq!(seen, AuthState::Unauthenticated);
            let seen = state
                .store("svc", Some("  key  ".into()), &store)
                .await
                .unwrap();
            assert_eq!(seen, AuthState::Authenticated { account: None });
            assert!(state.has_key());
            // Blank input signs out instead of storing whitespace.
            let seen = state
                .store("svc", Some("   ".into()), &store)
                .await
                .unwrap();
            assert_eq!(seen, AuthState::Unauthenticated);
            assert!(!state.has_key());
        });
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn api_key_state_prefers_environment_and_locks_reset() {
        block_on(async {
            // SAFETY: single-threaded test with no other environment readers.
            unsafe { std::env::set_var("CCHT_TEST_API_KEY", "env-key") };
            let store = MemoryCredentialsProvider::new();
            let mut state = ApiKeyState::new(Some("CCHT_TEST_API_KEY"));
            let seen = state.load_if_needed("svc", &store).await.unwrap();
            assert_eq!(seen, AuthState::Authenticated { account: None });
            assert!(state.is_from_env_var());
            assert_eq!(
                state.store("svc", None, &store).await.unwrap_err(),
                AuthError::NotSupported
            );
            // SAFETY: restores the pre-test environment; see above.
            unsafe { std::env::remove_var("CCHT_TEST_API_KEY") };
        });
    }

    #[test]
    fn api_key_state_reloads_on_service_change() {
        block_on(async {
            let store = MemoryCredentialsProvider::new();
            let mut state = ApiKeyState::new(None);
            state
                .store("a", Some("key-a".into()), &store)
                .await
                .unwrap();
            assert!(state.has_key());
            state.handle_service_change("b");
            assert!(!state.has_key());
            let seen = state.load_if_needed("b", &store).await.unwrap();
            assert_eq!(seen, AuthState::Unauthenticated);
        });
    }

    #[test]
    fn api_key_state_keeps_key_for_same_service() {
        block_on(async {
            let store = MemoryCredentialsProvider::new();
            let mut state = ApiKeyState::new(None);
            state
                .store("a", Some("key-a".into()), &store)
                .await
                .unwrap();
            state.handle_service_change("a");
            assert!(state.has_key());
            // A repeated load for the same service returns the cached snapshot.
            let seen = state.load_if_needed("a", &store).await.unwrap();
            assert_eq!(seen, AuthState::Authenticated { account: None });
        });
    }

    #[test]
    fn auth_state_reports_readiness() {
        assert!(!AuthState::Unknown.authenticated());
        assert!(!AuthState::Unauthenticated.authenticated());
        assert!(AuthState::Authenticated { account: None }.authenticated());
    }
}

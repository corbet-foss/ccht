//! Environment allowlist profile for spawning agent processes.
//!
//! Child processes inherit only explicitly allowed variables; every other
//! value is blanked to an empty string so secrets from the parent process
//! cannot silently replace native agent login. Paths and locale names are
//! not secrets, while tokens and keys must never be forwarded.

use std::collections::BTreeMap;

/// Environment allowlist applied to agent child processes.
///
/// [`EnvProfile::Strict`] forwards a minimal locale, home, and certificate
/// set. [`EnvProfile::Permissive`] is a documented superset that additionally
/// forwards common locale and certificate-bundle variables; both profiles
/// blank every other variable to an empty string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvProfile {
    /// Minimal allowlist for spawned agent processes.
    Strict,
    /// Strict set plus extra locale and certificate-bundle variables.
    Permissive,
}

impl EnvProfile {
    /// Minimal profile: `PATH`, `HOME`, `XDG_*`, `LANG`/`LC_*`, `TZ`,
    /// `TERM`, `TMPDIR`, `SSL_CERT_*`, and `NODE_EXTRA_CA_CERTS`.
    #[must_use]
    pub fn strict() -> Self {
        Self::Strict
    }

    /// Superset of [`EnvProfile::strict`] that additionally allows
    /// `LANGUAGE`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, and `CA_BUNDLE`.
    ///
    /// These extras are `TMPDIR`-independent locale and certificate path
    /// variables; no secret-bearing variable is added.
    #[must_use]
    pub fn permissive() -> Self {
        Self::Permissive
    }

    /// Whether this profile forwards `key` with its value intact.
    fn allows(&self, key: &str) -> bool {
        if is_strict_allowed(key) {
            return true;
        }
        match self {
            Self::Strict => false,
            Self::Permissive => is_permissive_extra(key),
        }
    }

    /// Copy `env`, keeping allowed values and blanking the rest to empty.
    ///
    /// All input keys are preserved; disallowed values become `""` so the
    /// child cannot inherit ambient secrets through the environment.
    #[must_use]
    pub fn apply(&self, env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        env.iter()
            .map(|(key, value)| {
                if self.allows(key) {
                    (key.clone(), value.clone())
                } else {
                    (key.clone(), String::new())
                }
            })
            .collect()
    }

    /// Filter [`crate::native::AgentCommand`] environment in place.
    ///
    /// Allowed entries keep their values; every other entry is blanked to
    /// an empty string. The program and arguments are left unchanged.
    pub fn apply_to_command(&self, cmd: &mut crate::native::AgentCommand) {
        cmd.env = self.apply(&cmd.env);
    }
}

/// Strict allowlist: exact names plus `XDG_`, `LC_`, and `SSL_CERT_` prefixes.
fn is_strict_allowed(key: &str) -> bool {
    match key {
        "PATH" | "HOME" | "LANG" | "TZ" | "TERM" | "TMPDIR" | "NODE_EXTRA_CA_CERTS" => true,
        _ => key.starts_with("XDG_") || key.starts_with("LC_") || key.starts_with("SSL_CERT_"),
    }
}

/// Permissive extras beyond the strict set; still locale/cert paths only.
fn is_permissive_extra(key: &str) -> bool {
    matches!(
        key,
        "LANGUAGE" | "REQUESTS_CA_BUNDLE" | "CURL_CA_BUNDLE" | "CA_BUNDLE"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::AgentCommand;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn strict_keeps_allowlist_and_blanks_secrets() {
        let profile = EnvProfile::strict();
        let input = env(&[
            ("PATH", "/usr/bin"),
            ("HOME", "/home/app"),
            ("XDG_CONFIG_HOME", "/home/app/.config"),
            ("LANG", "en_US.UTF-8"),
            ("LC_MESSAGES", "en_US.UTF-8"),
            ("TZ", "UTC"),
            ("TERM", "xterm"),
            ("TMPDIR", "/tmp"),
            ("SSL_CERT_FILE", "/etc/ssl/certs.pem"),
            ("NODE_EXTRA_CA_CERTS", "/etc/ssl/certs.pem"),
            ("OPENAI_API_KEY", "super-secret"),
            ("AWS_SECRET_ACCESS_KEY", "super-secret"),
            ("GITHUB_TOKEN", "super-secret"),
        ]);
        let filtered = profile.apply(&input);
        for (key, value) in &input {
            if is_strict_allowed(key) {
                assert_eq!(filtered.get(key).map(String::as_str), Some(value.as_str()));
            } else {
                assert_eq!(
                    filtered.get(key).map(String::as_str),
                    Some(""),
                    "expected blanking for {key}"
                );
            }
        }
        assert_eq!(
            filtered.get("OPENAI_API_KEY").map(String::as_str),
            Some(""),
            "secret variables must be blanked"
        );
    }

    #[test]
    fn permissive_is_a_documented_superset_of_strict() {
        let strict = EnvProfile::strict();
        let permissive = EnvProfile::permissive();
        let strict_keys = [
            "PATH",
            "HOME",
            "XDG_DATA_HOME",
            "LANG",
            "LC_ALL",
            "TZ",
            "TERM",
            "TMPDIR",
            "SSL_CERT_DIR",
            "NODE_EXTRA_CA_CERTS",
        ];
        let extra_keys = [
            "LANGUAGE",
            "REQUESTS_CA_BUNDLE",
            "CURL_CA_BUNDLE",
            "CA_BUNDLE",
        ];
        for key in strict_keys {
            let input = env(&[(key, "value")]);
            assert_eq!(
                strict.apply(&input).get(key).map(String::as_str),
                Some("value")
            );
            assert_eq!(
                permissive.apply(&input).get(key).map(String::as_str),
                Some("value"),
                "permissive must keep every strict variable"
            );
        }
        for key in extra_keys {
            let input = env(&[(key, "value")]);
            assert_eq!(
                strict.apply(&input).get(key).map(String::as_str),
                Some(""),
                "extra variable must not be in the strict set"
            );
            assert_eq!(
                permissive.apply(&input).get(key).map(String::as_str),
                Some("value")
            );
        }
    }

    #[test]
    fn permissive_still_blanks_secrets() {
        let filtered = EnvProfile::permissive().apply(&env(&[
            ("OPENAI_API_KEY", "super-secret"),
            ("AWS_SESSION_TOKEN", "super-secret"),
            ("ANTHROPIC_AUTH_TOKEN", "super-secret"),
            ("LANGUAGE", "en"),
        ]));
        assert_eq!(filtered.get("OPENAI_API_KEY").map(String::as_str), Some(""));
        assert_eq!(
            filtered.get("AWS_SESSION_TOKEN").map(String::as_str),
            Some("")
        );
        assert_eq!(
            filtered.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str),
            Some("")
        );
        assert_eq!(filtered.get("LANGUAGE").map(String::as_str), Some("en"));
    }

    #[test]
    fn apply_to_command_filters_env_without_touching_program() {
        let mut command = AgentCommand::new("agent");
        command.env.insert("PATH".into(), "/usr/bin".into());
        command
            .env
            .insert("OPENAI_API_KEY".into(), "super-secret".into());
        EnvProfile::strict().apply_to_command(&mut command);
        assert_eq!(
            command.program.to_str(),
            Some("agent"),
            "program must be unchanged"
        );
        assert_eq!(
            command.env.get("PATH").map(String::as_str),
            Some("/usr/bin")
        );
        assert_eq!(
            command.env.get("OPENAI_API_KEY").map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn agent_command_debug_shows_keys_only() {
        let mut command = AgentCommand::new("agent");
        command
            .env
            .insert("OPENAI_API_KEY".into(), "super-secret-value".into());
        EnvProfile::strict().apply_to_command(&mut command);
        let rendered = format!("{command:?}");
        assert!(
            rendered.contains("OPENAI_API_KEY"),
            "debug output should keep environment keys"
        );
        assert!(
            !rendered.contains("super-secret-value"),
            "debug output must never leak environment values"
        );
    }

    #[test]
    fn empty_environment_stays_empty_and_names_are_case_sensitive() {
        let empty: BTreeMap<String, String> = BTreeMap::new();
        assert!(EnvProfile::strict().apply(&empty).is_empty());
        assert!(EnvProfile::permissive().apply(&empty).is_empty());
        // Allowlist names are exact; lowercase variants must be blanked.
        let input = env(&[("path", "/usr/bin"), ("Path", "/usr/bin")]);
        let filtered = EnvProfile::strict().apply(&input);
        assert_eq!(filtered.get("path").map(String::as_str), Some(""));
        assert_eq!(filtered.get("Path").map(String::as_str), Some(""));
    }
}

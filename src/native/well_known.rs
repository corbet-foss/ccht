//! Well-known ACP agent executables behind one pure-data table.
//!
//! Each entry names the maintained upstream executable and the exact
//! arguments that select its ACP mode. This module performs no I/O, installs
//! nothing, stores no credentials, and knows no product labels, login
//! commands, or managed-versus-external splits: applications own which agents
//! they support, how runtimes are provisioned, and how sign-in is presented.
//! Callers turn an entry into a spawn command with
//! [`WellKnownAgent::command`], then connect through
//! [`super::NativeClient::connect`] as usual.
//!
//! The argument shapes follow each vendor's own CLI contract for exposing
//! ACP over stdio (for example `opencode acp` from the MIT-licensed OpenCode
//! CLI). They are recorded here as invocation facts, not copied
//! implementation.

use super::AgentCommand;

/// One maintained upstream agent and the arguments selecting its ACP mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WellKnownAgent {
    /// Stable short identity (for example `"codex"`).
    pub id: &'static str,
    /// Executable name as resolved from `PATH` or an installed location.
    pub program: &'static str,
    /// Arguments selecting ACP mode, passed without shell parsing.
    pub args: &'static [&'static str],
}

impl WellKnownAgent {
    /// Spawn command for this agent with no environment overrides.
    #[must_use]
    pub fn command(self) -> AgentCommand {
        AgentCommand::new(self.program).args(self.args.iter().copied())
    }
}

/// Every well-known agent, in a stable order.
pub const ALL: &[WellKnownAgent] = &[
    WellKnownAgent {
        id: "codex",
        program: "codex-acp",
        args: &[],
    },
    WellKnownAgent {
        id: "gemini",
        program: "gemini",
        args: &["--acp"],
    },
    WellKnownAgent {
        id: "copilot",
        program: "copilot",
        args: &["--acp", "--stdio"],
    },
    WellKnownAgent {
        id: "claude",
        program: "claude-agent-acp",
        args: &[],
    },
    WellKnownAgent {
        id: "opencode",
        program: "opencode",
        args: &["acp"],
    },
];

/// Look up a well-known agent by its [`WellKnownAgent::id`].
#[must_use]
pub fn find(id: &str) -> Option<WellKnownAgent> {
    ALL.iter().find(|agent| agent.id == id).copied()
}

/// Spawn command for a well-known agent id, if known.
#[must_use]
pub fn command(id: &str) -> Option<AgentCommand> {
    find(id).map(WellKnownAgent::command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_builds_a_valid_stdio_command() {
        assert!(!ALL.is_empty());
        for agent in ALL {
            let command = agent.command();
            assert_eq!(
                command.program.to_str(),
                Some(agent.program),
                "program mismatch for {}",
                agent.id
            );
            let expected: Vec<String> = agent.args.iter().map(ToString::to_string).collect();
            assert_eq!(command.args, expected, "args mismatch for {}", agent.id);
            command
                .validate_stdio()
                .expect("well-known entries must be valid stdio");
        }
    }

    #[test]
    fn ids_are_unique_and_findable() {
        let mut seen = std::collections::HashSet::new();
        for agent in ALL {
            assert!(
                seen.insert(agent.id),
                "duplicate well-known id {}",
                agent.id
            );
            assert_eq!(find(agent.id), Some(*agent));
            assert!(command(agent.id).is_some());
        }
        assert_eq!(find("unknown-agent"), None);
        assert!(command("unknown-agent").is_none());
    }

    #[test]
    fn argument_shapes_match_vendor_acp_contracts() {
        assert_eq!(find("codex").expect("codex entry").args, &[] as &[&str]);
        assert_eq!(find("claude").expect("claude entry").args, &[] as &[&str]);
        assert_eq!(find("opencode").expect("opencode entry").args, &["acp"]);
        assert_eq!(find("gemini").expect("gemini entry").args, &["--acp"]);
        assert_eq!(
            find("copilot").expect("copilot entry").args,
            &["--acp", "--stdio"]
        );
    }
}

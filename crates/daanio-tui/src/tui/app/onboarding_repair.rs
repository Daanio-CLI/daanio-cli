//! Agent-assisted onboarding recovery.
//!
//! When a first-run login import/validation fails, the recovery screen offers
//! to hand the problem to an AI coding agent the user already uses. This module:
//!
//!   1. Guesses the user's *preferred* agent by looking at which external CLI's
//!      credentials/transcripts were touched most recently
//!      ([`detect_preferred_repair_agent`]).
//!   2. Builds an agent-friendly *repair brief* ([`build_repair_brief`]) that
//!      states the failure, points at the log file, and lists the exact
//!      non-interactive commands the agent can run to diagnose and fix the
//!      login (`daanio auth-test --provider daanio` and
//!      `daanio login --provider daanio`).
//!
//! The brief is plain text so it works whether we copy it to the clipboard,
//! show it on screen, or seed an agent's prompt.

use std::path::PathBuf;
use std::time::SystemTime;

/// An external coding agent daanio can hand a repair task to. These map to the
/// real CLI binaries users run, so the brief can name the exact command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepairAgent {
    Codex,
    ClaudeCode,
    Copilot,
    Cursor,
    Gemini,
    Pi,
    OpenCode,
}

impl RepairAgent {
    /// Human-readable name for the recovery prompt ("have Codex help fix this").
    pub(crate) fn label(self) -> &'static str {
        match self {
            RepairAgent::Codex => "Codex",
            RepairAgent::ClaudeCode => "Claude Code",
            RepairAgent::Copilot => "Copilot",
            RepairAgent::Cursor => "Cursor",
            RepairAgent::Gemini => "Gemini",
            RepairAgent::Pi => "Pi",
            RepairAgent::OpenCode => "OpenCode",
        }
    }

    /// The shell command that launches this agent in the current directory, if
    /// it is a CLI we can name. The user can paste the brief into it.
    pub(crate) fn launch_command(self) -> &'static str {
        match self {
            RepairAgent::Codex => "codex",
            RepairAgent::ClaudeCode => "claude",
            RepairAgent::Copilot => "gh copilot",
            RepairAgent::Cursor => "cursor-agent",
            RepairAgent::Gemini => "gemini",
            RepairAgent::Pi => "pi",
            RepairAgent::OpenCode => "opencode",
        }
    }

    /// The credential files whose mtime signals "the user used this agent
    /// recently". Relative to the (sandbox-aware) external home.
    fn credential_rel_paths(self) -> &'static [&'static str] {
        match self {
            RepairAgent::Codex => &[".codex/auth.json"],
            RepairAgent::ClaudeCode => &[".claude/.credentials.json"],
            RepairAgent::Copilot => &[
                ".config/github-copilot/hosts.json",
                ".config/github-copilot/apps.json",
            ],
            RepairAgent::Cursor => &[".cursor/auth.json"],
            RepairAgent::Gemini => &[".gemini/oauth_creds.json"],
            RepairAgent::Pi => &[".pi/agent/auth.json"],
            RepairAgent::OpenCode => &[".local/share/opencode/auth.json"],
        }
    }

    /// All candidate agents, most "primary" first (used only as a stable tie
    /// breaker when two credentials share an mtime).
    fn all() -> [RepairAgent; 7] {
        [
            RepairAgent::Codex,
            RepairAgent::ClaudeCode,
            RepairAgent::Copilot,
            RepairAgent::Cursor,
            RepairAgent::Gemini,
            RepairAgent::Pi,
            RepairAgent::OpenCode,
        ]
    }
}

/// Resolve a path under the (sandbox-aware) external home so detection honors
/// `DAANIO_HOME`/external isolation, matching the onboarding import detectors.
fn external_home_path(rel: &str) -> PathBuf {
    crate::storage::user_home_path(rel)
        .ok()
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rel)))
        .unwrap_or_else(|| PathBuf::from(rel))
}

/// The most recent modification time across an agent's credential files, if any
/// exist and are non-empty.
fn agent_last_used(agent: RepairAgent) -> Option<SystemTime> {
    agent
        .credential_rel_paths()
        .iter()
        .filter_map(|rel| {
            let path = external_home_path(rel);
            let meta = std::fs::metadata(&path).ok()?;
            if meta.is_file() && meta.len() > 0 {
                meta.modified().ok()
            } else {
                None
            }
        })
        .max()
}

/// Guess the user's preferred repair agent: the external CLI whose credentials
/// were modified most recently. Returns `None` when no known agent credential
/// is present (so the recovery screen hides the "ask an agent" option).
pub(crate) fn detect_preferred_repair_agent() -> Option<RepairAgent> {
    if std::env::var_os("DAANIO_FIRST_PARTY_ONLY").is_some() {
        return None;
    }
    RepairAgent::all()
        .into_iter()
        .filter_map(|agent| agent_last_used(agent).map(|t| (agent, t)))
        .max_by_key(|(_, t)| *t)
        .map(|(agent, _)| agent)
}

/// Build the agent-friendly repair brief: a plain-text task description the user
/// can paste into their agent (or we can copy to the clipboard). It states the
/// failure, points at the log file, and lists the exact non-interactive
/// commands the agent runs to diagnose and fix the login.
///
/// `failure` is the short reason already shown on the recovery screen.
/// `provider_hint` is retained for compatibility; public builds always repair
/// the Daanio gateway login.
pub(crate) fn build_repair_brief(
    agent: Option<RepairAgent>,
    failure: &str,
    _provider_hint: Option<&str>,
) -> String {
    let log_line = daanio_logging::log_path()
        .map(|p| format!("Logs:    {}", p.display()))
        .unwrap_or_else(|| "Logs:    ~/.daanio/logs/daanio-<date>.log".to_string());

    let mut brief = String::new();
    brief.push_str(
        "Daanio CLI could not validate the user's browser-authorized Daanio gateway credential during \
first-run onboarding. Please fix the Daanio login for the user.\n\n",
    );
    brief.push_str(&format!("Failure: {}\n", failure.trim()));
    brief.push_str(&format!("{log_line}\n\n"));

    brief.push_str("Diagnose (machine-readable, exit/JSON tells you what's wrong):\n");
    brief.push_str("  daanio auth-test --provider daanio --json\n");
    brief.push_str("  daanio auth doctor   # human-readable, structured recovery steps\n\n");

    brief.push_str("Fix using secure Daanio browser sign-in:\n");
    brief.push_str("  daanio login --provider daanio\n");
    brief.push_str(
        "Do not request or enter OpenAI, Anthropic, Google, OpenRouter, or other upstream-provider credentials. Daanio manages those providers server-side.\n\n",
    );

    brief.push_str("Re-validate (success means done):\n");
    brief.push_str(
        "  daanio auth-test --provider daanio --json   # success:true in the JSON = fixed\n\n",
    );

    if let Some(agent) = agent {
        brief.push_str(&format!(
            "You appear to be {}; you can run these commands directly. ",
            agent.label()
        ));
    }
    brief.push_str(
        "When auth-test reports success, tell the user to restart daanio (or press Enter on \
the onboarding screen to sign in to Daanio again).\n",
    );
    brief
}

/// Stable path where the latest onboarding repair brief is written, so a helper
/// agent launched in this directory can simply `cat` it without the user having
/// to paste anything. Lives under the daanio home so it honors `DAANIO_HOME`.
pub(crate) fn repair_brief_path() -> Option<PathBuf> {
    crate::storage::daanio_dir()
        .ok()
        .map(|dir| dir.join("onboarding-repair-brief.txt"))
}

/// Write the repair brief to [`repair_brief_path`] so an agent can read it
/// directly. Returns the path on success.
pub(crate) fn persist_repair_brief(brief: &str) -> Option<PathBuf> {
    let path = repair_brief_path()?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, brief).ok()?;
    Some(path)
}

/// One-line status to show after preparing the brief, naming the agent and how
/// to use the copied brief.
pub(crate) fn repair_brief_status(agent: Option<RepairAgent>, copied: bool) -> String {
    let copied_part = if copied {
        "Repair brief copied to clipboard"
    } else {
        "Repair brief ready (clipboard unavailable)"
    };
    match agent {
        Some(agent) => format!(
            "{copied_part} - paste it into {} ({})",
            agent.label(),
            agent.launch_command()
        ),
        None => format!("{copied_part} - paste it into your coding agent"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_lists_the_non_interactive_commands_and_log_path() {
        let brief = build_repair_brief(
            Some(RepairAgent::Codex),
            "the saved credential was rejected",
            Some("openai"),
        );
        // States the failure verbatim.
        assert!(
            brief.contains("the saved credential was rejected"),
            "{brief}"
        );
        // The exact agent-runnable commands.
        assert!(
            brief.contains("daanio auth-test --provider daanio --json"),
            "{brief}"
        );
        assert!(brief.contains("daanio login --provider daanio"), "{brief}");
        assert!(!brief.contains("daanio provider add"), "{brief}");
        assert!(brief.contains("upstream-provider credentials"), "{brief}");
        // Points at the logs.
        assert!(brief.contains("Logs:"), "{brief}");
        // Names the detected agent so it knows it can act directly.
        assert!(brief.contains("Codex"), "{brief}");
    }

    #[test]
    fn brief_without_provider_or_agent_is_still_actionable() {
        let brief = build_repair_brief(None, "unknown failure", None);
        assert!(
            brief.contains("daanio auth-test --provider daanio --json"),
            "{brief}"
        );
        assert!(brief.contains("daanio auth doctor"), "{brief}");
        // No agent label, but still tells the user what to do.
        assert!(brief.contains("restart daanio"), "{brief}");
    }

    #[test]
    fn status_names_the_agent_and_launch_command() {
        let s = repair_brief_status(Some(RepairAgent::ClaudeCode), true);
        assert!(s.contains("Claude Code"), "{s}");
        assert!(s.contains("claude"), "{s}");
        assert!(s.contains("copied"), "{s}");
        let none = repair_brief_status(None, false);
        assert!(none.contains("coding agent"), "{none}");
    }
}

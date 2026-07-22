//! Mapping from parsed CLI arguments to an initial process title.
//!
//! This logic depends on the clap `Args`/`Command` types defined in `cli`, so
//! it lives in the CLI layer. The low-level title-setting primitives it uses
//! (`compact_process_title`, `session_name`, `set_title`) live in the
//! `process_title` core module.

use crate::cli::args::{AmbientCommand, Args, Command};
use crate::process_title::{compact_process_title, session_name, set_title};

pub(crate) fn initial_title(args: &Args) -> String {
    match &args.command {
        Some(Command::Serve { .. }) => "daanio:server".to_string(),
        Some(Command::Acp) => "daanio acp".to_string(),
        Some(Command::Server { .. }) => "daanio server".to_string(),
        Some(Command::Connect) => "daanio:client".to_string(),
        Some(Command::Run { .. }) => "daanio run".to_string(),
        Some(Command::Login { .. }) => "daanio login".to_string(),
        Some(Command::Account { .. }) => "daanio account".to_string(),
        Some(Command::Repl) => "daanio repl".to_string(),
        Some(Command::Update) => "daanio update".to_string(),
        Some(Command::Version { .. }) => "daanio version".to_string(),
        Some(Command::Usage { .. }) => "daanio usage".to_string(),
        Some(Command::SelfDev { .. }) => "daanio:selfdev".to_string(),
        Some(Command::Debug { .. }) => "daanio debug".to_string(),
        Some(Command::Auth(_)) => "daanio auth".to_string(),
        Some(Command::Provider(_)) => "daanio provider".to_string(),
        Some(Command::Memory(_)) => "daanio memory".to_string(),
        Some(Command::Session(_)) => "daanio session".to_string(),
        Some(Command::Ambient(subcommand)) => match subcommand {
            AmbientCommand::RunVisible => "daanio ambient visible".to_string(),
            _ => "daanio ambient".to_string(),
        },
        Some(Command::Cloud(_)) => "daanio cloud".to_string(),
        Some(Command::Pair { .. }) => "daanio pair".to_string(),
        Some(Command::Permissions) => "daanio permissions".to_string(),
        Some(Command::Transcript { .. }) => "daanio transcript".to_string(),
        Some(Command::Dictate { .. }) => "daanio dictate".to_string(),
        Some(Command::SetupHotkey {
            listen_macos_hotkey,
            notify_cli_launch,
            listen_windows_hotkey,
            uninstall,
        }) => {
            if *listen_macos_hotkey || *listen_windows_hotkey {
                "daanio hotkey listener".to_string()
            } else if notify_cli_launch.is_some() {
                "daanio shortcut reminder".to_string()
            } else if *uninstall {
                "daanio hotkey uninstall".to_string()
            } else {
                "daanio hotkey setup".to_string()
            }
        }
        Some(Command::Browser { .. }) => "daanio browser".to_string(),
        Some(Command::Replay { .. }) => "daanio replay".to_string(),
        Some(Command::Model(_)) => "daanio model".to_string(),
        Some(Command::ProviderTestCoverage { .. }) => "daanio provider-test-coverage".to_string(),
        Some(Command::ProviderDoctor { .. }) => "daanio provider-doctor".to_string(),
        Some(Command::AuthTest { .. }) => "daanio auth-test".to_string(),
        Some(Command::Restart { .. }) => "daanio restart".to_string(),
        Some(Command::Menubar { .. }) => "daanio menubar".to_string(),
        Some(Command::SetupLauncher) => "daanio setup-launcher".to_string(),
        None => {
            if let Some(resume) = args.resume.as_deref().filter(|resume| !resume.is_empty()) {
                let prefix = if crate::cli::selfdev::client_selfdev_requested() {
                    "daanio:d:"
                } else {
                    "daanio:c:"
                };
                compact_process_title(prefix, Some(&session_name(resume)))
            } else if crate::cli::selfdev::client_selfdev_requested() {
                "daanio:selfdev".to_string()
            } else {
                "daanio:client".to_string()
            }
        }
    }
}

pub(crate) fn set_initial_title(args: &Args) {
    set_title(initial_title(args));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::lock_test_env;
    use clap::Parser;

    const SELFDEV_ENV: &str = daanio_selfdev_types::CLIENT_SELFDEV_ENV;

    fn with_selfdev_env_removed<T>(f: impl FnOnce() -> T) -> T {
        let _guard = lock_test_env();
        let previous = std::env::var_os(SELFDEV_ENV);
        crate::env::remove_var(SELFDEV_ENV);
        let result = f();
        if let Some(value) = previous {
            crate::env::set_var(SELFDEV_ENV, value);
        }
        result
    }

    #[test]
    fn initial_title_labels_server() {
        with_selfdev_env_removed(|| {
            let args = Args::parse_from(["daanio", "serve"]);
            assert_eq!(initial_title(&args), "daanio:server");
        });
    }

    #[test]
    fn initial_title_labels_resume_client_with_short_name() {
        with_selfdev_env_removed(|| {
            let args = Args::parse_from(["daanio", "--resume", "session_fox_123"]);
            assert_eq!(initial_title(&args), "daanio:c:fox");
        });
    }

    #[test]
    fn initial_title_labels_selfdev_command() {
        with_selfdev_env_removed(|| {
            let args = Args::parse_from(["daanio", "self-dev"]);
            assert_eq!(initial_title(&args), "daanio:selfdev");
        });
    }

    #[test]
    fn initial_title_labels_windows_hotkey_listener() {
        let args = Args::parse_from(["daanio", "setup-hotkey", "--listen-windows-hotkey"]);
        assert_eq!(initial_title(&args), "daanio hotkey listener");
    }

    #[test]
    fn initial_title_labels_hotkey_uninstall() {
        let args = Args::parse_from(["daanio", "setup-hotkey", "--uninstall"]);
        assert_eq!(initial_title(&args), "daanio hotkey uninstall");
    }
}

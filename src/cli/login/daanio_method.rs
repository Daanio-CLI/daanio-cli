//! Selection and dispatch for first-party Daanio authentication methods.

use super::{LoginOptions, daanio_device};
use anyhow::Result;
use std::io::{self, IsTerminal, Write};

pub(super) fn parse(input: &str) -> Result<crate::cli::args::DaanioLoginMethodArg> {
    use crate::cli::args::DaanioLoginMethodArg;
    match input.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "browser" | "oauth" | "device" => Ok(DaanioLoginMethodArg::Browser),
        "2" | "api-key" | "apikey" | "key" | "manual" => Ok(DaanioLoginMethodArg::ApiKey),
        value => {
            anyhow::bail!("Invalid Daanio login method '{value}'. Choose 1/browser or 2/api-key.")
        }
    }
}

pub(super) fn resolve(options: &LoginOptions) -> Result<crate::cli::args::DaanioLoginMethodArg> {
    if let Some(method) = options.daanio_method {
        return Ok(method);
    }
    if options.no_browser || !io::stdin().is_terminal() {
        return Ok(crate::cli::args::DaanioLoginMethodArg::Browser);
    }

    eprintln!(
        "\n{}",
        crate::cli::output::heading("Choose a Daanio login method")
    );
    eprintln!(
        "  {}  {}",
        crate::cli::output::command("1. Browser sign-in"),
        crate::cli::output::muted("recommended · OAuth device approval")
    );
    eprintln!(
        "  {}  {}",
        crate::cli::output::command("2. Enter Daanio API key"),
        crate::cli::output::muted("first-party gateway key only")
    );
    eprint!("\nChoose [1]: ");
    io::stderr().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    parse(&input)
}

pub(super) async fn login(
    method: crate::cli::args::DaanioLoginMethodArg,
    no_browser: bool,
) -> Result<daanio_device::LoginCompletion> {
    match method {
        crate::cli::args::DaanioLoginMethodArg::Browser => {
            crate::cli::output::stderr_heading("Starting secure Daanio browser sign-in…");
            daanio_device::login_daanio_device_flow(no_browser).await
        }
        crate::cli::args::DaanioLoginMethodArg::ApiKey => {
            daanio_device::login_daanio_api_key_flow().await
        }
    }
}

pub(crate) async fn run_account_login(
    no_browser: bool,
    method: Option<crate::cli::args::DaanioLoginMethodArg>,
) -> Result<()> {
    let options = LoginOptions {
        no_browser,
        daanio_method: method,
        ..LoginOptions::default()
    };
    login(resolve(&options)?, no_browser).await.map(|_| ())
}

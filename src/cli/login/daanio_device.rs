//! CLI orchestration for the Daanio account device authorization flow.
//!
//! Protocol parsing and HTTP behavior live in `subscription_api` so the CLI and
//! TUI share the same contract and redaction guarantees.

use anyhow::{Context, Result};
use std::future::Future;
use std::time::Duration;

use crate::subscription_api::{
    self, AccountApiError, ApprovedAccountKey, PollingBackoff, TokenPollOutcome,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoginCompletion {
    Active,
    KeySavedPlanPending,
    CanceledBeforeApproval,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum KeyPollCompletion {
    Approved(ApprovedAccountKey),
    Canceled,
}

pub(super) async fn poll_for_api_key<C>(
    client: &reqwest::Client,
    api_base: &str,
    device_code: &str,
    interval: u64,
    expires_in: u64,
    cancel: C,
) -> Result<KeyPollCompletion>
where
    C: Future<Output = std::io::Result<()>>,
{
    tokio::pin!(cancel);
    let base_delay = Duration::from_secs(interval.max(1));
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(expires_in.max(interval.max(1)));
    let mut backoff = PollingBackoff::new(base_delay);
    let mut reported_offline = false;

    loop {
        let delay = backoff.delay();
        if tokio::time::Instant::now() + delay >= deadline {
            anyhow::bail!(
                "Daanio account login timed out before browser approval. Run `daanio account login` to try again."
            );
        }
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            signal = &mut cancel => {
                signal.context("Failed to listen for Ctrl-C")?;
                return Ok(KeyPollCompletion::Canceled);
            }
        }

        // Deliberately do not poll cancellation while an exchange request is
        // in flight. The backend may atomically consume the one-time device
        // credential before the response reaches us. Finishing this bounded
        // request and persisting an approved key avoids stranding a live key
        // that the user can neither see nor revoke.
        match subscription_api::poll_device_token_once(client, api_base, device_code).await {
            Ok(TokenPollOutcome::Pending) => {
                backoff.on_pending();
                reported_offline = false;
            }
            Ok(TokenPollOutcome::SlowDown { retry_after }) => {
                backoff.on_slow_down(retry_after);
                reported_offline = false;
            }
            Ok(TokenPollOutcome::Approved(key)) => {
                return Ok(KeyPollCompletion::Approved(key));
            }
            Ok(TokenPollOutcome::Expired) => anyhow::bail!(
                "The browser approval expired or was already exchanged. Run `daanio account login` to start a new single-use flow."
            ),
            Ok(TokenPollOutcome::Denied) => {
                anyhow::bail!("Daanio account login was canceled or denied in the browser.")
            }
            Err(error) if error.is_temporary() => {
                if !reported_offline {
                    eprintln!("  Connection interrupted. Retrying with backoff...");
                    reported_offline = true;
                }
                backoff.on_offline_error();
            }
            Err(error) => return Err(anyhow::Error::new(error)),
        }
    }
}

fn persist_approved_key(approved: &ApprovedAccountKey) -> Result<()> {
    crate::subscription_catalog::persist_account_credentials(
        &approved.api_key,
        Some(&approved.account_id),
        Some(&approved.email),
        Some(&approved.tier),
    )?;
    crate::auth::AuthStatus::invalidate_cache();
    Ok(())
}

/// Manual first-party credential login. The secret is read without terminal
/// echo, checked against Daanio `/v1/me`, and only then written to disk. This
/// prevents upstream-provider API keys from being accepted by mistake.
pub(super) async fn login_daanio_api_key_flow() -> Result<LoginCompletion> {
    eprintln!("\n{}", crate::cli::output::heading("Daanio API Key Login"));
    eprintln!(
        "  {}",
        crate::cli::output::muted(
            "Only a Daanio gateway API key issued by daanio.com is accepted."
        )
    );
    eprint!("  Paste Daanio API key (input hidden): ");
    use std::io::Write as _;
    std::io::stderr().flush()?;
    let api_key = super::read_secret_line()?;
    if api_key.len() < 8 {
        anyhow::bail!("No valid Daanio API key was provided. Nothing was saved.");
    }

    eprintln!(
        "  {}",
        crate::cli::output::muted("Validating with Daanio /v1/me…")
    );
    let client = crate::provider::shared_http_client();
    let api_base = subscription_api::configured_api_base();
    let me = match subscription_api::fetch_subscription_me_with(&client, &api_base, &api_key).await
    {
        Ok(me) => me,
        Err(AccountApiError::Unauthorized) => anyhow::bail!(
            "That credential was not accepted by Daanio. Only a Daanio gateway API key issued by daanio.com can be used; nothing was saved."
        ),
        Err(error) => {
            anyhow::bail!("Daanio could not validate that API key: {error}. Nothing was saved.")
        }
    };

    crate::subscription_catalog::persist_account_credentials(
        &api_key,
        Some(&me.account_id),
        Some(&me.email),
        Some(&me.tier),
    )?;
    crate::auth::AuthStatus::invalidate_cache();

    let tier = me
        .parsed_tier()
        .map(|tier| tier.display_name().to_string())
        .unwrap_or_else(|| me.tier.clone());
    eprintln!(
        "  {}",
        crate::cli::output::success(format!("✓ Daanio API key validated for {}", me.email))
    );
    eprintln!(
        "  {}",
        crate::cli::output::muted("Credential saved securely (owner-only)")
    );
    if !tier.trim().is_empty() && !tier.eq_ignore_ascii_case("none") {
        eprintln!("  Plan: {tier} · status {}", me.status);
    }
    crate::telemetry::record_auth_success("daanio-subscription", "api_key");

    if me.has_active_paid_plan() {
        Ok(LoginCompletion::Active)
    } else {
        print_recovery_actions();
        Ok(LoginCompletion::KeySavedPlanPending)
    }
}

/// Full browser-first device login. No email or secret is requested in the
/// terminal. The exchanged Daanio credential is saved after one-time approval,
/// regardless of whether account-plan metadata is immediately available.
pub(super) async fn login_daanio_device_flow(no_browser: bool) -> Result<LoginCompletion> {
    let client = crate::provider::shared_http_client();
    let api_base = subscription_api::configured_api_base();
    let device = subscription_api::request_device_authorization(
        &client,
        &api_base,
        Some(crate::subscription_catalog::DaanioTier::Pro),
    )
    .await
    .map_err(anyhow::Error::new)
    .context("Failed to start Daanio account login")?;

    eprintln!("\n{}", crate::cli::output::heading("Daanio Account Login"));
    eprintln!("  {}", crate::cli::output::muted("Secure browser approval"));
    eprintln!(
        "  {}",
        crate::cli::output::link(&device.verification_uri_complete)
    );
    eprintln!("\n  Approve the request in your browser—no terminal email is needed.");
    super::maybe_open_browser(&device.verification_uri_complete, no_browser);
    eprintln!(
        "  {}",
        crate::cli::output::muted("Waiting for approval · Ctrl-C to cancel")
    );

    let approved = match poll_for_api_key(
        &client,
        &api_base,
        &device.device_code,
        device.interval,
        device.expires_in,
        tokio::signal::ctrl_c(),
    )
    .await?
    {
        KeyPollCompletion::Approved(approved) => approved,
        KeyPollCompletion::Canceled => {
            eprintln!("\n  Login canceled before approval. No credential was saved.");
            return Ok(LoginCompletion::CanceledBeforeApproval);
        }
    };

    persist_approved_key(&approved)?;
    if approved.email.trim().is_empty() {
        eprintln!("\n  {}", crate::cli::output::success("✓ Account approved"));
    } else {
        eprintln!(
            "\n  {}",
            crate::cli::output::success(format!("✓ Account approved for {}", approved.email))
        );
    }
    eprintln!(
        "  {}",
        crate::cli::output::muted("Credential saved securely (owner-only)")
    );
    let completion = match subscription_api::fetch_subscription_me_with(
        &client,
        &api_base,
        &approved.api_key,
    )
    .await
    {
        Ok(me) => {
            crate::subscription_catalog::persist_account_credentials(
                &approved.api_key,
                Some(&me.account_id),
                Some(&me.email),
                Some(&me.tier),
            )?;
            let tier = me
                .parsed_tier()
                .map(|tier| tier.display_name().to_string())
                .unwrap_or_else(|| me.tier.clone());
            if tier.trim().is_empty() || tier.eq_ignore_ascii_case("none") {
                eprintln!(
                    "  {}",
                    crate::cli::output::success(format!("✓ Signed in · status {}", me.status))
                );
            } else {
                eprintln!(
                    "  {}",
                    crate::cli::output::success(format!("✓ Signed in · {tier} · {}", me.status))
                );
            }
            if me.has_active_paid_plan() {
                LoginCompletion::Active
            } else {
                print_recovery_actions();
                LoginCompletion::KeySavedPlanPending
            }
        }
        Err(AccountApiError::Unauthorized) => {
            crate::subscription_catalog::clear_account_credentials()?;
            anyhow::bail!(
                "The newly issued Daanio credential was rejected by /v1/me. Local credentials were cleared; run `daanio login daanio` again."
            );
        }
        Err(error) => {
            eprintln!("  ✓ Signed in to Daanio.");
            eprintln!("  Account status could not be loaded yet: {error}");
            print_recovery_actions();
            LoginCompletion::KeySavedPlanPending
        }
    };

    crate::telemetry::record_auth_success("daanio-subscription", "device_code_browser");
    Ok(completion)
}

fn print_recovery_actions() {
    eprintln!("\n  {}", crate::cli::output::muted("Account commands"));
    eprintln!(
        "    {}",
        crate::cli::output::command("daanio account status")
    );
    eprintln!(
        "    {}",
        crate::cli::output::command("daanio account manage")
    );
    eprintln!(
        "    {}",
        crate::cli::output::command("daanio account logout")
    );
}

#[cfg(test)]
mod tests;

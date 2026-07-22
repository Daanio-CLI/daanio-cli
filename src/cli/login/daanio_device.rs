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

    eprintln!("\nDaanio Account Login");
    eprintln!("  Opening the secure account approval page:");
    eprintln!("  {}", device.verification_uri_complete);
    eprintln!("\n  Approve the request in that browser. No terminal email entry is needed.");
    super::maybe_open_browser(&device.verification_uri_complete, no_browser);
    eprintln!("  Waiting for browser approval. Press Ctrl-C to cancel...");

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
        eprintln!("\n  Account approved.");
    } else {
        eprintln!("\n  Account approved for {}.", approved.email);
    }
    eprintln!("  Credential saved securely with owner-only permissions.");
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
                eprintln!("  ✓ Signed in to Daanio (status: {}).", me.status);
            } else {
                eprintln!("  ✓ Signed in to Daanio ({tier}, status: {}).", me.status);
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
    eprintln!("  Check:   daanio account status");
    eprintln!("  Manage:  daanio account manage");
    eprintln!("  Log out: daanio account logout");
}

#[cfg(test)]
mod tests;

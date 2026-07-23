//! Manual first-party Daanio gateway credential login.

use super::{App, DisplayMessage, PendingLogin};
use crate::bus::{Bus, BusEvent, LoginCompleted};

impl App {
    pub(crate) fn start_daanio_api_key_login(&mut self) {
        self.push_display_message(DisplayMessage::system(
            "Daanio API Key Login\n\nPaste a Daanio gateway API key issued by daanio.com. The key is hidden while you type, validated against Daanio, and saved only after validation succeeds.\n\nUpstream OpenAI, Anthropic, OpenRouter, and other provider keys are not accepted. Type /cancel to go back."
                .to_string(),
        ));
        self.set_status_notice("Daanio login: paste your API key (input hidden)");
        self.begin_pending_login(PendingLogin::DaanioApiKey);
    }

    pub(super) fn submit_daanio_api_key(&mut self, input: &str) {
        if input.is_empty() {
            self.push_display_message(DisplayMessage::system(
                "Paste your Daanio gateway API key, or type /cancel to abort. Upstream-provider keys are not accepted."
                    .to_string(),
            ));
            self.pending_login = Some(PendingLogin::DaanioApiKey);
            return;
        }
        if input.len() < 8 {
            self.push_display_message(DisplayMessage::error(
                "That value does not look like a Daanio API key. Paste the complete key issued by daanio.com, or type /cancel."
                    .to_string(),
            ));
            self.pending_login = Some(PendingLogin::DaanioApiKey);
            return;
        }

        self.set_status_notice("Daanio login: validating key...");
        self.push_display_message(DisplayMessage::system(
            "Validating the Daanio gateway credential...".to_string(),
        ));
        let api_key = input.to_string();
        tokio::spawn(async move {
            let client = crate::provider::shared_http_client();
            let api_base = crate::subscription_api::configured_api_base();
            match crate::subscription_api::fetch_subscription_me_with(
                &client, &api_base, &api_key,
            )
            .await
            {
                Ok(me) => {
                    let saved = crate::subscription_catalog::persist_account_credentials(
                        &api_key,
                        Some(&me.account_id),
                        Some(&me.email),
                        Some(&me.tier),
                    );
                    match saved {
                        Ok(()) => {
                            crate::auth::AuthStatus::invalidate_cache();
                            publish(true, format!(
                                "Daanio API key validated and saved securely for {}. Upstream-provider credentials were not stored.",
                                me.email
                            ));
                        }
                        Err(error) => publish(
                            false,
                            format!(
                                "The Daanio API key was valid, but could not be saved securely: {error}"
                            ),
                        ),
                    }
                }
                Err(crate::subscription_api::AccountApiError::Unauthorized) => publish(
                    false,
                    "That credential was not accepted by Daanio /v1/me. Only a Daanio gateway API key issued by daanio.com can be used here; nothing was saved."
                        .to_string(),
                ),
                Err(error) => publish(
                    false,
                    format!("Daanio could not validate that API key: {error}. Nothing was saved."),
                ),
            }
        });
    }
}

fn publish(success: bool, message: String) {
    Bus::global().publish(BusEvent::LoginCompleted(LoginCompleted {
        provider: "daanio".to_string(),
        success,
        message,
    }));
}

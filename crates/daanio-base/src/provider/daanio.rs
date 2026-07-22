use super::{EventStream, ModelRoute, MultiProvider, NativeToolResultSender, Provider, copilot};
use crate::message::{Message, ToolDefinition};
use crate::provider::models::ensure_model_allowed_for_subscription;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, RwLock};

pub struct DaanioProvider {
    inner: MultiProvider,
    selected_model: Arc<RwLock<String>>,
    /// `None` until the authenticated gateway catalog has been fetched. Once
    /// hydrated, including with an empty list, the server is authoritative.
    live_models: Arc<RwLock<Option<Vec<String>>>>,
}

impl DaanioProvider {
    pub fn new() -> Self {
        crate::subscription_catalog::apply_runtime_env();
        Self::apply_runtime_profile();
        let inner = MultiProvider::new_fast();
        let default_model = crate::subscription_catalog::default_model().id.to_string();
        let _ = inner.set_model(&default_model);
        Self {
            inner,
            selected_model: Arc::new(RwLock::new(default_model)),
            live_models: Arc::new(RwLock::new(None)),
        }
    }

    fn apply_runtime_profile() {
        let _ = crate::provider::activation::ProviderActivation::daanio_subscription(
            crate::subscription_catalog::default_model().id,
        )
        .apply_env();
    }

    fn ensure_runtime_mode(&self) {
        if !crate::subscription_catalog::is_runtime_mode_enabled() {
            crate::subscription_catalog::apply_runtime_env();
        }
        Self::apply_runtime_profile();
    }

    #[cfg(test)]
    fn entitled_models_for(
        tier: crate::subscription_catalog::DaanioTier,
    ) -> impl Iterator<Item = &'static crate::subscription_catalog::CuratedModel> {
        crate::subscription_catalog::curated_models()
            .iter()
            .filter(move |model| tier.allows(model.min_tier))
    }

    #[cfg(test)]
    fn model_routes_for(tier: crate::subscription_catalog::DaanioTier) -> Vec<ModelRoute> {
        Self::entitled_models_for(tier)
            .map(|model| Self::model_route(model.id.to_string()))
            .collect()
    }

    fn model_route(model: String) -> ModelRoute {
        ModelRoute {
            model,
            provider: crate::subscription_catalog::DAANIO_PROVIDER_DISPLAY_NAME.to_string(),
            api_method: crate::subscription_catalog::DAANIO_ROUTE_API_METHOD.to_string(),
            available: true,
            detail: "Advertised by the authenticated Daanio gateway catalog".to_string(),
            cheapness: None,
        }
    }

    fn normalize_advertised_models(advertised: &[String]) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        advertised
            .iter()
            .map(|model| model.trim())
            .filter(|model| !model.is_empty())
            .filter(|model| super::is_listable_model_name(model))
            .filter(|model| seen.insert((*model).to_string()))
            .map(str::to_string)
            .collect()
    }

    fn hydrated_models(&self) -> Vec<String> {
        self.live_models
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .unwrap_or_default()
    }

    fn store_live_models(&self, advertised: Vec<String>) {
        // `/v1/models` is authenticated with the browser-issued, revocable
        // account credential, so its response is the authority for this
        // account. Keep its chat-capable models without intersecting them with
        // the release-time curated catalog; the backend can add or remove
        // models without requiring a CLI build.
        let models = Self::normalize_advertised_models(&advertised);
        let selected = self.model();
        let replacement = if models.iter().any(|model| model == &selected) {
            None
        } else if models
            .iter()
            .any(|model| model == crate::subscription_catalog::default_model().id)
        {
            Some(crate::subscription_catalog::default_model().id.to_string())
        } else {
            models.first().cloned()
        };
        *self
            .live_models
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(models);
        if let Some(model) = replacement {
            let _ = self.inner.set_model(&model);
            if let Ok(mut selected_model) = self.selected_model.write() {
                *selected_model = model;
            }
        }
    }

    fn live_model_routes(&self) -> Vec<ModelRoute> {
        self.hydrated_models()
            .into_iter()
            .map(Self::model_route)
            .collect()
    }

    fn resolve_requested_model(&self, model: &str) -> Result<String> {
        let canonical = crate::subscription_catalog::canonical_model_id(model).unwrap_or(model);
        let live_models = self
            .live_models
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(live_models) = live_models.as_ref() {
            return live_models
                .iter()
                .find(|live| live.as_str() == canonical || live.as_str() == model.trim())
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Model '{canonical}' is not currently advertised by the Daanio API. Run `daanio model list --provider daanio` to refresh available models."
                    )
                });
        }
        ensure_model_allowed_for_subscription(model)?;
        Ok(canonical.to_string())
    }

    #[cfg(test)]
    fn set_advertised_models_for_test(&self, advertised: &[&str]) {
        self.store_live_models(
            advertised
                .iter()
                .map(|model| (*model).to_string())
                .collect(),
        );
    }
}

impl Default for DaanioProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for DaanioProvider {
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
        resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        self.ensure_runtime_mode();
        self.inner
            .complete(messages, tools, system, resume_session_id)
            .await
    }

    async fn complete_split(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system_static: &str,
        system_dynamic: &str,
        resume_session_id: Option<&str>,
    ) -> Result<EventStream> {
        self.ensure_runtime_mode();
        self.inner
            .complete_split(
                messages,
                tools,
                system_static,
                system_dynamic,
                resume_session_id,
            )
            .await
    }

    fn name(&self) -> &str {
        crate::subscription_catalog::DAANIO_PROVIDER_DISPLAY_NAME
    }

    fn model(&self) -> String {
        self.selected_model
            .read()
            .map(|model| model.clone())
            .unwrap_or_else(|_| crate::subscription_catalog::default_model().id.to_string())
    }

    fn set_model(&self, model: &str) -> Result<()> {
        self.ensure_runtime_mode();
        let selected = self.resolve_requested_model(model)?;
        self.inner.set_model(&selected)?;
        if let Ok(mut selected_model) = self.selected_model.write() {
            *selected_model = selected;
        }
        Ok(())
    }

    fn available_models(&self) -> Vec<&'static str> {
        self.ensure_runtime_mode();
        Vec::new()
    }

    fn available_models_display(&self) -> Vec<String> {
        self.ensure_runtime_mode();
        self.hydrated_models()
    }

    fn available_models_for_switching(&self) -> Vec<String> {
        self.ensure_runtime_mode();
        self.hydrated_models()
    }

    fn available_providers_for_model(&self, model: &str) -> Vec<String> {
        self.inner.available_providers_for_model(model)
    }

    fn provider_details_for_model(&self, model: &str) -> Vec<(String, String)> {
        self.inner.provider_details_for_model(model)
    }

    fn preferred_provider(&self) -> Option<String> {
        self.inner.preferred_provider()
    }

    fn model_routes(&self) -> Vec<ModelRoute> {
        self.ensure_runtime_mode();
        self.live_model_routes()
    }

    async fn prefetch_models(&self) -> Result<()> {
        self.ensure_runtime_mode();
        let api_key = crate::subscription_catalog::configured_api_key()
            .ok_or_else(|| anyhow::anyhow!("No Daanio browser credential is configured"))?;
        let models = crate::subscription_api::fetch_available_models_with(
            &crate::provider::shared_http_client(),
            &crate::subscription_api::configured_api_base(),
            &api_key,
        )
        .await
        .map_err(anyhow::Error::new)?;
        self.store_live_models(models);
        Ok(())
    }

    fn on_auth_changed(&self) {
        self.ensure_runtime_mode();
        *self
            .live_models
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.inner.on_auth_changed();
        let selected_model = self.model();
        let _ = self.inner.set_model(&selected_model);
    }

    fn auth_model_refresh_pending(&self) -> bool {
        self.inner.auth_model_refresh_pending()
    }

    fn reasoning_effort(&self) -> Option<String> {
        self.inner.reasoning_effort()
    }

    fn set_reasoning_effort(&self, effort: &str) -> Result<()> {
        self.inner.set_reasoning_effort(effort)
    }

    fn available_efforts(&self) -> Vec<&'static str> {
        self.inner.available_efforts()
    }

    fn native_compaction_mode(&self) -> Option<String> {
        self.inner.native_compaction_mode()
    }

    fn native_compaction_threshold_tokens(&self) -> Option<usize> {
        self.inner.native_compaction_threshold_tokens()
    }

    fn transport(&self) -> Option<String> {
        self.inner.transport()
    }

    fn set_transport(&self, transport: &str) -> Result<()> {
        self.inner.set_transport(transport)
    }

    fn available_transports(&self) -> Vec<&'static str> {
        self.inner.available_transports()
    }

    fn handles_tools_internally(&self) -> bool {
        self.inner.handles_tools_internally()
    }

    async fn invalidate_credentials(&self) {
        self.inner.invalidate_credentials().await;
    }

    fn set_premium_mode(&self, mode: copilot::PremiumMode) {
        self.inner.set_premium_mode(mode);
    }

    fn premium_mode(&self) -> copilot::PremiumMode {
        self.inner.premium_mode()
    }

    fn supports_compaction(&self) -> bool {
        self.inner.supports_compaction()
    }

    fn uses_daanio_compaction(&self) -> bool {
        self.inner.uses_daanio_compaction()
    }

    async fn native_compact(
        &self,
        messages: &[Message],
        existing_summary_text: Option<&str>,
        existing_openai_encrypted_content: Option<&str>,
    ) -> Result<crate::provider::NativeCompactionResult> {
        self.inner
            .native_compact(
                messages,
                existing_summary_text,
                existing_openai_encrypted_content,
            )
            .await
    }

    fn context_window(&self) -> usize {
        self.inner.context_window()
    }

    fn fork(&self) -> Arc<dyn Provider> {
        self.ensure_runtime_mode();
        let forked = Self::new();
        *forked
            .live_models
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = self
            .live_models
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let selected_model = self.model();
        let _ = forked.set_model(&selected_model);
        Arc::new(forked)
    }

    fn native_result_sender(&self) -> Option<NativeToolResultSender> {
        self.inner.native_result_sender()
    }

    fn drain_startup_notices(&self) -> Vec<String> {
        self.inner.drain_startup_notices()
    }

    fn switch_active_provider_to(&self, provider: &str) -> Result<()> {
        self.ensure_runtime_mode();
        self.inner.switch_active_provider_to(provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daanio_provider_enables_subscription_runtime_mode() {
        let _guard = crate::storage::lock_test_env();
        crate::subscription_catalog::clear_runtime_env();
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

        runtime.block_on(async {
            let provider = DaanioProvider::new();
            assert!(crate::subscription_catalog::is_runtime_mode_enabled());
            assert!(
                provider
                    .available_models_display()
                    .into_iter()
                    .all(|model| crate::subscription_catalog::is_curated_model(&model))
            );
        });

        crate::subscription_catalog::clear_runtime_env();
    }

    #[test]
    fn daanio_provider_name_and_default_model_are_curated() {
        let _guard = crate::storage::lock_test_env();
        crate::subscription_catalog::clear_runtime_env();
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

        runtime.block_on(async {
            let provider = DaanioProvider::new();
            assert_eq!(
                provider.name(),
                crate::subscription_catalog::DAANIO_PROVIDER_DISPLAY_NAME
            );
            let model = provider.model();
            assert!(
                crate::subscription_catalog::is_curated_model(&model),
                "expected curated model, got {model}"
            );
        });

        crate::subscription_catalog::clear_runtime_env();
    }

    #[test]
    fn live_catalog_uses_authenticated_chat_models_without_a_curated_allowlist() {
        let advertised = vec![
            "gpt-5.6-sol".to_string(),
            " gpt-5.6-sol ".to_string(),
            "server-model-not-in-cli-catalog".to_string(),
            "claude-fable-5".to_string(),
            "gpt-image-2".to_string(),
        ];
        assert_eq!(
            DaanioProvider::normalize_advertised_models(&advertised),
            vec![
                "gpt-5.6-sol",
                "server-model-not-in-cli-catalog",
                "claude-fable-5",
            ]
        );
    }

    #[test]
    fn hydrated_catalog_controls_display_routes_and_model_selection() {
        let _guard = crate::storage::lock_test_env();
        crate::subscription_catalog::clear_runtime_env();
        let provider = DaanioProvider::new();

        assert!(provider.available_models_display().is_empty());
        provider.set_advertised_models_for_test(&["gpt-5.5"]);
        assert_eq!(provider.available_models_display(), vec!["gpt-5.5"]);
        assert_eq!(provider.model(), "gpt-5.5");
        assert_eq!(
            provider
                .model_routes()
                .into_iter()
                .map(|route| route.model)
                .collect::<Vec<_>>(),
            vec!["gpt-5.5"]
        );
        assert!(provider.set_model("gpt-5.6-sol").is_err());

        provider.set_advertised_models_for_test(&["server-model-not-in-cli-catalog"]);
        assert_eq!(
            provider.available_models_display(),
            vec!["server-model-not-in-cli-catalog"]
        );
        assert!(
            provider
                .resolve_requested_model("server-model-not-in-cli-catalog")
                .is_ok()
        );

        provider.set_advertised_models_for_test(&[]);
        assert!(provider.available_models_display().is_empty());
        assert!(provider.model_routes().is_empty());
        assert!(provider.set_model("gpt-5.5").is_err());
        crate::subscription_catalog::clear_runtime_env();
    }

    #[test]
    fn daanio_provider_exposes_only_explicit_subscription_routes() {
        use crate::subscription_catalog::DaanioTier;

        let plus_routes = DaanioProvider::model_routes_for(DaanioTier::Plus);
        let gpt_route = plus_routes
            .iter()
            .find(|route| route.model == "gpt-5.5")
            .expect("Plus tier includes GPT-5.5");
        let route_selection = daanio_provider_core::RouteSelection::from_model_route(gpt_route);
        let flagship_routes = DaanioProvider::model_routes_for(DaanioTier::Flagship);
        let expected_models = vec![
            "claude-opus-4-8",
            "claude-sonnet-4-6",
            "gpt-5.5",
            "gpt-5.6-sol",
            "qwen3-coder-next",
            "devstral-2-123b",
            "deepseek-v3.2",
            "nova-2-lite",
            "minimax-m2.5",
            "mistral-large-3",
            "kimi-k2.5",
            "kimi-k2-thinking",
            "nemotron-nano-3-30b",
            "gpt-oss-120b",
            "gpt-oss-20b",
            "qwen3-next-80b",
            "glm-5",
            "glm-4.7-flash",
        ];

        assert_eq!(
            plus_routes
                .iter()
                .map(|route| route.model.as_str())
                .collect::<Vec<_>>(),
            expected_models
        );
        assert!(plus_routes.iter().all(|route| {
            route.provider == crate::subscription_catalog::DAANIO_PROVIDER_DISPLAY_NAME
                && route.api_method == "daanio-subscription"
                && route.available
        }));
        assert_eq!(
            DaanioProvider::entitled_models_for(DaanioTier::Plus)
                .map(|model| model.id.to_string())
                .collect::<Vec<_>>(),
            expected_models
                .iter()
                .map(|model| (*model).to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(route_selection.routed_model_spec(), "gpt-5.5");
        assert_eq!(
            route_selection.runtime_key,
            daanio_provider_core::RuntimeKey::DaanioSubscription
        );
        assert_eq!(route_selection.api_method, "daanio-subscription");
        assert_eq!(
            route_selection.provider_label,
            crate::subscription_catalog::DAANIO_PROVIDER_DISPLAY_NAME
        );
        assert_eq!(flagship_routes.len(), 19);
        assert!(
            flagship_routes
                .iter()
                .any(|route| route.model == "claude-fable-5")
        );
    }
}

use super::*;

impl Config {
    /// Apply environment variable overrides
    #[expect(
        clippy::collapsible_if,
        reason = "Environment override parsing is intentionally explicit and grouped by config area"
    )]
    pub(crate) fn apply_env_overrides(&mut self) {
        // Keybindings
        if let Ok(v) = std::env::var("DAANIO_SCROLL_UP_KEY") {
            self.keybindings.scroll_up = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SCROLL_DOWN_KEY") {
            self.keybindings.scroll_down = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SCROLL_PAGE_UP_KEY") {
            self.keybindings.scroll_page_up = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SCROLL_PAGE_DOWN_KEY") {
            self.keybindings.scroll_page_down = v;
        }
        if let Ok(v) = std::env::var("DAANIO_MODEL_SWITCH_KEY") {
            self.keybindings.model_switch_next = v;
        }
        if let Ok(v) = std::env::var("DAANIO_MODEL_SWITCH_PREV_KEY") {
            self.keybindings.model_switch_prev = v;
        }
        if let Ok(v) = std::env::var("DAANIO_EFFORT_INCREASE_KEY") {
            self.keybindings.effort_increase = v;
        }
        if let Ok(v) = std::env::var("DAANIO_EFFORT_DECREASE_KEY") {
            self.keybindings.effort_decrease = v;
        }
        if let Ok(v) = std::env::var("DAANIO_CENTERED_TOGGLE_KEY") {
            self.keybindings.centered_toggle = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SCROLL_PROMPT_UP_KEY") {
            self.keybindings.scroll_prompt_up = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SCROLL_PROMPT_DOWN_KEY") {
            self.keybindings.scroll_prompt_down = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SCROLL_BOOKMARK_KEY") {
            self.keybindings.scroll_bookmark = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SCROLL_UP_FALLBACK_KEY") {
            self.keybindings.scroll_up_fallback = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SCROLL_DOWN_FALLBACK_KEY") {
            self.keybindings.scroll_down_fallback = v;
        }
        if let Ok(v) = std::env::var("DAANIO_WORKSPACE_LEFT_KEY") {
            self.keybindings.workspace_left = v;
        }
        if let Ok(v) = std::env::var("DAANIO_WORKSPACE_DOWN_KEY") {
            self.keybindings.workspace_down = v;
        }
        if let Ok(v) = std::env::var("DAANIO_WORKSPACE_UP_KEY") {
            self.keybindings.workspace_up = v;
        }
        if let Ok(v) = std::env::var("DAANIO_WORKSPACE_RIGHT_KEY") {
            self.keybindings.workspace_right = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SIDE_PANEL_TOGGLE_KEY") {
            self.keybindings.side_panel_toggle = v;
        }
        if let Ok(v) = std::env::var("DAANIO_COPY_SELECTION_TOGGLE_KEY") {
            self.keybindings.copy_selection_toggle = v;
        }
        if let Ok(v) = std::env::var("DAANIO_DIAGRAM_PANE_TOGGLE_KEY") {
            self.keybindings.diagram_pane_toggle = v;
        }
        if let Ok(v) = std::env::var("DAANIO_TYPING_SCROLL_LOCK_TOGGLE_KEY") {
            self.keybindings.typing_scroll_lock_toggle = v;
        }
        if let Ok(v) = std::env::var("DAANIO_DIFF_MODE_CYCLE_KEY") {
            self.keybindings.diff_mode_cycle = v;
        }
        if let Ok(v) = std::env::var("DAANIO_INFO_WIDGET_TOGGLE_KEY") {
            self.keybindings.info_widget_toggle = v;
        }
        if let Ok(v) = std::env::var("DAANIO_NEW_TERMINAL_KEY") {
            self.keybindings.new_terminal = v;
        }

        // Dictation
        if let Ok(v) = std::env::var("DAANIO_DICTATION_COMMAND") {
            self.dictation.command = v;
        }
        if let Ok(v) = std::env::var("DAANIO_DICTATION_MODE")
            && let Ok(mode) = toml::from_str::<crate::protocol::TranscriptMode>(&format!(
                "\"{}\"",
                v.trim().to_ascii_lowercase()
            ))
        {
            self.dictation.mode = mode;
        }
        if let Ok(v) = std::env::var("DAANIO_DICTATION_KEY") {
            self.dictation.key = v;
        }
        if let Ok(v) = std::env::var("DAANIO_DICTATION_TIMEOUT_SECS")
            && let Ok(parsed) = v.trim().parse::<u64>()
        {
            self.dictation.timeout_secs = parsed;
        }

        // Tools
        if let Ok(v) = std::env::var("DAANIO_TOOL_PROFILE") {
            self.tools.profile = v;
        }
        if let Ok(v) = std::env::var("DAANIO_TOOLS") {
            self.tools.enabled = parse_env_list(&v);
        }
        if let Ok(v) = std::env::var("DAANIO_DISABLED_TOOLS") {
            self.tools.disabled = parse_env_list(&v);
        }
        if let Ok(v) = std::env::var("DAANIO_DISABLE_BASE_TOOLS")
            && let Some(parsed) = parse_env_bool(&v)
        {
            self.tools.disable_base_tools = parsed;
        }

        // ACP adapter
        if let Ok(v) = std::env::var("DAANIO_ACP_PROFILE") {
            let trimmed = v.trim().to_ascii_lowercase();
            if matches!(trimmed.as_str(), "standard" | "extended" | "full") {
                self.acp.profile = trimmed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_ACP_TOOL_PROFILE") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                self.acp.tool_profile = trimmed.to_string();
            }
        }

        // Display
        if let Ok(v) = std::env::var("DAANIO_DIFF_MODE") {
            match v.to_lowercase().as_str() {
                "off" | "none" | "0" | "false" => self.display.diff_mode = DiffDisplayMode::Off,
                "inline" | "on" | "1" | "true" => self.display.diff_mode = DiffDisplayMode::Inline,
                "full-inline" | "full_inline" | "fullinline" | "inline-full" | "inline_full"
                | "inlinefull" | "full" => {
                    self.display.diff_mode = DiffDisplayMode::FullInline;
                }
                "pinned" | "pin" => self.display.diff_mode = DiffDisplayMode::Pinned,
                "file" => self.display.diff_mode = DiffDisplayMode::File,
                _ => {}
            }
        } else if let Ok(v) = std::env::var("DAANIO_SHOW_DIFFS")
            && let Some(parsed) = parse_env_bool(&v)
        {
            self.display.diff_mode = if parsed {
                DiffDisplayMode::Inline
            } else {
                DiffDisplayMode::Off
            };
        }
        if let Ok(v) = std::env::var("DAANIO_PIN_IMAGES")
            && let Some(parsed) = parse_env_bool(&v)
        {
            self.display.pin_images = parsed;
        }
        if let Ok(v) = std::env::var("DAANIO_DISPLAY_CENTERED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.centered = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_DIFF_LINE_WRAP") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.diff_line_wrap = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_QUEUE_MODE") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.queue_mode = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_AUTO_SERVER_RELOAD") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.auto_server_reload = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_MOUSE_CAPTURE") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.mouse_capture = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_DEBUG_SOCKET") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.debug_socket = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_SHOW_THINKING") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.show_thinking = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_REASONING_DISPLAY") {
            if let Some(mode) = crate::config::ReasoningDisplayMode::parse(&v) {
                self.display.set_reasoning_display(mode);
            }
        }
        if let Ok(v) = std::env::var("DAANIO_MARKDOWN_SPACING") {
            match v.trim().to_lowercase().as_str() {
                "compact" => self.display.markdown_spacing = MarkdownSpacingMode::Compact,
                "document" | "doc" => {
                    self.display.markdown_spacing = MarkdownSpacingMode::Document;
                }
                _ => {}
            }
        }
        if let Ok(v) = std::env::var("DAANIO_IDLE_ANIMATION") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.idle_animation = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_PROMPT_ENTRY_ANIMATION") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.prompt_entry_animation = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_DISABLED_ANIMATIONS") {
            self.display.disabled_animations = parse_env_list(&v);
        }
        if let Ok(v) = std::env::var("DAANIO_ACTIVE_SESSIONS_MANAGER") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.active_sessions_manager = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_PERFORMANCE") {
            let trimmed = v.trim().to_lowercase();
            if matches!(trimmed.as_str(), "auto" | "full" | "reduced" | "minimal") {
                self.display.performance = trimmed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_ANIMATION_FPS") {
            if let Ok(fps) = v.trim().parse::<u32>() {
                self.display.animation_fps = fps.clamp(1, 120);
            }
        }
        if let Ok(v) = std::env::var("DAANIO_REDRAW_FPS") {
            if let Ok(fps) = v.trim().parse::<u32>() {
                self.display.redraw_fps = fps.clamp(1, 120);
            }
        }
        if let Ok(v) = std::env::var("DAANIO_COPY_BADGE_ALT_LABEL") {
            self.display.copy_badge_alt_label = v;
        }
        if let Ok(v) = std::env::var("DAANIO_COMPACT_NOTIFICATIONS") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.compact_notifications = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_SHOW_AGENTGREP_OUTPUT") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.show_agentgrep_output = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_TOOL_CALL_DETAILS") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.tool_call_details = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_LATEX_RENDERING")
            && let Some(mode) = LatexRenderingMode::parse(&v)
        {
            self.display.latex_rendering = mode;
        }
        if let Ok(v) = std::env::var("DAANIO_CHAT_NATIVE_SCROLLBAR") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.native_scrollbars.chat = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_SIDE_PANEL_NATIVE_SCROLLBAR") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.display.native_scrollbars.side_panel = parsed;
            }
        }

        // Features
        if let Ok(v) = std::env::var("DAANIO_MEMORY_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.memory = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_SWARM_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.swarm = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_ENABLE_MERMAID") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.mermaid = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_MESSAGE_TIMESTAMPS") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.message_timestamps = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_PERSIST_MEMORY_INJECTIONS") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.persist_memory_injections = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_KV_CACHE_MISS_NOTICES") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.features.kv_cache_miss_notices = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_UPDATE_CHANNEL")
            && let Some(channel) = UpdateChannel::parse(&v)
        {
            self.features.update_channel = channel;
        }

        // Agents (spawned helper sessions)
        if let Ok(v) = std::env::var("DAANIO_SWARM_MODEL") {
            let trimmed = v.trim();
            self.agents.swarm_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(v) = std::env::var("DAANIO_SWARM_SPAWN_MODE") {
            if let Some(parsed) = SwarmSpawnMode::parse(&v) {
                self.agents.swarm_spawn_mode = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_SWARM_STRIP_LAYOUT") {
            if let Some(parsed) = SwarmStripLayout::parse(&v) {
                self.agents.swarm_strip_layout = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_SWARM_MAX_CONCURRENT_AGENTS") {
            if let Ok(parsed) = v.trim().parse::<usize>() {
                self.agents.swarm_max_concurrent_agents = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_MEMORY_MODEL") {
            let trimmed = v.trim();
            self.agents.memory_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(v) = std::env::var("DAANIO_MEMORY_SIDECAR_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.agents.memory_sidecar_enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_MEMORY_EMBEDDING_BACKEND") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                self.agents.memory_embedding_backend = trimmed.to_string();
            }
        }
        if let Ok(v) = std::env::var("DAANIO_MEMORY_EMBEDDING_MODEL") {
            let trimmed = v.trim();
            self.agents.memory_embedding_model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(v) = std::env::var("DAANIO_MEMORY_EMBEDDING_BASE_URL") {
            let trimmed = v.trim();
            self.agents.memory_embedding_base_url = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(v) = std::env::var("DAANIO_MEMORY_EMBEDDING_DIM") {
            if let Ok(parsed) = v.trim().parse::<usize>() {
                self.agents.memory_embedding_dim = Some(parsed);
            }
        }

        // Terminal spawning
        if let Ok(v) = std::env::var("DAANIO_SPAWN_HOOK") {
            let trimmed = v.trim();
            // An explicitly empty env value disables a config-file hook.
            self.terminal.spawn_hook = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(v) = std::env::var("DAANIO_FOCUS_HOOK") {
            let trimmed = v.trim();
            // An explicitly empty env value disables a config-file hook.
            self.terminal.focus_hook = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        // Lifecycle hooks. Empty env values disable config-file hooks.
        fn hook_env_override(slot: &mut Option<String>, key: &str) {
            if let Ok(v) = std::env::var(key) {
                let trimmed = v.trim();
                *slot = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
        }
        hook_env_override(&mut self.hooks.turn_start, "DAANIO_HOOK_TURN_START");
        hook_env_override(&mut self.hooks.turn_end, "DAANIO_HOOK_TURN_END");
        hook_env_override(&mut self.hooks.session_start, "DAANIO_HOOK_SESSION_START");
        hook_env_override(&mut self.hooks.session_end, "DAANIO_HOOK_SESSION_END");
        hook_env_override(&mut self.hooks.pre_tool, "DAANIO_HOOK_PRE_TOOL");
        hook_env_override(&mut self.hooks.post_tool, "DAANIO_HOOK_POST_TOOL");
        if let Ok(v) = std::env::var("DAANIO_HOOK_PRE_TOOL_TIMEOUT_MS") {
            if let Ok(parsed) = v.trim().parse::<u64>() {
                self.hooks.pre_tool_timeout_ms = parsed;
            }
        }

        // Web search
        if let Ok(v) = std::env::var("DAANIO_WEBSEARCH_ENGINE")
            && let Some(engine) = WebSearchEngine::parse(&v)
        {
            self.websearch.engine = engine;
        }
        if let Ok(v) = std::env::var("DAANIO_WEBSEARCH_FALLBACK_ENGINES") {
            let engines = parse_env_list(&v)
                .into_iter()
                .filter_map(|item| WebSearchEngine::parse(&item))
                .collect::<Vec<_>>();
            if !engines.is_empty() {
                self.websearch.fallback_engines = engines;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_BING_API_KEY")
            && !v.trim().is_empty()
        {
            self.websearch.bing_api_key = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_BING_API_KEY_ENV")
            && !v.trim().is_empty()
        {
            self.websearch.bing_api_key_env = v;
        }
        if let Ok(v) = std::env::var("DAANIO_BING_MARKET")
            && !v.trim().is_empty()
        {
            self.websearch.bing_market = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SEARXNG_URL")
            && !v.trim().is_empty()
        {
            self.websearch.searxng_url = Some(v);
        }

        if let Ok(v) = std::env::var("DAANIO_TRUSTED_EXTERNAL_AUTH_SOURCES") {
            let mut source_ids = Vec::new();
            let mut source_paths = Vec::new();
            for value in parse_env_list(&v) {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.contains('|') {
                    source_paths.push(trimmed.to_ascii_lowercase());
                } else {
                    source_ids.push(trimmed.to_ascii_lowercase());
                }
            }
            self.auth.trusted_external_sources = source_ids;
            self.auth.trusted_external_source_paths = source_paths;
        }

        // Autoreview
        if let Ok(v) = std::env::var("DAANIO_AUTOREVIEW_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.autoreview.enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_AUTOREVIEW_MODEL") {
            let trimmed = v.trim();
            self.autoreview.model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        // Autojudge
        if let Ok(v) = std::env::var("DAANIO_AUTOJUDGE_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.autojudge.enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_AUTOJUDGE_MODEL") {
            let trimmed = v.trim();
            self.autojudge.model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        // Ambient
        if let Ok(v) = std::env::var("DAANIO_AMBIENT_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.ambient.enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_AMBIENT_PROVIDER") {
            self.ambient.provider = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_AMBIENT_MODEL") {
            self.ambient.model = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_AMBIENT_MIN_INTERVAL") {
            if let Ok(parsed) = v.trim().parse::<u32>() {
                self.ambient.min_interval_minutes = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_AMBIENT_MAX_INTERVAL") {
            if let Ok(parsed) = v.trim().parse::<u32>() {
                self.ambient.max_interval_minutes = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_AMBIENT_PROACTIVE") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.ambient.proactive_work = parsed;
            }
        }

        // Safety / notifications
        if let Ok(v) = std::env::var("DAANIO_NTFY_TOPIC") {
            self.safety.ntfy_topic = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_NTFY_SERVER") {
            self.safety.ntfy_server = v;
        }
        if let Ok(v) = std::env::var("DAANIO_SMTP_PASSWORD") {
            self.safety.email_password = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_EMAIL_TO") {
            self.safety.email_to = Some(v);
            self.safety.email_enabled = true;
        }
        if let Ok(v) = std::env::var("DAANIO_IMAP_HOST") {
            self.safety.email_imap_host = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_EMAIL_REPLY_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.safety.email_reply_enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_TELEGRAM_BOT_TOKEN") {
            self.safety.telegram_bot_token = Some(v);
            self.safety.telegram_enabled = true;
        }
        if let Ok(v) = std::env::var("DAANIO_TELEGRAM_CHAT_ID") {
            self.safety.telegram_chat_id = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_TELEGRAM_REPLY_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.safety.telegram_reply_enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_DISCORD_BOT_TOKEN") {
            self.safety.discord_bot_token = Some(v);
            self.safety.discord_enabled = true;
        }
        if let Ok(v) = std::env::var("DAANIO_DISCORD_CHANNEL_ID") {
            self.safety.discord_channel_id = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_DISCORD_BOT_USER_ID") {
            self.safety.discord_bot_user_id = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_DISCORD_REPLY_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.safety.discord_reply_enabled = parsed;
            }
        }
        // Jade cloud relay channel
        if let Ok(v) = std::env::var("DAANIO_JADE_RELAY_API_BASE") {
            self.safety.jade_relay_api_base = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_JADE_RELAY_TOKEN") {
            self.safety.jade_relay_token = Some(v);
            self.safety.jade_relay_enabled = true;
        }
        if let Ok(v) = std::env::var("DAANIO_JADE_RELAY_TOKEN_ID") {
            self.safety.jade_relay_token_id = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_JADE_RELAY_USER_ID") {
            self.safety.jade_relay_user_id = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_JADE_RELAY_SESSION_ID") {
            self.safety.jade_relay_session_id = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_JADE_RELAY_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.safety.jade_relay_enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_JADE_RELAY_REPLY_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.safety.jade_relay_reply_enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_JADE_RELAY_LAUNCH_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.safety.jade_relay_launch_enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_JADE_RELAY_LAUNCH_WORKING_DIR") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                self.safety.jade_relay_launch_working_dir = Some(trimmed.to_string());
            }
        }
        if let Ok(v) = std::env::var("DAANIO_AMBIENT_VISIBLE") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.ambient.visible = parsed;
            }
        }

        // Gateway (iOS/web)
        if let Ok(v) = std::env::var("DAANIO_GATEWAY_ENABLED") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.gateway.enabled = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_GATEWAY_PORT") {
            if let Ok(parsed) = v.trim().parse::<u16>() {
                self.gateway.port = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_GATEWAY_BIND_ADDR") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                self.gateway.bind_addr = trimmed.to_string();
            }
        }

        // Power management
        if let Ok(v) = std::env::var("DAANIO_PREVENT_SLEEP_WHILE_STREAMING") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.power.prevent_sleep_while_streaming = parsed;
            }
        }

        // Provider
        if let Ok(v) = std::env::var("DAANIO_MODEL") {
            self.provider.default_model = Some(v);
        }
        if let Ok(v) = std::env::var("DAANIO_PROVIDER") {
            let trimmed = v.trim().to_lowercase();
            if !trimmed.is_empty() {
                self.provider.default_provider = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("DAANIO_OPENAI_REASONING_EFFORT") {
            let trimmed = v.trim().to_string();
            if !trimmed.is_empty() {
                self.provider.openai_reasoning_effort = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("DAANIO_ANTHROPIC_REASONING_EFFORT") {
            let trimmed = v.trim().to_string();
            if !trimmed.is_empty() {
                self.provider.anthropic_reasoning_effort = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("DAANIO_OPENAI_TRANSPORT") {
            let trimmed = v.trim().to_string();
            if !trimmed.is_empty() {
                self.provider.openai_transport = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("DAANIO_OPENAI_SERVICE_TIER") {
            let trimmed = v.trim().to_string();
            if !trimmed.is_empty() {
                self.provider.openai_service_tier = Some(trimmed);
            }
        }
        if let Ok(v) = std::env::var("DAANIO_OPENAI_NATIVE_COMPACTION_MODE") {
            let trimmed = v.trim().to_ascii_lowercase();
            if !trimmed.is_empty() {
                self.provider.openai_native_compaction_mode = trimmed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_OPENAI_NATIVE_COMPACTION_THRESHOLD_TOKENS") {
            if let Ok(parsed) = v.trim().parse::<usize>() {
                if parsed > 0 {
                    self.provider.openai_native_compaction_threshold_tokens = parsed;
                }
            }
        }
        if let Ok(v) = std::env::var("DAANIO_PRESERVE_REASONING_CONTEXT") {
            if let Some(parsed) = parse_env_bool(&v) {
                self.provider.preserve_reasoning_context = parsed;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_CROSS_PROVIDER_FAILOVER") {
            if let Some(mode) = CrossProviderFailoverMode::parse(&v) {
                self.provider.cross_provider_failover = mode;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_SAME_PROVIDER_ACCOUNT_FAILOVER") {
            if let Some(enabled) = parse_env_bool(&v) {
                self.provider.same_provider_account_failover = enabled;
            }
        }
        if let Ok(v) = std::env::var("DAANIO_STREAM_IDLE_TIMEOUT_SECS") {
            if let Ok(parsed) = v.trim().parse::<u64>() {
                if parsed > 0 {
                    self.provider.stream_idle_timeout_secs = parsed;
                }
            }
        }

        // Copilot premium mode: env var overrides config
        // If set in config but not in env, propagate config -> env
        if let Ok(v) = std::env::var("DAANIO_COPILOT_PREMIUM") {
            self.provider.copilot_premium = Some(v);
        } else if let Some(ref mode) = self.provider.copilot_premium {
            let env_val = match mode.as_str() {
                "zero" | "0" => "0",
                "one" | "1" => "1",
                _ => "",
            };
            if !env_val.is_empty() {
                crate::env::set_var("DAANIO_COPILOT_PREMIUM", env_val);
            }
        }
    }
}

fn parse_env_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_env_list(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect()
}

use daanio_message_types::{ContentBlock, Message, Role, ToolDefinition, sanitize_tool_id};
use daanio_provider_core::anthropic_map_tool_name_for_oauth as map_tool_name_for_oauth;
use serde::Serialize;
use serde_json::{Value, json};
use std::fmt;

/// Anthropic rejects Messages API bodies at roughly 32 MiB. Keep two MiB of
/// headroom for gateway differences and future envelope fields.
pub const MAX_REQUEST_BYTES: usize = 30 * 1024 * 1024;

const RECENT_MESSAGES_TO_PRESERVE: usize = 3;
const TOOL_RESULT_MIN_BYTES: usize = 1024;
const HISTORY_TEXT_MIN_BYTES: usize = 2048;
const COMPACTION_HEADROOM_BYTES: usize = 4096;
const MAX_CONTEXT_CAPSULE_BYTES: usize = 64 * 1024;
const MAX_CONTEXT_ITEM_BYTES: usize = 2 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestSizeCompaction {
    pub original_bytes: usize,
    pub final_bytes: usize,
    pub images_omitted: usize,
    pub tool_results_truncated: usize,
    pub history_blocks_truncated: usize,
    pub thinking_blocks_omitted: usize,
    pub messages_dropped: usize,
    pub context_capsule_bytes: usize,
}

impl RequestSizeCompaction {
    pub fn compacted(&self) -> bool {
        self.original_bytes != self.final_bytes
    }
}

#[derive(Debug)]
pub struct RequestSizeError {
    pub original_bytes: usize,
    pub final_bytes: usize,
    pub limit_bytes: usize,
    pub system_bytes: usize,
    pub tools_bytes: usize,
    pub messages_bytes: usize,
    serialization_error: Option<serde_json::Error>,
}

impl RequestSizeError {
    fn serialization(error: serde_json::Error) -> Self {
        Self {
            original_bytes: 0,
            final_bytes: 0,
            limit_bytes: 0,
            system_bytes: 0,
            tools_bytes: 0,
            messages_bytes: 0,
            serialization_error: Some(error),
        }
    }

    fn too_large(
        request: &ApiRequest,
        original_bytes: usize,
        final_bytes: usize,
        limit_bytes: usize,
    ) -> Self {
        Self {
            original_bytes,
            final_bytes,
            limit_bytes,
            system_bytes: serialized_len(&request.system).unwrap_or(0),
            tools_bytes: serialized_len(&request.tools).unwrap_or(0),
            messages_bytes: serialized_len(&request.messages).unwrap_or(0),
            serialization_error: None,
        }
    }
}

impl fmt::Display for RequestSizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(error) = &self.serialization_error {
            return write!(
                formatter,
                "Could not serialize the Anthropic /v1/messages payload locally: {error}"
            );
        }
        write!(
            formatter,
            "Anthropic /v1/messages payload is {} bytes after local compaction (initially {} bytes; safe limit {} bytes below the provider's ~32 MiB cap). Preserved components include about {} bytes of system prompt, {} bytes of tool definitions, and {} bytes of messages. Shorten system/instruction files or tool schemas, remove or resize attachments, redirect large tool output to a file and read targeted sections, shorten the latest prompt, or start a new/compacted conversation.",
            self.final_bytes,
            self.original_bytes,
            self.limit_bytes,
            self.system_bytes,
            self.tools_bytes,
            self.messages_bytes,
        )
    }
}

impl std::error::Error for RequestSizeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.serialization_error
            .as_ref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}

fn serialized_len<T: Serialize + ?Sized>(value: &T) -> Result<usize, serde_json::Error> {
    serde_json::to_vec(value).map(|body| body.len())
}

/// Return the exact compact serde JSON byte length used for the outbound body.
pub fn serialized_request_len(request: &ApiRequest) -> Result<usize, RequestSizeError> {
    serialized_len(request).map_err(RequestSizeError::serialization)
}

/// Bound the final serialized Anthropic request before it reaches the network.
/// This runs after system blocks, tool schemas, cache markers, metadata, base64,
/// and tool outputs have all been placed in the exact outbound `ApiRequest`.
pub fn preflight_messages_request(
    request: &mut ApiRequest,
) -> Result<RequestSizeCompaction, RequestSizeError> {
    preflight_messages_request_with_limit(request, MAX_REQUEST_BYTES)
}

/// Explicit-limit seam used by focused boundary tests.
pub fn preflight_messages_request_with_limit(
    request: &mut ApiRequest,
    max_bytes: usize,
) -> Result<RequestSizeCompaction, RequestSizeError> {
    let original_bytes = serialized_request_len(request)?;
    let mut stats = RequestSizeCompaction {
        original_bytes,
        final_bytes: original_bytes,
        ..RequestSizeCompaction::default()
    };
    if original_bytes <= max_bytes {
        return Ok(stats);
    }

    let mut dropped_messages = Vec::new();

    // Prefer whole old turns over mutating recent useful context. A suffix is
    // accepted only when roles alternate and every tool_use/tool_result relation
    // remains intact, so tool exchanges are removed as an atomic history unit.
    drop_oldest_valid_prefixes(
        request,
        max_bytes,
        RECENT_MESSAGES_TO_PRESERVE,
        &mut dropped_messages,
        &mut stats,
    )?;

    while serialized_request_len(request)? > max_bytes && omit_oldest_image(request) {
        stats.images_omitted += 1;
    }
    while serialized_request_len(request)? > max_bytes
        && truncate_oldest_tool_result(request, max_bytes)?
    {
        stats.tool_results_truncated += 1;
    }
    while serialized_request_len(request)? > max_bytes && omit_oldest_thinking(request) {
        stats.thinking_blocks_omitted += 1;
    }
    while serialized_request_len(request)? > max_bytes
        && truncate_oldest_history_text(request, max_bytes)?
    {
        stats.history_blocks_truncated += 1;
    }

    // If recent history itself remains unusually dense, retain the newest valid
    // structural suffix. The newest message is never silently text-truncated.
    drop_oldest_valid_prefixes(request, max_bytes, 1, &mut dropped_messages, &mut stats)?;

    stats.final_bytes = serialized_request_len(request)?;
    if stats.final_bytes > max_bytes {
        return Err(RequestSizeError::too_large(
            request,
            stats.original_bytes,
            stats.final_bytes,
            max_bytes,
        ));
    }

    if !dropped_messages.is_empty() {
        let capsule = build_context_capsule(&dropped_messages, MAX_CONTEXT_CAPSULE_BYTES);
        stats.context_capsule_bytes = insert_context_capsule_to_fit(request, capsule, max_bytes)?;
        stats.final_bytes = serialized_request_len(request)?;
    }

    Ok(stats)
}

fn drop_oldest_valid_prefixes(
    request: &mut ApiRequest,
    max_bytes: usize,
    minimum_remaining: usize,
    dropped_messages: &mut Vec<ApiMessage>,
    stats: &mut RequestSizeCompaction,
) -> Result<(), RequestSizeError> {
    while serialized_request_len(request)? > max_bytes {
        let max_drop = request.messages.len().saturating_sub(minimum_remaining);
        let Some(prefix_len) =
            (1..=max_drop).find(|&start| valid_anthropic_history(&request.messages[start..]))
        else {
            break;
        };
        dropped_messages.extend(request.messages.drain(..prefix_len));
        stats.messages_dropped += prefix_len;
    }
    Ok(())
}

fn valid_anthropic_history(messages: &[ApiMessage]) -> bool {
    if messages.is_empty() || messages[0].role != "user" {
        return false;
    }
    if messages.windows(2).any(|pair| pair[0].role == pair[1].role) {
        return false;
    }
    for (index, message) in messages.iter().enumerate() {
        for block in &message.content {
            match block {
                ApiContentBlock::ToolUse { id, .. } => {
                    let matched = message.role == "assistant"
                        && messages.get(index + 1).is_some_and(|next| {
                            next.role == "user"
                                && next.content.iter().any(|candidate| {
                                    matches!(
                                        candidate,
                                        ApiContentBlock::ToolResult { tool_use_id, .. }
                                            if tool_use_id == id
                                    )
                                })
                        });
                    if !matched {
                        return false;
                    }
                }
                ApiContentBlock::ToolResult { tool_use_id, .. } => {
                    let matched = message.role == "user"
                        && index > 0
                        && messages[index - 1].role == "assistant"
                        && messages[index - 1].content.iter().any(|candidate| {
                            matches!(
                                candidate,
                                ApiContentBlock::ToolUse { id, .. } if id == tool_use_id
                            )
                        });
                    if !matched {
                        return false;
                    }
                }
                _ => {}
            }
        }
    }
    true
}

fn omit_oldest_image(request: &mut ApiRequest) -> bool {
    for message in &mut request.messages {
        for block in &mut message.content {
            match block {
                ApiContentBlock::Image { source } => {
                    let notice = image_omitted_notice(source);
                    *block = ApiContentBlock::Text {
                        text: notice,
                        cache_control: None,
                    };
                    return true;
                }
                ApiContentBlock::ToolResult {
                    content: ToolResultContent::Blocks(blocks),
                    ..
                } => {
                    if let Some(index) = blocks
                        .iter()
                        .position(|item| matches!(item, ToolResultContentBlock::Image { .. }))
                    {
                        let ToolResultContentBlock::Image { source } = &blocks[index] else {
                            unreachable!();
                        };
                        blocks[index] = ToolResultContentBlock::Text {
                            text: image_omitted_notice(source),
                        };
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

fn image_omitted_notice(source: &ApiImageSource) -> String {
    format!(
        "[Image omitted by daanio because the serialized Anthropic request exceeded the safe payload limit: media_type={}, original_base64_chars={}. Reattach a smaller image if it is still needed.]",
        source.media_type,
        source.data.len()
    )
}

fn truncate_oldest_tool_result(
    request: &mut ApiRequest,
    max_bytes: usize,
) -> Result<bool, RequestSizeError> {
    let excess = serialized_request_len(request)?.saturating_sub(max_bytes);
    for message in &mut request.messages {
        for block in &mut message.content {
            let ApiContentBlock::ToolResult { content, .. } = block else {
                continue;
            };
            match content {
                ToolResultContent::Text(text) => {
                    if compact_text(text, excess, TOOL_RESULT_MIN_BYTES, "Tool output") {
                        return Ok(true);
                    }
                }
                ToolResultContent::Blocks(blocks) => {
                    for block in blocks {
                        if let ToolResultContentBlock::Text { text } = block
                            && compact_text(text, excess, TOOL_RESULT_MIN_BYTES, "Tool output")
                        {
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }
    Ok(false)
}

fn omit_oldest_thinking(request: &mut ApiRequest) -> bool {
    for message in &mut request.messages {
        if let Some(index) = message
            .content
            .iter()
            .position(|block| matches!(block, ApiContentBlock::Thinking { .. }))
        {
            if message.content.len() == 1 {
                message.content[index] = ApiContentBlock::Text {
                    text: "[Earlier signed thinking omitted by daanio to fit the Anthropic request payload limit.]".to_string(),
                    cache_control: None,
                };
            } else {
                message.content.remove(index);
            }
            return true;
        }
    }
    false
}

fn truncate_oldest_history_text(
    request: &mut ApiRequest,
    max_bytes: usize,
) -> Result<bool, RequestSizeError> {
    let excess = serialized_request_len(request)?.saturating_sub(max_bytes);
    let history_len = request.messages.len().saturating_sub(1);
    for message in &mut request.messages[..history_len] {
        for block in &mut message.content {
            if let ApiContentBlock::Text { text, .. } = block
                && compact_text(
                    text,
                    excess,
                    HISTORY_TEXT_MIN_BYTES,
                    "Earlier conversation content",
                )
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn compact_text(text: &mut String, excess: usize, minimum: usize, label: &str) -> bool {
    if text.len() <= minimum {
        return false;
    }
    let original_bytes = text.len();
    let target = original_bytes
        .saturating_sub(excess.saturating_add(COMPACTION_HEADROOM_BYTES))
        .max(minimum)
        .min(original_bytes);
    if target >= original_bytes {
        return false;
    }
    let marker = format!(
        "\n\n[{label} truncated by daanio to fit the Anthropic request payload: kept selected content from {original_bytes} UTF-8 bytes. Read a targeted section or start a compacted conversation for omitted content.]\n\n"
    );
    let content_budget = target.saturating_sub(marker.len());
    let head_budget = content_budget.saturating_mul(3) / 4;
    let tail_budget = content_budget.saturating_sub(head_budget);
    let head_end = floor_char_boundary(text, head_budget);
    let tail_start = ceil_char_boundary(text, text.len().saturating_sub(tail_budget));
    *text = format!("{}{}{}", &text[..head_end], marker, &text[tail_start..]);
    true
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index < value.len() && !value.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn insert_context_capsule_to_fit(
    request: &mut ApiRequest,
    mut capsule: String,
    max_bytes: usize,
) -> Result<usize, RequestSizeError> {
    while !capsule.is_empty() {
        let mut candidate = request.clone();
        insert_context_capsule(&mut candidate.messages, capsule.clone());
        if serialized_request_len(&candidate)? <= max_bytes {
            request.messages = candidate.messages;
            return Ok(capsule.len());
        }
        let next_len = capsule.len() / 2;
        capsule = truncate_owned_at_boundary(capsule, next_len);
    }
    Ok(0)
}

fn insert_context_capsule(messages: &mut Vec<ApiMessage>, capsule: String) {
    let block = ApiContentBlock::Text {
        text: capsule,
        cache_control: None,
    };
    if let Some(first) = messages.first_mut()
        && first.role == "user"
    {
        first.content.insert(0, block);
    } else {
        messages.insert(
            0,
            ApiMessage {
                role: "user".to_string(),
                content: vec![block],
            },
        );
    }
}

fn build_context_capsule(messages: &[ApiMessage], max_bytes: usize) -> String {
    let mut capsule = String::from(
        "[Compacted earlier conversation context. Large binary data and verbose tool output were excluded.]\n",
    );
    for message in messages {
        let role = if message.role == "assistant" {
            "assistant"
        } else {
            "user"
        };
        for block in &message.content {
            let summary = summarize_context_block(block);
            if summary.is_empty() {
                continue;
            }
            let line = format!("- {role}: {summary}\n");
            if capsule.len().saturating_add(line.len()) > max_bytes {
                capsule.push_str("- [Additional earlier context omitted.]\n");
                return truncate_owned_at_boundary(capsule, max_bytes);
            }
            capsule.push_str(&line);
        }
    }
    capsule
}

fn summarize_context_block(block: &ApiContentBlock) -> String {
    match block {
        ApiContentBlock::Text { text, .. } => compact_context_text(text),
        ApiContentBlock::ToolUse {
            id, name, input, ..
        } => format!(
            "tool call name={name} id={id} input={}",
            compact_context_text(&input.to_string())
        ),
        ApiContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let output = match content {
                ToolResultContent::Text(text) => compact_context_text(text),
                ToolResultContent::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        ToolResultContentBlock::Text { text } => Some(compact_context_text(text)),
                        ToolResultContentBlock::Image { .. } => None,
                    })
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join(" | "),
            };
            format!("tool result id={tool_use_id} error={is_error} output={output}")
        }
        ApiContentBlock::Image { source } => format!(
            "image omitted media_type={} original_base64_chars={}",
            source.media_type,
            source.data.len()
        ),
        ApiContentBlock::Thinking { .. } => String::new(),
    }
}

fn compact_context_text(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_owned_at_boundary(normalized, MAX_CONTEXT_ITEM_BYTES)
}

fn truncate_owned_at_boundary(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let end = floor_char_boundary(&value, max_bytes);
    value.truncate(end);
    value
}

/// Claude Code billing attribution text observed in the official CLI's system
/// prompt blocks.
pub const OAUTH_BILLING_HEADER: &str = "cc_version=2.1.123; cc_entrypoint=sdk-cli; cch=33f85;";

const CLAUDE_CODE_IDENTITY: &str = "You are a Claude agent, built on Anthropic's Claude Agent SDK.";

pub fn format_messages(messages: &[Message], is_oauth: bool) -> Vec<ApiMessage> {
    use std::collections::HashSet;

    // First pass: collect all tool_use IDs and tool_result IDs
    let mut tool_use_ids: HashSet<String> = HashSet::new();
    let mut tool_result_ids: HashSet<String> = HashSet::new();

    for msg in messages {
        for block in &msg.content {
            match block {
                ContentBlock::ToolUse { id, .. } => {
                    tool_use_ids.insert(id.clone());
                }
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    tool_result_ids.insert(tool_use_id.clone());
                }
                _ => {}
            }
        }
    }

    // Find dangling tool_uses (no matching tool_result)
    let dangling: HashSet<_> = tool_use_ids.difference(&tool_result_ids).cloned().collect();
    if !dangling.is_empty() {
        daanio_logging::info(&format!(
            "[anthropic] Repairing {} dangling tool_use(s) by injecting synthetic tool_results",
            dangling.len()
        ));
    }

    // Second pass: build messages, injecting synthetic tool_results after assistant messages
    // that have dangling tool_uses
    let mut result: Vec<ApiMessage> = Vec::new();

    for msg in messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };

        let content = format_content_blocks(&msg.content, is_oauth);

        if !content.is_empty() {
            result.push(ApiMessage {
                role: role.to_string(),
                content,
            });
        }

        // If this is an assistant message with dangling tool_uses, inject synthetic results
        if matches!(msg.role, Role::Assistant) {
            let mut synthetic_results: Vec<ApiContentBlock> = Vec::new();
            for block in &msg.content {
                if let ContentBlock::ToolUse { id, .. } = block
                    && dangling.contains(id)
                {
                    synthetic_results.push(ApiContentBlock::ToolResult {
                        tool_use_id: sanitize_tool_id(id),
                        content: ToolResultContent::Text(
                            "[Session interrupted before tool execution completed]".to_string(),
                        ),
                        is_error: true,
                    });
                }
            }
            if !synthetic_results.is_empty() {
                result.push(ApiMessage {
                    role: "user".to_string(),
                    content: synthetic_results,
                });
            }
        }
    }

    // Third pass: merge consecutive messages of the same role
    // Anthropic API requires strictly alternating user/assistant messages
    let pre_merge_count = result.len();
    let mut merged: Vec<ApiMessage> = Vec::new();
    for msg in result {
        if let Some(last) = merged.last_mut()
            && last.role == msg.role
        {
            last.content.extend(msg.content);
            continue;
        }
        merged.push(msg);
    }

    if merged.len() != pre_merge_count {
        daanio_logging::info(&format!(
            "[anthropic] Merged {} consecutive same-role messages",
            pre_merge_count - merged.len()
        ));
    }

    // Validate: check each assistant message with tool_use has matching tool_result in next user message
    for (i, msg) in merged.iter().enumerate() {
        if msg.role == "assistant" {
            let tool_uses: Vec<&String> = msg
                .content
                .iter()
                .filter_map(|b| {
                    if let ApiContentBlock::ToolUse { id, .. } = b {
                        Some(id)
                    } else {
                        None
                    }
                })
                .collect();

            if !tool_uses.is_empty() {
                // Check next message
                if let Some(next) = merged.get(i + 1) {
                    if next.role != "user" {
                        daanio_logging::warn(&format!(
                            "[anthropic] Message {} has tool_use but next message is {} (should be user)",
                            i, next.role
                        ));
                    } else {
                        let tool_results: std::collections::HashSet<&String> = next
                            .content
                            .iter()
                            .filter_map(|b| {
                                if let ApiContentBlock::ToolResult { tool_use_id, .. } = b {
                                    Some(tool_use_id)
                                } else {
                                    None
                                }
                            })
                            .collect();

                        for tu_id in &tool_uses {
                            if !tool_results.contains(*tu_id) {
                                daanio_logging::warn(&format!(
                                    "[anthropic] Message {} has tool_use {} but no matching tool_result in message {}",
                                    i,
                                    tu_id,
                                    i + 1
                                ));
                            }
                        }
                    }
                } else {
                    daanio_logging::warn(&format!(
                        "[anthropic] Message {} has tool_use but no next message",
                        i
                    ));
                }
            }
        }
    }

    merged
}

/// Convert our ContentBlock to Anthropic API format
pub fn format_content_blocks(blocks: &[ContentBlock], is_oauth: bool) -> Vec<ApiContentBlock> {
    let mut result: Vec<ApiContentBlock> = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text { text, .. } => {
                // A text block that immediately follows an image-bearing tool_result is the
                // "[Attached image associated with the preceding tool result: ...]" label
                // emitted alongside image tool outputs. The Anthropic API requires every
                // tool_result for a parallel tool-call turn to be contiguous in the next user
                // message; a sibling text block wedged between tool_results makes the API
                // report later tool_use ids as missing their tool_result. Fold the label into
                // the tool_result's content blocks so the tool_results stay contiguous.
                if let Some(ApiContentBlock::ToolResult {
                    content: ToolResultContent::Blocks(blocks),
                    ..
                }) = result.last_mut()
                    && blocks
                        .iter()
                        .any(|b| matches!(b, ToolResultContentBlock::Image { .. }))
                {
                    blocks.push(ToolResultContentBlock::Text { text: text.clone() });
                } else {
                    result.push(ApiContentBlock::Text {
                        text: text.clone(),
                        cache_control: None,
                    });
                }
            }
            ContentBlock::AnthropicThinking {
                thinking,
                signature,
            } => {
                result.push(ApiContentBlock::Thinking {
                    thinking: thinking.clone(),
                    signature: signature.clone(),
                });
            }
            ContentBlock::ToolUse {
                id, name, input, ..
            } => {
                result.push(ApiContentBlock::ToolUse {
                    id: sanitize_tool_id(id),
                    name: if is_oauth {
                        map_tool_name_for_oauth(name)
                    } else {
                        name.clone()
                    },
                    input: if input.is_object() {
                        input.clone()
                    } else {
                        serde_json::json!({})
                    },
                    cache_control: None,
                });
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                result.push(ApiContentBlock::ToolResult {
                    tool_use_id: sanitize_tool_id(tool_use_id),
                    content: ToolResultContent::Text(content.clone()),
                    is_error: is_error.unwrap_or(false),
                });
            }
            ContentBlock::Image { media_type, data } => {
                let img_block = ToolResultContentBlock::Image {
                    source: ApiImageSource {
                        kind: "base64".to_string(),
                        media_type: media_type.clone(),
                        data: data.clone(),
                    },
                };
                if let Some(ApiContentBlock::ToolResult { content, .. }) = result.last_mut() {
                    match content {
                        ToolResultContent::Text(text) => {
                            let text_block = ToolResultContentBlock::Text {
                                text: std::mem::take(text),
                            };
                            *content = ToolResultContent::Blocks(vec![text_block, img_block]);
                        }
                        ToolResultContent::Blocks(blocks) => {
                            blocks.push(img_block);
                        }
                    }
                } else {
                    result.push(ApiContentBlock::Image {
                        source: ApiImageSource {
                            kind: "base64".to_string(),
                            media_type: media_type.clone(),
                            data: data.clone(),
                        },
                    });
                }
            }
            _ => {}
        }
    }
    result
}

/// Convert tool definitions to Anthropic API format
/// Adds cache_control to the last tool for prompt caching
/// Local tool names that are represented by the curated Claude-Code builtin
/// definitions in OAuth mode. These keep their hand-tuned schemas/descriptions
/// (which the Anthropic subscription endpoint expects) instead of the raw
/// registry definitions; every other tool is forwarded as-is (see #409).
const OAUTH_BUILTIN_LOCAL_TOOLS: &[&str] = &[
    "subagent",
    "bash",
    "edit",
    "glob",
    "grep",
    "read",
    "schedule",
    "skill_manage",
    "write",
];

/// Anthropic accepts JSON Schema combinators inside object properties, but
/// rejects `oneOf`, `anyOf`, and `allOf` at the input schema's top level. Keep
/// the common object shape and widen top-level variants into one object whose
/// properties cover every branch. Runtime tool deserialization remains the
/// authority for action-specific constraints.
fn anthropic_input_schema(schema: &Value) -> Value {
    let Value::Object(source) = schema else {
        return json!({"type": "object", "properties": {}});
    };

    let mut output = source.clone();
    let mut merged_properties = output
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut all_of_required = Vec::new();

    for keyword in ["oneOf", "anyOf", "allOf"] {
        let Some(branches) = output
            .remove(keyword)
            .and_then(|value| value.as_array().cloned())
        else {
            continue;
        };
        for branch in branches {
            let Some(branch) = branch.as_object() else {
                continue;
            };
            if let Some(properties) = branch.get("properties").and_then(Value::as_object) {
                for (name, property) in properties {
                    merged_properties
                        .entry(name.clone())
                        .or_insert_with(|| property.clone());
                }
            }
            if keyword == "allOf"
                && let Some(required) = branch.get("required").and_then(Value::as_array)
            {
                for name in required.iter().filter_map(Value::as_str) {
                    if !all_of_required.iter().any(|existing| existing == name) {
                        all_of_required.push(name.to_string());
                    }
                }
            }
        }
    }

    output.insert("type".to_string(), Value::String("object".to_string()));
    output.insert("properties".to_string(), Value::Object(merged_properties));
    if !all_of_required.is_empty() {
        let required = output
            .entry("required".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(required) = required {
            for name in all_of_required {
                if !required
                    .iter()
                    .any(|existing| existing.as_str() == Some(&name))
                {
                    required.push(Value::String(name));
                }
            }
        }
    }
    Value::Object(output)
}

pub fn format_tools(tools: &[ToolDefinition], is_oauth: bool, cache_ttl_1h: bool) -> Vec<ApiTool> {
    if is_oauth {
        // Curated Claude-Code builtin tool definitions. These remain hand-tuned
        // because the Anthropic OAuth (subscription) endpoint expects the
        // builtin names with compatible schemas. Anything not represented here
        // is appended from the real registry below so OAuth users keep the full
        // toolset (websearch, webfetch, browser, codesearch, memory, ...).
        let mut out = vec![
            ApiTool {
                name: "Agent".to_string(),
                description: "Launch a new agent to handle complex, multi-step tasks.".to_string(),
                input_schema: json!({"type":"object","properties":{"description":{"type":"string"},"prompt":{"type":"string"},"subagent_type":{"type":"string"},"run_in_background":{"type":"boolean"}},"required":["description","prompt"],"additionalProperties":false}),
                cache_control: None,
            },
            ApiTool {
                name: "Bash".to_string(),
                description: "Executes a given bash command and returns its output.".to_string(),
                input_schema: json!({"type":"object","properties":{"command":{"type":"string"},"timeout":{"type":"integer"},"run_in_background":{"type":"boolean"}},"required":["command"],"additionalProperties":false}),
                cache_control: None,
            },
            ApiTool {
                name: "Edit".to_string(),
                description: "Performs exact string replacements in files.".to_string(),
                input_schema: json!({"type":"object","properties":{"file_path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean","default":false}},"required":["file_path","old_string","new_string"],"additionalProperties":false}),
                cache_control: None,
            },
            ApiTool {
                name: "Glob".to_string(),
                description: "Fast file pattern matching tool.".to_string(),
                input_schema: json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"}},"required":["pattern"],"additionalProperties":false}),
                cache_control: None,
            },
            ApiTool {
                name: "Grep".to_string(),
                description: "A powerful search tool built on ripgrep.".to_string(),
                input_schema: json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"glob":{"type":"string"},"output_mode":{"type":"string","enum":["content","files_with_matches","count"]},"-B":{"type":"number"},"-A":{"type":"number"},"-C":{"type":"number"},"context":{"type":"number"},"-n":{"type":"boolean"},"-i":{"type":"boolean"},"type":{"type":"string"},"head_limit":{"type":"number"},"offset":{"type":"number"},"multiline":{"type":"boolean"}},"required":["pattern"],"additionalProperties":false}),
                cache_control: None,
            },
            ApiTool {
                name: "Read".to_string(),
                description: "Reads a file from the local filesystem.".to_string(),
                input_schema: json!({"type":"object","properties":{"file_path":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","exclusiveMinimum":0},"pages":{"type":"string"}},"required":["file_path"],"additionalProperties":false}),
                cache_control: None,
            },
            ApiTool {
                name: "ScheduleWakeup".to_string(),
                description: "Schedule when to resume work in /loop dynamic mode.".to_string(),
                input_schema: json!({"type":"object","properties":{"delaySeconds":{"type":"number"},"reason":{"type":"string"},"prompt":{"type":"string"}},"required":["delaySeconds","reason","prompt"],"additionalProperties":false}),
                cache_control: None,
            },
            ApiTool {
                name: "Skill".to_string(),
                description: "Execute a skill within the main conversation".to_string(),
                input_schema: json!({"type":"object","properties":{"skill":{"type":"string"},"args":{"type":"string"}},"required":["skill"],"additionalProperties":false}),
                cache_control: None,
            },
            ApiTool {
                name: "Write".to_string(),
                description: "Writes a file to the local filesystem.".to_string(),
                input_schema: json!({"type":"object","properties":{"file_path":{"type":"string"},"content":{"type":"string"}},"required":["file_path","content"],"additionalProperties":false}),
                cache_control: None,
            },
        ];

        // Forward every other registered tool, remapping its name to the
        // OAuth-accepted form. This restores websearch/webfetch/browser/
        // codesearch/memory/swarm/multiedit/open/etc. for subscription users,
        // matching the documented "remap names, keep the full toolset" behavior
        // and the (deprecated) Claude CLI transport.
        for tool in tools {
            if OAUTH_BUILTIN_LOCAL_TOOLS.contains(&tool.name.as_str()) {
                continue;
            }
            out.push(ApiTool {
                name: map_tool_name_for_oauth(&tool.name),
                description: tool.description.clone(),
                input_schema: anthropic_input_schema(&tool.input_schema),
                cache_control: None,
            });
        }

        // Move the prompt-cache breakpoint to the final tool in the list.
        if let Some(last) = out.last_mut() {
            last.cache_control = Some(CacheControlParam::ephemeral(cache_ttl_1h));
        }

        return out;
    }

    let len = tools.len();
    tools
        .iter()
        .enumerate()
        .map(|(i, tool)| ApiTool {
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: anthropic_input_schema(&tool.input_schema),
            cache_control: if i == len - 1 {
                Some(CacheControlParam::ephemeral(cache_ttl_1h))
            } else {
                None
            },
        })
        .collect()
}

#[derive(Serialize, Clone)]
pub struct ApiRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<ApiSystem>,
    pub messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ApiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ApiMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ApiThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<ApiOutputConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    pub stream: bool,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiThinking {
    Adaptive {
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<&'static str>,
    },
    Enabled {
        budget_tokens: u32,
    },
}

#[derive(Serialize, Clone)]
pub struct ApiOutputConfig {
    pub effort: String,
}

#[derive(Serialize, Clone)]
pub struct ApiMetadata {
    pub user_id: String,
}

#[derive(Serialize, Clone)]
#[serde(untagged)]
pub enum ApiSystem {
    Blocks(Vec<ApiSystemBlock>),
}

/// Cache control for prompt caching
#[derive(Serialize, Clone)]
pub struct CacheControlParam {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<&'static str>,
}

impl CacheControlParam {
    fn ephemeral(cache_ttl_1h: bool) -> Self {
        if cache_ttl_1h {
            Self::ephemeral_1h()
        } else {
            Self {
                kind: "ephemeral",
                ttl: None,
            }
        }
    }

    fn ephemeral_1h() -> Self {
        Self {
            kind: "ephemeral",
            ttl: Some("1h"),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct ApiSystemBlock {
    #[serde(rename = "type")]
    pub block_type: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControlParam>,
}

pub fn build_system_param(system: &str, is_oauth: bool, cache_ttl_1h: bool) -> Option<ApiSystem> {
    build_system_param_split(system, "", is_oauth, cache_ttl_1h)
}

/// Build system param with split static/dynamic content for better caching
pub fn build_system_param_split(
    static_part: &str,
    dynamic_part: &str,
    is_oauth: bool,
    cache_ttl_1h: bool,
) -> Option<ApiSystem> {
    if is_oauth {
        let mut blocks = Vec::new();
        blocks.push(ApiSystemBlock {
            block_type: "text",
            text: format!("x-anthropic-billing-header: {}", OAUTH_BILLING_HEADER),
            cache_control: None,
        });
        blocks.push(ApiSystemBlock {
            block_type: "text",
            text: CLAUDE_CODE_IDENTITY.to_string(),
            cache_control: None,
        });
        // Static content - CACHED (instruction files, base prompt, skills)
        if !static_part.is_empty() {
            blocks.push(ApiSystemBlock {
                block_type: "text",
                text: static_part.to_string(),
                cache_control: Some(CacheControlParam::ephemeral(cache_ttl_1h)),
            });
        }
        // Dynamic content - NOT cached (date, git status, memory)
        if !dynamic_part.is_empty() {
            blocks.push(ApiSystemBlock {
                block_type: "text",
                text: dynamic_part.to_string(),
                cache_control: None,
            });
        }
        return Some(ApiSystem::Blocks(blocks));
    }

    // Non-OAuth: use block format with cache control for static part only
    let has_static = !static_part.is_empty();
    let has_dynamic = !dynamic_part.is_empty();

    if !has_static && !has_dynamic {
        None
    } else {
        let mut blocks = Vec::new();
        if has_static {
            blocks.push(ApiSystemBlock {
                block_type: "text",
                text: static_part.to_string(),
                cache_control: Some(CacheControlParam::ephemeral(cache_ttl_1h)),
            });
        }
        if has_dynamic {
            blocks.push(ApiSystemBlock {
                block_type: "text",
                text: dynamic_part.to_string(),
                cache_control: None,
            });
        }
        Some(ApiSystem::Blocks(blocks))
    }
}

pub fn format_messages_with_identity(
    messages: Vec<ApiMessage>,
    _is_oauth: bool,
    cache_ttl_1h: bool,
) -> Vec<ApiMessage> {
    let mut out = messages;

    // Add cache breakpoints for both OAuth and non-OAuth paths
    add_message_cache_breakpoint(&mut out, cache_ttl_1h);

    out
}

/// Add cache_control to messages for conversation caching.
///
/// Strategy: sliding two-marker window
///   - Second-to-last assistant message → READ marker (re-uses cache snapshot from previous turn)
///   - Last assistant message           → WRITE marker (creates new snapshot for the next turn)
///
/// This ensures each turn N+1 reads from turn N's conversation cache, paying only
/// cache_read_input_tokens for the already-cached history instead of full input tokens.
///
/// Budget: system (1) + tools (1) + messages (up to 2) = 4 total, within Anthropic's limit.
pub fn add_message_cache_breakpoint(messages: &mut [ApiMessage], cache_ttl_1h: bool) {
    daanio_logging::info(&format!(
        "Conversation caching: {} messages to process",
        messages.len()
    ));

    if messages.len() < 3 {
        // Need at least: user + assistant + user to be worth caching
        daanio_logging::info("Conversation caching: too few messages, skipping");
        return;
    }

    // Collect indices of up to 2 most recent assistant messages (newest first)
    let mut assistant_indices: Vec<usize> = Vec::with_capacity(2);
    for (i, msg) in messages.iter().enumerate().rev() {
        if msg.role == "assistant" {
            assistant_indices.push(i);
            if assistant_indices.len() == 2 {
                break;
            }
        }
    }

    if assistant_indices.is_empty() {
        daanio_logging::info("Conversation caching: no assistant message found");
        return;
    }

    // Place cache_control on both (newest = WRITE for next turn, older = READ from prev turn)
    let total = assistant_indices.len();
    for (slot, &idx) in assistant_indices.iter().enumerate() {
        let label = if slot == 0 {
            "WRITE (newest)"
        } else {
            "READ (prev-turn)"
        };
        let mut added = false;
        if let Some(msg) = messages.get_mut(idx) {
            for block in msg.content.iter_mut().rev() {
                match block {
                    ApiContentBlock::Text { cache_control, .. }
                    | ApiContentBlock::ToolUse { cache_control, .. } => {
                        *cache_control = Some(CacheControlParam::ephemeral(cache_ttl_1h));
                        added = true;
                        break;
                    }
                    _ => {}
                }
            }
        }
        if added {
            daanio_logging::info(&format!(
                "Conversation caching: breakpoint {}/{} at message {} [{}]",
                slot + 1,
                total,
                idx,
                label
            ));
        } else {
            daanio_logging::info(&format!(
                "Conversation caching: no cacheable block in assistant message {} [{}]",
                idx, label
            ));
        }
    }
}

#[derive(Serialize, Clone)]
pub struct ApiMessage {
    pub role: String,
    pub content: Vec<ApiContentBlock>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
pub enum ApiContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControlParam>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControlParam>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: ToolResultContent,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },
    #[serde(rename = "image")]
    Image { source: ApiImageSource },
}

#[derive(Serialize, Clone)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<ToolResultContentBlock>),
}

#[derive(Serialize, Clone)]
#[serde(tag = "type")]
pub enum ToolResultContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ApiImageSource },
}

#[derive(Serialize, Clone)]
pub struct ApiImageSource {
    #[serde(rename = "type")]
    pub kind: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Serialize, Clone)]
pub struct ApiTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControlParam>,
}

#[cfg(test)]
mod request_size_tests {
    use super::*;

    fn request(messages: Vec<ApiMessage>) -> ApiRequest {
        ApiRequest {
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 1024,
            system: None,
            messages,
            tools: None,
            metadata: None,
            thinking: None,
            output_config: None,
            temperature: None,
            service_tier: None,
            stream: true,
        }
    }

    fn message(role: &str, content: Vec<ApiContentBlock>) -> ApiMessage {
        ApiMessage {
            role: role.to_string(),
            content,
        }
    }

    fn text(value: impl Into<String>) -> ApiContentBlock {
        ApiContentBlock::Text {
            text: value.into(),
            cache_control: None,
        }
    }

    #[test]
    fn exact_serialized_boundary_is_accepted_unchanged() {
        let mut request = request(vec![message("user", vec![text("hello")])]);
        let before = serde_json::to_vec(&request).unwrap();
        let stats = preflight_messages_request_with_limit(&mut request, before.len()).unwrap();
        assert_eq!(stats.original_bytes, stats.final_bytes);
        assert_eq!(serialized_request_len(&request).unwrap(), before.len());
        assert_eq!(serde_json::to_vec(&request).unwrap(), before);
    }

    #[test]
    fn one_byte_below_irreducible_payload_returns_actionable_error() {
        let mut request = request(vec![message("user", vec![text("hello")])]);
        let exact = serialized_request_len(&request).unwrap();
        let error = preflight_messages_request_with_limit(&mut request, exact - 1).unwrap_err();
        assert_eq!(error.original_bytes, exact);
        assert_eq!(error.limit_bytes, exact - 1);
        let message = error.to_string();
        assert!(message.contains("Anthropic /v1/messages payload is"));
        assert!(message.contains("Shorten system/instruction files or tool schemas"));
        assert!(message.contains("redirect large tool output to a file"));
    }

    #[test]
    fn payload_preflight_omits_oldest_base64_image_first() {
        let image = ApiContentBlock::Image {
            source: ApiImageSource {
                kind: "base64".to_string(),
                media_type: "image/png".to_string(),
                data: "A".repeat(4096),
            },
        };
        let mut request = request(vec![
            message("user", vec![image, text("old")]),
            message("assistant", vec![text("answer")]),
            message("user", vec![text("current")]),
        ]);
        let stats = preflight_messages_request_with_limit(&mut request, 1800).unwrap();
        assert_eq!(stats.images_omitted, 1);
        assert_eq!(stats.messages_dropped, 0);
        assert!(stats.final_bytes <= 1800);
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("Image omitted by daanio"));
        assert!(json.contains("current"));
    }

    #[test]
    fn payload_preflight_truncates_large_tool_output() {
        let mut request = request(vec![
            message("user", vec![text("run the tool")]),
            message(
                "assistant",
                vec![ApiContentBlock::ToolUse {
                    id: "tool-1".to_string(),
                    name: "read".to_string(),
                    input: json!({"file_path":"large.log"}),
                    cache_control: None,
                }],
            ),
            message(
                "user",
                vec![ApiContentBlock::ToolResult {
                    tool_use_id: "tool-1".to_string(),
                    content: ToolResultContent::Text("x".repeat(900_000)),
                    is_error: false,
                }],
            ),
            message("assistant", vec![text("observed")]),
            message("user", vec![text("continue")]),
        ]);
        let stats = preflight_messages_request_with_limit(&mut request, 400_000).unwrap();
        assert_eq!(stats.tool_results_truncated, 1);
        assert_eq!(stats.messages_dropped, 0);
        assert!(stats.final_bytes <= 400_000);
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("Tool output truncated by daanio"));
        assert!(json.contains("continue"));
        assert!(valid_anthropic_history(&request.messages));
    }

    #[test]
    fn dropped_history_becomes_a_bounded_context_capsule() {
        let mut request = request(vec![
            message(
                "user",
                vec![text(format!("oldest-marker {}", "a".repeat(20_000)))],
            ),
            message("assistant", vec![text("oldest-answer")]),
            message("user", vec![text("recent question")]),
            message("assistant", vec![text("recent answer")]),
            message("user", vec![text("current request")]),
        ]);
        let target = serialized_request_len(&request).unwrap() - 10_000;
        // The latest exchange and current message alone must fit, which makes
        // the oldest complete exchange removable before text mutation.
        let recent_only = request.messages[2..].to_vec();
        let mut recent_request = request.clone();
        recent_request.messages = recent_only;
        let target = target.max(serialized_request_len(&recent_request).unwrap() + 256);
        let stats = preflight_messages_request_with_limit(&mut request, target).unwrap();
        assert!(stats.messages_dropped >= 1);
        assert!(stats.context_capsule_bytes > 0);
        assert!(stats.context_capsule_bytes <= MAX_CONTEXT_CAPSULE_BYTES);
        assert!(stats.final_bytes <= target);
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("Compacted earlier conversation context"));
        assert!(json.contains("oldest-marker"));
        assert!(json.contains("recent question"));
        assert!(json.contains("current request"));
        assert!(valid_anthropic_history(&request.messages));
    }

    #[test]
    fn context_capsule_is_deterministic_and_excludes_base64() {
        let dropped = vec![message(
            "user",
            vec![
                text("decision: keep retry policy"),
                ApiContentBlock::Image {
                    source: ApiImageSource {
                        kind: "base64".to_string(),
                        media_type: "image/png".to_string(),
                        data: "SECRETBASE64".repeat(100),
                    },
                },
            ],
        )];
        let first = build_context_capsule(&dropped, MAX_CONTEXT_CAPSULE_BYTES);
        let second = build_context_capsule(&dropped, MAX_CONTEXT_CAPSULE_BYTES);
        assert_eq!(first, second);
        assert!(first.contains("decision: keep retry policy"));
        assert!(first.contains("image omitted media_type=image/png"));
        assert!(!first.contains("SECRETBASE64"));
    }

    #[test]
    fn oldest_tool_use_and_result_are_dropped_as_a_valid_unit() {
        let mut request = request(vec![
            message("user", vec![text("old tool question")]),
            message(
                "assistant",
                vec![ApiContentBlock::ToolUse {
                    id: "call-1".to_string(),
                    name: "read".to_string(),
                    input: json!({"file_path":"old.log","padding":"x".repeat(12_000)}),
                    cache_control: None,
                }],
            ),
            message(
                "user",
                vec![ApiContentBlock::ToolResult {
                    tool_use_id: "call-1".to_string(),
                    content: ToolResultContent::Text("old result".to_string()),
                    is_error: false,
                }],
            ),
            message("assistant", vec![text("old tool answer")]),
            message("user", vec![text("recent question")]),
            message("assistant", vec![text("recent answer")]),
            message("user", vec![text("current request")]),
        ]);
        let mut recent_request = request.clone();
        recent_request.messages = request.messages[4..].to_vec();
        let limit = serialized_request_len(&recent_request).unwrap() + 256;

        let stats = preflight_messages_request_with_limit(&mut request, limit).unwrap();

        assert!(stats.messages_dropped >= 4);
        assert!(valid_anthropic_history(&request.messages));
        let json = serde_json::to_string(&request).unwrap();
        assert!(!json.contains("\"type\":\"tool_use\""));
        assert!(!json.contains("\"type\":\"tool_result\""));
        assert!(json.contains("recent question"));
        assert!(json.contains("current request"));
    }

    #[test]
    fn payload_preflight_errors_when_current_message_alone_cannot_fit() {
        let mut request = request(vec![message("user", vec![text("z".repeat(4096))])]);
        let error = preflight_messages_request_with_limit(&mut request, 512).unwrap_err();
        assert!(error.final_bytes > error.limit_bytes);
        assert!(error.to_string().contains("shorten the latest prompt"));
    }

    #[test]
    fn exact_length_counts_system_tools_and_base64() {
        let mut request = request(vec![message(
            "user",
            vec![ApiContentBlock::Image {
                source: ApiImageSource {
                    kind: "base64".to_string(),
                    media_type: "image/png".to_string(),
                    data: "QUJDRA==".repeat(100),
                },
            }],
        )]);
        request.system = build_system_param("system-marker", false, false);
        request.tools = Some(vec![ApiTool {
            name: "tool-marker".to_string(),
            description: "description".repeat(20),
            input_schema: json!({"type":"object","properties":{"value":{"type":"string"}}}),
            cache_control: None,
        }]);
        assert_eq!(
            serialized_request_len(&request).unwrap(),
            serde_json::to_vec(&request).unwrap().len()
        );
    }
}

#[cfg(test)]
mod cache_prefix_invariant_tests {
    //! Deterministic proof that injecting a trailing memory message can never move
    //! the Anthropic prefix-cache breakpoints off the stable assistant prefix.
    //!
    //! Anthropic caching is strict-prefix: a `cache_control` breakpoint caches every
    //! token up to and including the block it sits on. `add_message_cache_breakpoint`
    //! always anchors the two breakpoints on the two most recent *assistant* messages.
    //! Memory is injected by the agent as a trailing *user* message (see
    //! `turn_loops.rs` / `turn_streaming_mpsc.rs`). Therefore the breakpoint anchors,
    //! and every token they cache, are identical with or without the memory suffix.
    //! These tests pin that invariant so a refactor cannot silently break the cache.

    use super::*;
    use daanio_message_types::{ContentBlock, Message, Role};

    fn text_msg(role: Role, text: &str) -> Message {
        Message {
            role,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        }
    }

    /// A realistic warm conversation: user/assistant turns ending on a user message.
    fn base_conversation() -> Vec<Message> {
        vec![
            text_msg(Role::User, "Q1"),
            text_msg(Role::Assistant, "A1"),
            text_msg(Role::User, "Q2"),
            text_msg(Role::Assistant, "A2"),
            text_msg(Role::User, "Q3"),
        ]
    }

    /// Returns the indices of ApiMessages that carry a cache_control breakpoint,
    /// paired with the role of that message.
    fn breakpoint_anchors(messages: &[ApiMessage]) -> Vec<(usize, String)> {
        messages
            .iter()
            .enumerate()
            .filter_map(|(i, msg)| {
                let has_bp = msg.content.iter().any(|block| {
                    matches!(
                        block,
                        ApiContentBlock::Text {
                            cache_control: Some(_),
                            ..
                        } | ApiContentBlock::ToolUse {
                            cache_control: Some(_),
                            ..
                        }
                    )
                });
                has_bp.then(|| (i, msg.role.clone()))
            })
            .collect()
    }

    /// Serialize only the prefix up to and including the last breakpoint. This is the
    /// exact span Anthropic caches; if it is byte-identical across two requests, the
    /// cache is guaranteed to be reused.
    fn cached_prefix_json(messages: &[ApiMessage]) -> String {
        let last_bp = breakpoint_anchors(messages)
            .last()
            .map(|(idx, _)| *idx)
            .expect("expected at least one cache breakpoint");
        serde_json::to_string(&messages[..=last_bp]).expect("serialize cached prefix")
    }

    fn formatted_with_breakpoints(messages: &[Message]) -> Vec<ApiMessage> {
        let mut api = format_messages(messages, false);
        add_message_cache_breakpoint(&mut api, false);
        api
    }

    #[test]
    fn breakpoints_anchor_on_assistant_messages_only() {
        let api = formatted_with_breakpoints(&base_conversation());
        let anchors = breakpoint_anchors(&api);
        assert!(!anchors.is_empty(), "expected breakpoints to be placed");
        for (idx, role) in &anchors {
            assert_eq!(
                role, "assistant",
                "breakpoint at message {idx} must be on an assistant message, got {role}"
            );
        }
    }

    #[test]
    fn trailing_memory_message_does_not_move_breakpoints() {
        let base = base_conversation();
        let mut with_memory = base.clone();
        with_memory.push(text_msg(
            Role::User,
            "<memory>relevant recall injected for this turn</memory>",
        ));

        let base_api = formatted_with_breakpoints(&base);
        let mem_api = formatted_with_breakpoints(&with_memory);

        let base_anchors = breakpoint_anchors(&base_api);
        let mem_anchors = breakpoint_anchors(&mem_api);

        assert_eq!(
            base_anchors, mem_anchors,
            "memory suffix moved the cache breakpoints: {base_anchors:?} -> {mem_anchors:?}"
        );
    }

    #[test]
    fn cached_prefix_is_byte_identical_with_and_without_memory() {
        let base = base_conversation();
        let mut with_memory = base.clone();
        with_memory.push(text_msg(
            Role::User,
            "<memory>turn-specific recall</memory>",
        ));

        let base_prefix = cached_prefix_json(&formatted_with_breakpoints(&base));
        let mem_prefix = cached_prefix_json(&formatted_with_breakpoints(&with_memory));

        assert_eq!(
            base_prefix, mem_prefix,
            "the cached prefix span differs once memory is appended; cache would be invalidated"
        );
    }

    #[test]
    fn different_memory_each_turn_keeps_identical_cached_prefix() {
        // The memory content changes every turn. Because it is a trailing user message
        // placed *after* the newest assistant breakpoint, the cached prefix must remain
        // identical regardless of what memory is injected.
        let base = base_conversation();
        let cached = cached_prefix_json(&formatted_with_breakpoints(&base));

        for memory in [
            "<memory>recall A</memory>",
            "<memory>completely different recall B with more text</memory>",
            "",
        ] {
            let mut msgs = base.clone();
            if !memory.is_empty() {
                msgs.push(text_msg(Role::User, memory));
            }
            let candidate = cached_prefix_json(&formatted_with_breakpoints(&msgs));
            assert_eq!(
                cached, candidate,
                "memory variant {memory:?} changed the cached prefix span"
            );
        }
    }

    fn tool_def(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("{name} description"),
            input_schema: json!({"type":"object","properties":{}}),
        }
    }

    #[test]
    fn format_tools_removes_top_level_combinators_for_anthropic_api() {
        let tool = ToolDefinition {
            name: "custom".to_string(),
            description: "schema compatibility regression".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {"type": "string"},
                    "nested_union": {
                        "anyOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]
                    }
                },
                "required": ["action"],
                "oneOf": [
                    {"type": "object", "properties": {"label": {"type": "string"}}},
                    {"type": "object", "properties": {"task_id": {"type": "string"}}}
                ],
                "allOf": [
                    {"type": "object", "properties": {"intent": {"type": "string"}}, "required": ["intent"]}
                ]
            }),
        };

        let formatted = format_tools(&[tool], false, false);
        let schema = &formatted[0].input_schema;
        for keyword in ["oneOf", "anyOf", "allOf"] {
            assert!(
                schema.get(keyword).is_none(),
                "Anthropic rejects top-level {keyword}: {schema}"
            );
        }
        for property in ["action", "nested_union", "label", "task_id", "intent"] {
            assert!(
                schema["properties"].get(property).is_some(),
                "missing merged property {property}: {schema}"
            );
        }
        assert!(
            schema["properties"]["nested_union"].get("anyOf").is_some(),
            "nested combinators remain supported and should not be flattened"
        );
        assert_eq!(schema["required"], json!(["action", "intent"]));
    }

    #[test]
    fn oauth_format_tools_keeps_full_custom_toolset() {
        // Registry includes builtins (remapped) plus extra tools that must survive.
        let registry = vec![
            tool_def("bash"),
            tool_def("read"),
            tool_def("subagent"),
            tool_def("websearch"),
            tool_def("webfetch"),
            tool_def("browser"),
            tool_def("codesearch"),
            tool_def("memory"),
        ];

        let formatted = format_tools(&registry, true, false);
        let names: Vec<&str> = formatted.iter().map(|t| t.name.as_str()).collect();

        // Curated builtins are present under their OAuth names.
        for builtin in ["Bash", "Read", "Agent", "Write", "Edit", "Glob", "Grep"] {
            assert!(
                names.contains(&builtin),
                "missing builtin {builtin} in {names:?}"
            );
        }
        // The previously-dropped custom tools are now forwarded.
        for custom in ["websearch", "webfetch", "browser", "codesearch", "memory"] {
            assert!(
                names.contains(&custom),
                "custom tool {custom} was dropped on OAuth; got {names:?}"
            );
        }
        // No duplicate Agent/Bash/Read from the registry remap.
        assert_eq!(names.iter().filter(|n| **n == "Agent").count(), 1);
        assert_eq!(names.iter().filter(|n| **n == "Bash").count(), 1);
        assert_eq!(names.iter().filter(|n| **n == "Read").count(), 1);
    }

    #[test]
    fn oauth_format_tools_places_single_cache_breakpoint_on_last_tool() {
        let registry = vec![tool_def("bash"), tool_def("websearch")];
        let formatted = format_tools(&registry, true, false);
        let with_cache: Vec<&str> = formatted
            .iter()
            .filter(|t| t.cache_control.is_some())
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(with_cache.len(), 1, "expected exactly one cache breakpoint");
        assert_eq!(
            formatted.last().map(|t| t.name.as_str()),
            with_cache.first().copied(),
            "cache breakpoint must be on the final tool"
        );
    }
}

# Daanio Model Context Windows and Modes

> Generated from the live Daanio database on **2026-08-24 05:53 UTC**.

This document lists every model with at least one enabled routing ability. Context and capability values use Daanio's exact/prefix/contains/suffix model-rule precedence. **Not specified** means Daanio has no configured value; the table does not guess.

The machine-readable source for this snapshot is [daanio-model-catalog-2026-08-24.json](daanio-model-catalog-2026-08-24.json).

## Summary

- Enabled models: **36**
- Models with configured context windows: **34**
- Models with advertised reasoning modes: **28**
- A context window includes input plus generated output unless the provider documents a separate maximum-input restriction.

## Reasoning mode legend

| Mode | Meaning |
|---|---|
| `none` | No deliberate reasoning |
| `minimal` | Minimal reasoning overhead |
| `low` | Low reasoning effort |
| `medium` | Balanced reasoning effort |
| `high` | High reasoning effort |
| `xhigh` | Extra-high reasoning effort |
| `max` | Maximum supported reasoning effort |

## Anthropic / Claude

| Model | Context window | Max output | Reasoning modes | Image input | Enabled groups |
|---|---:|---:|---|:---:|---|
| `claude-fable-5` | 1,000,000 | Not specified | `none`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-haiku-4-5-20251001` | 200,000 | Not specified | None advertised | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-opus-4-1-20250805` | 200,000 | Not specified | None advertised | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-opus-4-5-20251101` | 200,000 | Not specified | `none`, `low`, `medium`, `high` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-opus-4-6` | 200,000 | Not specified | `none`, `low`, `medium`, `high`, `max` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-opus-4-6-thinking` | 200,000 | Not specified | `none`, `low`, `medium`, `high`, `max` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-opus-4-7` | 1,000,000 | Not specified | `none`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-opus-4-8` | 1,000,000 | Not specified | `none`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-opus-4-8[1m]` | 1,000,000 | Not specified | `none`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-opus-5` | 1,000,000 | Not specified | `none`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-sonnet-4-5-20250929` | 200,000 | Not specified | None advertised | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-sonnet-4-6` | 200,000 | Not specified | `none`, `low`, `medium`, `high`, `max` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |
| `claude-sonnet-5` | 1,000,000 | Not specified | `none`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `Claude API External`, `Claude Code Non-External`, `cli`, `mobile` |

## Google / Gemini

| Model | Context window | Max output | Reasoning modes | Image input | Enabled groups |
|---|---:|---:|---|:---:|---|
| `gemini-3-flash` | 1,000,000 | Not specified | `minimal`, `low`, `medium`, `high` | Yes | `cli`, `Gemini API External`, `Gemini Non-External`, `mobile` |
| `gemini-3-flash-agent` | 1,000,000 | Not specified | `minimal`, `low`, `medium`, `high` | Yes | `cli`, `Gemini API External`, `Gemini Non-External`, `mobile` |
| `gemini-3.1-flash-image` | 1,000,000 | Not specified | `minimal`, `low`, `medium`, `high` | Yes | `cli`, `mobile` |
| `gemini-3.1-flash-lite` | 1,000,000 | Not specified | `minimal`, `low`, `medium`, `high` | Yes | `cli`, `Gemini API External`, `Gemini Non-External`, `mobile` |
| `gemini-3.1-pro-low` | 1,000,000 | Not specified | `minimal`, `low`, `medium`, `high` | Yes | `cli`, `Gemini API External`, `Gemini Non-External`, `mobile` |
| `gemini-3.5-flash-extra-low` | 1,000,000 | Not specified | `minimal`, `low`, `medium`, `high` | Yes | `cli`, `Gemini API External`, `Gemini Non-External`, `mobile` |
| `gemini-3.5-flash-low` | 1,000,000 | Not specified | `minimal`, `low`, `medium`, `high` | Yes | `cli`, `Gemini API External`, `Gemini Non-External`, `mobile` |
| `gemini-3.6-flash-high` | 1,000,000 | Not specified | `minimal`, `low`, `medium`, `high` | Yes | `cli`, `Gemini API External`, `Gemini Non-External`, `mobile` |
| `gemini-pro-agent` | 1,000,000 | Not specified | `minimal`, `low`, `medium`, `high` | Yes | `cli`, `Gemini API External`, `Gemini Non-External`, `mobile` |

## Moonshot / Kimi

| Model | Context window | Max output | Reasoning modes | Image input | Enabled groups |
|---|---:|---:|---|:---:|---|
| `kimi-k2.5` | 262,144 | Not specified | None advertised | No | `cli`, `Kimi API External`, `Kimi Non-External`, `mobile` |
| `kimi-k2.6` | 262,144 | Not specified | None advertised | No | `cli`, `Kimi API External`, `Kimi Non-External`, `mobile` |
| `kimi-k3` | 262,144 | Not specified | None advertised | No | `cli`, `Kimi API External`, `Kimi Non-External`, `mobile` |

## OpenAI / GPT

| Model | Context window | Max output | Reasoning modes | Image input | Enabled groups |
|---|---:|---:|---|:---:|---|
| `gpt-5.4` | 1,050,000 | Not specified | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `cli`, `Codex API External`, `Codex Non-External`, `mobile` |
| `gpt-5.4-mini` | 400,000 | Not specified | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `cli`, `Codex API External`, `Codex Non-External`, `mobile` |
| `gpt-5.5` | 1,050,000 | Not specified | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `cli`, `Codex API External`, `Codex Non-External`, `mobile` |
| `gpt-5.6-luna` | 1,050,000 | Not specified | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `cli`, `Codex API External`, `Codex Non-External`, `mobile` |
| `gpt-5.6-sol` | 1,050,000 | Not specified | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `cli`, `Codex API External`, `Codex Non-External`, `mobile` |
| `gpt-5.6-terra` | 1,050,000 | Not specified | `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` | Yes | `cli`, `Codex API External`, `Codex Non-External`, `mobile` |
| `gpt-image-1.5` | Not specified | Not specified | None advertised | No | `cli`, `mobile` |
| `gpt-image-2` | Not specified | Not specified | None advertised | No | `cli`, `mobile` |

## xAI / Grok

| Model | Context window | Max output | Reasoning modes | Image input | Enabled groups |
|---|---:|---:|---|:---:|---|
| `grok-4.3` | 262,144 | Not specified | `low`, `high` | Yes | `cli`, `mobile`, `xAI API External` |
| `grok-4.5` | 262,144 | Not specified | `low`, `high` | Yes | `cli`, `mobile`, `xAI API External` |
| `grok-4.6` | 500,000 | Not specified | `low`, `high` | Yes | `cli`, `mobile`, `xAI API External` |

## Verified corrected values

| Model | Context window | Source |
|---|---:|---|
| `grok-4.6` | 500,000 | Official xAI Grok 4.6 documentation |
| `gpt-5.4` | 1,050,000 | Official OpenAI model documentation |
| `gpt-5.4-mini` | 400,000 | Official OpenAI model documentation |
| `gpt-5.5` | 1,050,000 | Official OpenAI model documentation |
| `gpt-5.6-sol` | 1,050,000 | Official OpenAI GPT-5.6 documentation |
| `gpt-5.6-terra` | 1,050,000 | Official OpenAI GPT-5.6 family documentation |
| `gpt-5.6-luna` | 1,050,000 | Official OpenAI GPT-5.6 family documentation |

## Provider-specific notes

- GPT-5.4 Mini has a **400,000-token total context window** and an official **272,000 maximum input-token** limit.
- GPT-5.6 has a **1,050,000-token total context window** and an official **922,000 maximum input-token** limit.
- Grok 4.6 has a **500,000-token context window**. xAI applies higher-context pricing above 200,000 input tokens.
- Models whose context is **Not specified** remain usable. Daanio simply does not advertise a context value for them.
- The authenticated Daanio `/v1/models` catalog remains authoritative at runtime for context, reasoning modes, image input, and future model additions. Static CLI values are compatibility fallbacks for startup and offline resolution.

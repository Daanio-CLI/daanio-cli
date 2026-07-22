#!/usr/bin/env bash
set -euo pipefail

# Run Terminal-Bench through Harbor with daanio using Opus 4.8.
# Default route is OpenRouter (anthropic/claude-opus-4.8) since native Claude
# OAuth may be unavailable. Override with DAANIO_TB_MODEL / env vars.

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd)
DEFAULT_BINARY_DIR=${DAANIO_HARBOR_BINARY_DIR:-/tmp/daanio-compat-dist}
DEFAULT_BINARY_PATH=${DAANIO_HARBOR_BINARY:-$DEFAULT_BINARY_DIR/daanio-linux-x86_64.bin}
DEFAULT_MODEL=${DAANIO_TB_MODEL:-anthropic-api/claude-opus-4-8}
DEFAULT_PATH=${DAANIO_TB_PATH:-/tmp/terminal-bench-2.1}

have_model=0
have_agent_import=0
have_task_source=0

for arg in "$@"; do
  case "$arg" in
    --model|-m)
      have_model=1
      ;;
    --agent-import-path)
      have_agent_import=1
      ;;
    --path|-p|--dataset|-d|--task|-t)
      have_task_source=1
      ;;
  esac
done

if [[ ! -x "$DEFAULT_BINARY_PATH" ]]; then
  echo "Building Linux-compatible daanio binary into $DEFAULT_BINARY_DIR" >&2
  "$REPO_ROOT/scripts/build_linux_compat.sh" "$DEFAULT_BINARY_DIR"
fi

# Resolve provider keys from daanio's env files if not already set.
if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
  OR_ENV=${DAANIO_HARBOR_OPENROUTER_ENV:-$HOME/.config/daanio/openrouter.env}
  if [[ -f "$OR_ENV" ]]; then
    export DAANIO_HARBOR_OPENROUTER_ENV="$OR_ENV"
  fi
fi
if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
  ANT_ENV=${DAANIO_HARBOR_ANTHROPIC_ENV:-$HOME/.config/daanio/anthropic.env}
  if [[ -f "$ANT_ENV" ]]; then
    export DAANIO_HARBOR_ANTHROPIC_ENV="$ANT_ENV"
  fi
fi

export PYTHONPATH="$REPO_ROOT/scripts${PYTHONPATH:+:$PYTHONPATH}"
export DAANIO_HARBOR_BINARY="$DEFAULT_BINARY_PATH"
export DAANIO_ANTHROPIC_REASONING_EFFORT=${DAANIO_ANTHROPIC_REASONING_EFFORT:-high}
export DAANIO_NO_TELEMETRY=${DAANIO_NO_TELEMETRY:-1}

HARBOR_BIN=${DAANIO_HARBOR_BIN:-harbor}

cmd=($HARBOR_BIN run)
if [[ $have_task_source -eq 0 ]]; then
  cmd+=(--path "$DEFAULT_PATH")
fi
if [[ $have_agent_import -eq 0 ]]; then
  cmd+=(--agent-import-path daanio_harbor_claude_agent:DaanioClaudeHarborAgent)
fi
if [[ $have_model -eq 0 ]]; then
  cmd+=(--model "$DEFAULT_MODEL")
fi
cmd+=("$@")

{
  echo "Running Harbor with daanio Opus 4.8 adapter"
  echo "  binary: $DAANIO_HARBOR_BINARY"
  echo "  model:  ${DEFAULT_MODEL}"
} >&2

exec "${cmd[@]}"

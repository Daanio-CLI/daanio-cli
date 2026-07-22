#!/usr/bin/env bash
# Shared loader for Daanio remote build defaults.
#
# The config file is intentionally a shell fragment so users can write either:
#   DAANIO_REMOTE_HOST=builder
# or:
#   export DAANIO_REMOTE_HOST=builder
#
# Explicit environment variables take precedence over values loaded from the
# config file. This lets callers temporarily disable remote builds with, for
# example, `DAANIO_REMOTE_CARGO=0 scripts/dev_cargo.sh check`.

daanio_remote_config_path() {
  if [[ -n "${DAANIO_REMOTE_CONFIG:-}" ]]; then
    printf '%s\n' "$DAANIO_REMOTE_CONFIG"
  elif [[ -n "${XDG_CONFIG_HOME:-}" ]]; then
    printf '%s\n' "$XDG_CONFIG_HOME/daanio/remote-build.env"
  elif [[ -n "${HOME:-}" ]]; then
    printf '%s\n' "$HOME/.config/daanio/remote-build.env"
  fi
}

daanio_load_remote_config() {
  local config_file
  config_file="$(daanio_remote_config_path)"
  [[ -n "$config_file" && -f "$config_file" ]] || return 0

  local had_remote_cargo=0 remote_cargo=""
  local had_remote_host=0 remote_host=""
  local had_remote_dir=0 remote_dir=""
  local had_remote_ssh_bin=0 remote_ssh_bin=""
  local had_remote_rsync_bin=0 remote_rsync_bin=""

  if [[ ${DAANIO_REMOTE_CARGO+x} ]]; then
    had_remote_cargo=1
    remote_cargo="$DAANIO_REMOTE_CARGO"
  fi
  if [[ ${DAANIO_REMOTE_HOST+x} ]]; then
    had_remote_host=1
    remote_host="$DAANIO_REMOTE_HOST"
  fi
  if [[ ${DAANIO_REMOTE_DIR+x} ]]; then
    had_remote_dir=1
    remote_dir="$DAANIO_REMOTE_DIR"
  fi
  if [[ ${DAANIO_REMOTE_SSH_BIN+x} ]]; then
    had_remote_ssh_bin=1
    remote_ssh_bin="$DAANIO_REMOTE_SSH_BIN"
  fi
  if [[ ${DAANIO_REMOTE_RSYNC_BIN+x} ]]; then
    had_remote_rsync_bin=1
    remote_rsync_bin="$DAANIO_REMOTE_RSYNC_BIN"
  fi

  # shellcheck source=/dev/null
  source "$config_file"

  if [[ "$had_remote_cargo" -eq 1 ]]; then
    DAANIO_REMOTE_CARGO="$remote_cargo"
  fi
  if [[ "$had_remote_host" -eq 1 ]]; then
    DAANIO_REMOTE_HOST="$remote_host"
  fi
  if [[ "$had_remote_dir" -eq 1 ]]; then
    DAANIO_REMOTE_DIR="$remote_dir"
  fi
  if [[ "$had_remote_ssh_bin" -eq 1 ]]; then
    DAANIO_REMOTE_SSH_BIN="$remote_ssh_bin"
  fi
  if [[ "$had_remote_rsync_bin" -eq 1 ]]; then
    DAANIO_REMOTE_RSYNC_BIN="$remote_rsync_bin"
  fi
}

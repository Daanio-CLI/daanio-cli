#!/usr/bin/env bash
# Install the current release binary into the immutable version store,
# update the stable + current channel symlinks, and point the launcher at current.
#
# Paths after install:
# - ~/.daanio/builds/versions/<hash>/daanio (immutable)
# - ~/.daanio/builds/stable/daanio -> .../versions/<hash>/daanio
# - ~/.daanio/builds/current/daanio -> .../versions/<hash>/daanio
# - ~/.local/bin/daanio -> ~/.daanio/builds/current/daanio (launcher)
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

profile="${DAANIO_RELEASE_PROFILE:-release-lto}"
if [[ "${1:-}" == "--fast" ]]; then
  profile="release"
  shift
fi

if [[ "$#" -gt 0 ]]; then
  echo "Usage: $0 [--fast]" >&2
  exit 1
fi

case "$profile" in
  release-lto)
    echo "Building with LTO (this takes a few minutes)..."
    ;;
  release)
    echo "Building fast release profile (no LTO)..."
    ;;
  *)
    echo "Unsupported profile: $profile (expected: release or release-lto)" >&2
    exit 1
    ;;
esac

cargo build --profile "$profile" --manifest-path "$repo_root/Cargo.toml"
bin="$repo_root/target/$profile/daanio"

if [[ ! -x "$bin" ]]; then
  echo "Release binary not found: $bin" >&2
  exit 1
fi

hash=""
if command -v git >/dev/null 2>&1; then
  if git -C "$repo_root" rev-parse --git-dir >/dev/null 2>&1; then
    hash="$(git -C "$repo_root" rev-parse --short HEAD 2>/dev/null || true)"
    if [[ -n "${hash}" ]] && [[ -n "$(git -C "$repo_root" status --porcelain 2>/dev/null || true)" ]]; then
      hash="${hash}-dirty"
    fi
  fi
fi

if [[ -z "$hash" ]]; then
  hash="$(date +%Y%m%d%H%M%S)"
fi

# Install versioned binary into ~/.daanio/builds/versions/<hash>/
builds_dir="$HOME/.daanio/builds"
version_dir="$builds_dir/versions/$hash"
mkdir -p "$version_dir"
install -m 755 "$bin" "$version_dir/daanio"

# Update stable symlink
stable_dir="$builds_dir/stable"
mkdir -p "$stable_dir"
ln -sfn "$version_dir/daanio" "$stable_dir/daanio"

# Update stable-version marker
printf '%s\n' "$hash" > "$builds_dir/stable-version"

# Update current symlink + marker
current_dir="$builds_dir/current"
mkdir -p "$current_dir"
ln -sfn "$version_dir/daanio" "$current_dir/daanio"
printf '%s\n' "$hash" > "$builds_dir/current-version"

# Update launcher path to current channel
install_dir="${DAANIO_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$install_dir"
ln -sfn "$current_dir/daanio" "$install_dir/daanio"

echo "Installed: $version_dir/daanio"
echo "Updated stable symlink: $stable_dir/daanio -> $version_dir/daanio"
echo "Updated current symlink: $current_dir/daanio -> $version_dir/daanio"
echo "Updated launcher symlink: $install_dir/daanio -> $current_dir/daanio"

# Configure supported desktop launch hotkeys as part of installation. This is
# idempotent and best-effort because headless installs may not expose a desktop
# session; the first interactive launch retries automatically.
case "$(uname -s)" in
  Darwin|Linux)
    if "$install_dir/daanio" setup-hotkey </dev/null >/dev/null 2>&1; then
      echo "Configured system-wide daanio launch hotkeys (when supported)."
    fi
    ;;
esac

# Gracefully reload any running background server onto the binary we just
# installed (issue #291). `server reload` only reloads when the running daemon
# is genuinely older, hands live headless/swarm sessions to the new process, and
# is a no-op when no server is running, so it is safe to call unconditionally.
if [ "${DAANIO_SKIP_SERVER_RELOAD:-}" != "1" ]; then
  if "$install_dir/daanio" server reload </dev/null >/dev/null 2>&1; then
    echo "Reloaded the running daanio server onto $hash (if one was active)."
  fi
fi

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$install_dir"; then
  echo ""
  echo "Tip: add $install_dir to PATH if needed."
fi

# Ensure the launcher dir is on PATH for bash, zsh and fish in future shells.
# shellcheck source=scripts/lib/configure_path.sh
. "$(dirname "$0")/lib/configure_path.sh"
daanio_configure_path "$install_dir"

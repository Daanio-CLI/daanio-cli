# Daanio CLI

Daanio CLI is a fast coding agent for the terminal, designed to connect to
[Daanio](https://daanio.com)'s unified OpenAI-compatible AI gateway. It supports
interactive TUI sessions, non-interactive runs, persistent server/client
workflows, Daanio-hosted model selection, MCP tools, memory, and coordinated
agents. Upstream model providers are reached only through Daanio's server-side
gateway.

This project is a renamed fork of
[1jehuang/jcode](https://github.com/1jehuang/jcode). See [NOTICE.md](NOTICE.md)
and [LICENSE](LICENSE) for attribution and license terms.

## Current status

The application identity has been changed throughout the workspace:

- executable and root package: `daanio`
- application data: `~/.daanio`
- environment variables: `DAANIO_*`
- internal Rust crates: `daanio-*` / `daanio_*`
- desktop and mobile targets: Daanio

Telemetry and sponsored discovery are disabled by default. Official release
builds update from `Daanio-CLI/daanio-cli`; development builds require an
explicit `DAANIO_GITHUB_REPOSITORY` setting. See [RELEASE_SETUP.md](RELEASE_SETUP.md)
before publishing a commercial build.

## Build from source

Prerequisites:

- Git
- a current stable Rust toolchain with Cargo
- platform build tools required by Rust dependencies

```bash
git clone https://github.com/Daanio-CLI/daanio-cli.git
cd daanio-cli
cargo build --release
./target/release/daanio --help
```

For a local user installation after building:

```bash
scripts/install_release.sh --fast
```

This installs the launcher at `~/.local/bin/daanio` and keeps application data
under `~/.daanio`.

## Quick start

Sign in to Daanio securely in your browser:

```bash
# Opens daanio.com for approval; no API-key copy/paste is required
daanio login --provider daanio

# Start the interactive terminal UI
daanio --provider daanio

# Run one prompt non-interactively
daanio run --provider daanio "explain this repository"

# Select another model shown in Daanio Model Square
daanio run --provider daanio --model gpt-5.6-sol "explain this repository"

# Start a persistent server and attach a client
daanio serve
daanio connect
```

Run `daanio --help` and `daanio <command> --help` for the complete command set.
Daanio uses `https://api.daanio.com/v1` with a browser-authorized, revocable
gateway credential. See
the official [Getting Started](https://daanio.com/docs/getting-started) and
[Codex integration](https://daanio.com/docs/codex) guides. Do not enter an
OpenAI, Anthropic, Google, OpenRouter, or other upstream-provider key: Daanio
selects and authenticates upstream models server-side through its gateway.

## Configuration

Global configuration and state use `~/.daanio`. Project-local configuration
uses `.daanio`. The primary configuration file is:

```text
~/.daanio/config.toml
```

Examples and architecture notes remain available in [docs](docs). Historical
changelog entries describe the upstream implementation from which this fork was
created.

## Privacy defaults

- Telemetry is off unless `DAANIO_TELEMETRY_ENDPOINT` is explicitly configured.
- Sponsored tool discovery is off unless `[sponsors] enabled = true` and an
  endpoint is configured.
- Official release builds check `Daanio-CLI/daanio-cli` for updates.
- Development builds do not check for updates unless
  `DAANIO_GITHUB_REPOSITORY` is set explicitly.

## Support

Use `/support` inside the TUI to prepare a diagnostic email to
`support@daanio.com`. Set `DAANIO_SUPPORT_EMAIL` only if your distribution uses
a different support address.

## Credits

Daanio CLI is a fork of Jeremy Huang's open-source
[jcode](https://github.com/1jehuang/jcode) project. We gratefully acknowledge
Jeremy Huang and all jcode contributors for the original project. See
[CREDITS.md](CREDITS.md) and [NOTICE.md](NOTICE.md) for full attribution.

## License

MIT. Retain the copyright and permission notice when distributing copies or
substantial portions. See [LICENSE](LICENSE) and [NOTICE.md](NOTICE.md).

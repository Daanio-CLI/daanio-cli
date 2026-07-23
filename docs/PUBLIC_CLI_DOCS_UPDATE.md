# Daanio CLI public documentation update

This is the content and implementation checklist for updating
[daanio.com/docs](https://daanio.com/docs). It describes the current public CLI
behavior after the guided Daanio login and live model-selection changes.

## Documentation pages to update

1. Update `/docs/getting-started` with the short install, login, and first-run
   flow below.
2. Update `/docs/install-cli` with platform-specific installation and PATH
   troubleshooting.
3. Add `/docs/cli-login` for browser login, manual Daanio API-key login,
   account commands, and credential security.
4. Add the new CLI login page to the docs sidebar near **Getting Started** and
   **Install CLI**. If Markdown routes are discovered automatically, verify the
   final title and order in the rendered sidebar.
5. Cross-link all three pages so users can move from installation to login and
   model selection without searching.

Do not publish a fixed list of models. Daanio CLI loads the account's current
chat-model catalog from the Daanio API, so a hardcoded list will become stale.

## Copy for Getting Started

The following section can be adapted directly for `/docs/getting-started`.

### Install Daanio CLI

macOS or Linux:

```bash
curl -fsSL https://daanio.com/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://daanio.com/install.ps1 | iex
```

Open a new terminal after installation, then confirm the CLI is available:

```bash
daanio --version
```

### Sign in

```bash
daanio login daanio
```

Daanio asks you to choose one of two methods:

1. **Browser sign-in (recommended)** — Daanio opens a single-use approval page
   on `daanio.com`. Sign in and approve the CLI request. You do not need to copy
   a credential into the terminal.
2. **Enter Daanio API key** — paste a first-party gateway API key created in
   your [Daanio account](https://daanio.com/account). The terminal hides the
   value, validates it with Daanio, and saves it only after validation succeeds.

The manual method accepts only a Daanio gateway credential. Do not enter an
OpenAI, Anthropic, OpenRouter, Google, or other upstream-provider key. Daanio
manages upstream model authentication on the server.

### Choose a model

Start the interactive CLI:

```bash
daanio
```

After first login, Daanio automatically opens the live model picker. Choose a
model and press Enter. The selection becomes the default for the main session
and for new agents unless you explicitly choose another model later.

Use `/model` at any time to select another model. The picker comes from your
live Daanio account catalog; it is not a hardcoded list in the CLI.

### Verify the setup

```bash
daanio account status
daanio auth-test --provider daanio
```

A successful authentication test ends with `result: PASS`.

## Copy for Install CLI

Use the following expanded content on `/docs/install-cli`.

### macOS and Linux

Quick install:

```bash
curl -fsSL https://daanio.com/install.sh | bash
```

Review the installer before running it:

```bash
curl -fsSL https://daanio.com/install.sh -o /tmp/daanio-install.sh
less /tmp/daanio-install.sh
bash /tmp/daanio-install.sh
```

The installer selects the prebuilt binary for the current operating system and
CPU architecture, verifies its checksum, installs the stable build, and adds
the Daanio launcher to the user's PATH. A normal installation does not require
Rust or Cargo.

Supported release targets should include:

- macOS Apple Silicon (`aarch64`), including M1, M2, M3, and later chips
- macOS Intel (`x86_64`)
- Linux `x86_64`
- Linux ARM64 (`aarch64`)

### Windows

Quick install in PowerShell:

```powershell
irm https://daanio.com/install.ps1 | iex
```

Review the installer before running it:

```powershell
$scriptText = irm https://daanio.com/install.ps1
$scriptText
& ([scriptblock]::Create($scriptText))
```

The installer selects the Windows x64 or ARM64 release, verifies its checksum,
and installs Daanio under `%LOCALAPPDATA%\daanio` without requiring
administrator privileges. Public Windows executables should carry the Daanio
Authenticode signature.

### Verify installation

macOS or Linux:

```bash
command -v daanio
daanio --version
```

Windows PowerShell:

```powershell
Get-Command daanio
daanio --version
Get-AuthenticodeSignature (Get-Command daanio).Source
```

If `daanio` is not found immediately after installation, close and reopen the
terminal so the updated PATH is loaded. On macOS and Linux, `~/.local/bin`
should appear before `~/.cargo/bin` in PATH.

## Copy for CLI Login

Use this as the main content for `/docs/cli-login`.

### Interactive login-method selection

```bash
daanio login daanio
```

The browser method is selected by default. Use the arrow keys to choose a
method and press Enter.

For scripts or users who already know which method they want:

```bash
# OAuth 2.0 device authorization through daanio.com
daanio login daanio --method browser

# Hidden manual entry of a first-party Daanio gateway key
daanio login daanio --method api-key
```

The same choices are available through the account command:

```bash
daanio account login --method browser
daanio account login --method api-key
```

### Browser sign-in

Browser sign-in is recommended because the user never has to copy a credential.
The CLI:

1. Requests a single-use device authorization from the Daanio API.
2. Opens `https://daanio.com/device?flow=...` in the browser.
3. Waits while the user signs in and approves the request.
4. Exchanges the approved flow for a revocable Daanio CLI credential.
5. Checks `/v1/me` and stores the credential with owner-only permissions.

For SSH, containers, or terminals without a local browser:

```bash
daanio login daanio --method browser --no-browser
```

Open the printed public approval URL in any browser and complete the same
approval flow.

### Manual Daanio API-key login

Manual entry is a fallback for users who already have a Daanio gateway key:

```bash
daanio login daanio --method api-key
```

Security behavior:

- typed characters are hidden;
- the key is never printed in terminal output;
- Daanio validates the key with `https://api.daanio.com/v1/me`;
- the key is saved only if Daanio accepts it;
- an invalid, offline, or upstream-provider credential is not saved;
- the key must never be included in screenshots, support messages, logs, or
  documentation examples.

Only a key issued by Daanio belongs in this prompt. OpenAI, Anthropic,
OpenRouter, Google, and other model-provider credentials are not supported by
the public Daanio CLI.

### Model setup after login

In first-run onboarding, either successful login method continues directly to
the live model picker. The user must choose a model when no default exists.

The chosen model:

- is saved as the local CLI default for that Daanio setup;
- is used by the main session;
- is inherited by newly created agents;
- changes for future agents when the user changes the main model;
- can be overridden when a user explicitly assigns another model to an agent.

There is no silent fallback model. If neither the main session nor an agent has
a selected model, the CLI asks the user to choose one.

Use these TUI commands:

```text
/model                 Open the live model picker
/refresh-model-list    Reload the model catalog from Daanio
/account               Inspect or manage the Daanio account
/login                 Sign in again
```

The docs must not claim that a particular number of models is always available.
Availability depends on the account and server-side catalog. Image and non-chat
models are not shown in the terminal chat-model picker.

### Account management

```bash
# View the signed-in account, plan, and status
daanio account status

# Open Daanio account management
daanio account manage

# Revoke the current CLI key and remove the local credential
daanio account logout
```

### Troubleshooting

Run the guided authentication and runtime checks:

```bash
daanio auth-test --provider daanio
daanio auth doctor daanio
```

If authentication succeeds but the model list looks stale:

1. Run `/refresh-model-list` inside the TUI.
2. Open `/model` again.
3. If using a remote client/server session, reconnect and retry.

If a model request reports `model_not_found`, select a model currently returned
by the live Daanio model picker. Do not document or configure a hardcoded
fallback model.

## Required callouts

Use consistent wording across all public pages:

> Daanio CLI accepts only a Daanio gateway credential. Upstream model-provider
> API keys are managed server-side and must not be entered into the CLI.

> Browser sign-in is recommended. Manual Daanio API-key entry is available for
> users who already have a first-party gateway key.

Do not say that manual API-key login accepts an OpenAI-compatible provider key.
The Daanio gateway speaks an OpenAI-compatible runtime protocol, but its
credential is still issued and validated by Daanio.

## Documentation acceptance checklist

- [ ] Getting Started contains macOS/Linux and Windows install commands.
- [ ] Install CLI lists macOS Intel/Apple Silicon, Linux x64/ARM64, and Windows
      x64/ARM64 prebuilt targets.
- [ ] Install CLI says Cargo is not required for a normal prebuilt install.
- [ ] Login documentation shows both browser and manual Daanio-key methods.
- [ ] Browser login is visibly labeled **recommended**.
- [ ] Manual login is clearly limited to first-party Daanio gateway keys.
- [ ] No example contains a real or realistic secret credential.
- [ ] The docs explain that manual input is hidden and validated before saving.
- [ ] The first-run live model picker and `/model` command are documented.
- [ ] The docs say models come from the API and do not publish a fixed model
      count or hardcoded production list.
- [ ] Main-session model inheritance by new agents is documented.
- [ ] `daanio account status`, `daanio auth-test --provider daanio`, and logout
      recovery commands are included.
- [ ] All commands are tested against the current public release on macOS and
      Windows before deployment.
- [ ] Links to [jcode](https://github.com/1jehuang/jcode), the MIT license, and
      Daanio CLI attribution remain present on the repository/about page.

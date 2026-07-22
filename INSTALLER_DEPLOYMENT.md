# Daanio CLI installer deployment

This guide turns the existing Daanio installer scripts into public,
Claude Code-style installation commands for macOS, Linux, and Windows.

Public commands after deployment:

```bash
curl -fsSL https://daanio.com/install.sh | bash
```

```powershell
irm https://daanio.com/install.ps1 | iex
```

The source installers already exist:

- `scripts/install.sh` for macOS, Linux, and Git Bash on Windows.
- `scripts/install.ps1` for native Windows PowerShell.
- `.github/workflows/release.yml` for building and publishing release assets.

Do not publish the installer until every item in the final checklist passes.

## 1. Official release repository

The official public release repository is:

```text
DAANIO_GITHUB_REPOSITORY=Daanio-CLI/daanio-cli
```

```text
DAANIO_REPO_URL=https://github.com/Daanio-CLI/daanio-cli.git
```

Confirm no inherited Daanio distribution references remain before publishing:

```bash
rg -n '1jehuang/daanio' scripts .github docs README.md RELEASE_SETUP.md
```

The `1jehuang/jcode` references in `README.md`, `CREDITS.md`, `NOTICE.md`, and
`LICENSE` attribution are intentional and must remain.

Production metadata base:

```text
https://daanio.com/releases
```

The default in both installer scripts must match. Keep the
`DAANIO_RELEASE_METADATA_BASE` override for staging and disaster recovery.

## 2. Add the public website routes

The website/CDN must serve these unauthenticated HTTPS endpoints:

| Endpoint | Response |
|---|---|
| `GET /install.sh` | Exact raw contents of `scripts/install.sh` |
| `GET /install.ps1` | Exact raw contents of `scripts/install.ps1` |
| `GET /releases/latest/version` | Latest stable tag followed by a newline |
| `GET /releases/<tag>/download-bases` | One trusted HTTPS download base per line |
| `GET /releases/<tag>/SHA256SUMS` | Checksum manifest from that GitHub release |

Example metadata responses for a hypothetical `v0.1.0` release:

`GET /releases/latest/version`:

```text
v0.1.0
```

`GET /releases/v0.1.0/download-bases`:

```text
https://github.com/Daanio-CLI/daanio-cli/releases/download/v0.1.0
```

The installer routes must:

- Return `200 OK` and script text, not an HTML page.
- Use `Content-Type: text/plain; charset=utf-8`.
- Use `X-Content-Type-Options: nosniff`.
- Never require cookies, login, JavaScript, or a browser challenge.
- Never include API keys, signing credentials, or other secrets.
- Use a short cache lifetime for `/install.sh`, `/install.ps1`, and
  `/releases/latest/version` so emergency fixes propagate quickly.
- Keep tagged release metadata immutable.

The website may proxy these files from object storage or copy them during a
deployment. Copying is usually more reliable than requesting GitHub on every
installation.

## 3. Configure the release workflow

`.github/workflows/release.yml` already builds these assets:

```text
daanio-macos-aarch64.tar.gz
daanio-macos-x86_64.tar.gz
daanio-windows-x86_64.exe
daanio-windows-x86_64.tar.gz
daanio-windows-aarch64.exe
daanio-windows-aarch64.tar.gz
daanio-linux-x86_64.tar.gz
daanio-linux-aarch64.tar.gz
SHA256SUMS
```

Configure repository Actions permissions so the workflow can create releases
and upload assets. If the build still needs private Git dependencies or
submodules, add a read-only `DEPLOY_KEY` secret. Otherwise remove that
requirement from the workflow rather than storing an unnecessary key.

Set release-build environment values to the official repository:

```text
DAANIO_GITHUB_REPOSITORY=<owner>/<repository>
DAANIO_REPO_URL=https://github.com/<owner>/<repository>.git
```

Do not point automatic updates or self-development cloning at the upstream
jcode repository. Doing so could replace Daanio with an upstream binary.

## 4. Configure Windows signing

Official Windows binaries should be Authenticode-signed. The release workflow
already supports Azure Artifact Signing with GitHub OIDC.

Create an Azure Artifact Signing account and public-trust certificate profile,
then configure these GitHub Actions secrets:

```text
AZURE_CLIENT_ID
AZURE_TENANT_ID
AZURE_SUBSCRIPTION_ID
```

Configure these GitHub Actions variables:

```text
WINDOWS_SIGNING_ENDPOINT
WINDOWS_SIGNING_ACCOUNT
WINDOWS_SIGNING_CERTIFICATE_PROFILE
WINDOWS_SIGNING_REQUIRED=true
```

Grant the GitHub OIDC identity the **Artifact Signing Certificate Profile
Signer** role. Keep signing required for official releases. An unsigned build
may be useful for internal testing, but it should not be advertised as the
public Windows installer.

Verify a downloaded release on Windows:

```powershell
Get-AuthenticodeSignature .\daanio-windows-x86_64.exe |
  Format-List Status,StatusMessage,SignerCertificate
```

The status must be `Valid`.

## 5. Configure macOS signing and notarization

The current workflow builds macOS binaries but does not yet implement the full
Apple signing/notarization path. Before a public macOS launch:

1. Enroll the Daanio organization in the Apple Developer Program.
2. Create a Developer ID Application certificate.
3. Store the certificate and its password as encrypted GitHub Actions secrets.
4. Sign both the Apple Silicon and Intel binaries with hardened runtime and a
   trusted timestamp.
5. Submit the distributable package to Apple's notary service.
6. Verify the signature and notarization during the release workflow.

Suggested secret names for a future workflow implementation are:

```text
APPLE_CERTIFICATE_P12_BASE64
APPLE_CERTIFICATE_PASSWORD
APPLE_SIGNING_IDENTITY
APPLE_TEAM_ID
APPLE_ID
APPLE_APP_PASSWORD
```

These names are recommendations; the current workflow does not consume them
until signing steps are added.

Minimum verification commands on macOS:

```bash
codesign --verify --deep --strict --verbose=2 ./daanio
codesign -dv --verbose=4 ./daanio
spctl --assess --type execute --verbose=4 ./daanio
```

For the strongest Gatekeeper experience, distribute the signed CLI inside a
notarized package and have the bootstrap installer extract/install that package.

## 6. Prepare a release

Use semantic versions such as `v0.1.0`.

1. Update the workspace version when cutting a real release.
2. Update release notes and the changelog.
3. Run formatting and tests.
4. Build release binaries locally where practical.
5. Confirm `LICENSE` and `NOTICE.md` remain present and are referenced or
   bundled with the commercial distribution.
6. Commit the release changes.
7. Create and push the version tag to the official repository.

Example tag commands, only after the release commit is reviewed:

```bash
git tag -a v0.1.0 -m "Daanio CLI v0.1.0"
git push origin v0.1.0
```

Pushing the tag starts `.github/workflows/release.yml`. The workflow initially
creates a draft, builds every platform, uploads checksums, and publishes only
after the required assets are available.

## 7. Verify the GitHub release

Before updating `/releases/latest/version`, verify the release contains the
expected files:

```bash
gh release view v0.1.0 --repo Daanio-CLI/daanio-cli
```

Download and verify the checksum manifest:

```bash
gh release download v0.1.0 \
  --repo Daanio-CLI/daanio-cli \
  --pattern SHA256SUMS \
  --pattern 'daanio-*'
sha256sum --check SHA256SUMS
```

On macOS, use `shasum -a 256` if `sha256sum` is unavailable.

Confirm that:

- Every advertised platform asset exists.
- `SHA256SUMS` contains each downloadable asset exactly once.
- Windows signatures are valid.
- macOS signatures/notarization are valid.
- `daanio --version` matches the release tag.
- The binary uses `https://api.daanio.com/v1` and Daanio browser login.

## 8. Publish release metadata

After the GitHub release passes verification:

1. Upload its `SHA256SUMS` to
   `/releases/<tag>/SHA256SUMS`.
2. Publish `/releases/<tag>/download-bases`.
3. Update `/releases/latest/version` last.

Updating the `latest` pointer last prevents installers from seeing a version
before its binaries and checksums are ready.

Do not modify assets or checksum files for an existing tag. If a release is
wrong, create a new patch release such as `v0.1.1`.

## 9. Test the public macOS installer

Test both Apple Silicon and Intel hardware or CI runners.

```bash
curl -fsSL https://daanio.com/install.sh -o /tmp/daanio-install.sh
bash -n /tmp/daanio-install.sh
bash /tmp/daanio-install.sh
```

Open a new terminal, then verify:

```bash
command -v daanio
daanio --version
daanio login
daanio auth-test --provider daanio
daanio
```

The installer should place the launcher at `~/.local/bin/daanio`, configure a
future-shell `PATH`, verify SHA-256 before installation, and preserve/reload an
already-running shared server during updates.

## 10. Test the public Windows installer

Test Windows 11 x64 and Windows 11 ARM64 in PowerShell 5.1 and current
PowerShell where possible.

Review and run the downloaded script:

```powershell
$scriptText = irm https://daanio.com/install.ps1
$scriptText
& ([scriptblock]::Create($scriptText))
```

Open a new PowerShell window, then verify:

```powershell
Get-Command daanio
daanio --version
Get-AuthenticodeSignature (Get-Command daanio).Source
daanio login
daanio auth-test --provider daanio
daanio
```

The installer should select the correct x64 or ARM64 asset, verify SHA-256,
install under `%LOCALAPPDATA%\daanio`, and add its `bin` directory to the user
`PATH` without requiring administrator privileges.

## 11. Add the commands to Daanio documentation

Add an **Install Daanio CLI** section to the getting-started page.

macOS and Linux:

```bash
curl -fsSL https://daanio.com/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://daanio.com/install.ps1 | iex
```

Then show:

```bash
daanio login
daanio
```

Also provide a safer review-first alternative that downloads the script before
executing it. Never ask users to paste an upstream OpenAI, Anthropic, Google,
OpenRouter, or other provider API key into the CLI.

## 12. Rollback procedure

If a newly published installer or binary is broken:

1. Stop advertising the affected tag.
2. Point `/releases/latest/version` back to the last verified tag.
3. Keep the broken tag immutable for incident analysis or mark its GitHub
   release as a prerelease/draft.
4. Fix the issue and publish a new patch version.
5. Re-run clean-install and update tests before moving `latest` forward again.

Never silently replace a binary under an existing version tag because its
published checksum and signature are part of the release's trust record.

## Final production checklist

- [ ] Official Daanio GitHub repository selected.
- [ ] All inherited release URLs replaced.
- [ ] `https://daanio.com/install.sh` returns the raw shell installer.
- [ ] `https://daanio.com/install.ps1` returns the raw PowerShell installer.
- [ ] Release metadata endpoints are deployed without authentication.
- [ ] macOS Apple Silicon and Intel assets are present.
- [ ] Windows x64 and ARM64 assets are present.
- [ ] Linux x64 and ARM64 assets are present.
- [ ] `SHA256SUMS` verifies every asset.
- [ ] Windows binaries have valid Authenticode signatures.
- [ ] macOS binaries/package are signed and notarized.
- [ ] `LICENSE` and `NOTICE.md` are retained for distribution.
- [ ] Clean install tested on macOS Apple Silicon.
- [ ] Clean install tested on macOS Intel.
- [ ] Clean install tested on Windows x64.
- [ ] Clean install tested on Windows ARM64.
- [ ] Browser login and `auth-test` pass after clean installation.
- [ ] Update/reinstall preserves credentials and reloads the shared server.
- [ ] Rollback procedure has been tested.

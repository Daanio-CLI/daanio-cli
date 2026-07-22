# Manual Windows signing with Google Cloud KMS

Use this procedure when the public GitHub repository must have no permission to
use the LPB Trading Corporation signing key. GitHub Actions compiles unsigned
Windows binaries and stores them as non-release workflow artifacts. An authorized
operator downloads them into a private Google Cloud Shell session, signs them
with the non-exportable Cloud HSM key, and uploads only the signed files.

The KMS private key never leaves Google Cloud. The short-lived Google access
token is used only inside the operator's Cloud Shell and must never be added to
GitHub Secrets, command output, release assets, or repository files.

## Prerequisites

- An issued Authenticode certificate chain whose leaf certificate has the
  `Code Signing` extended-key usage and matches the KMS public key.
- `gcloud`, `gh`, Java, and [Jsign](https://github.com/ebourg/jsign) installed
  in the private signing environment.
- `gh auth status` reports an account allowed to update
  `Daanio-CLI/daanio-cli` releases.
- The Google identity in Cloud Shell has
  `cloudkms.cryptoKeyVersions.useToSign` on the signing key.

Do not use the Cavium/Google HSM attestation chain as `--certfile`. It proves
hardware custody but is not the issued Authenticode certificate.

## 1. Select the release and download unsigned artifacts

Find the successful release workflow run for the tag, then download both
unsigned workflow artifacts:

```bash
export DAANIO_RELEASE_TAG='v0.1.0-daanio.2'
export DAANIO_RELEASE_RUN_ID='REPLACE_WITH_RUN_ID'
mkdir -p "$HOME/daanio-windows-signing/unsigned"
cd "$HOME/daanio-windows-signing"
gh run download "$DAANIO_RELEASE_RUN_ID" --repo Daanio-CLI/daanio-cli --name windows-unsigned-x86_64 --dir unsigned/x86_64
gh run download "$DAANIO_RELEASE_RUN_ID" --repo Daanio-CLI/daanio-cli --name windows-unsigned-aarch64 --dir unsigned/aarch64
```

The expected inputs are:

```text
unsigned/x86_64/daanio-windows-x86_64.exe
unsigned/aarch64/daanio-windows-aarch64.exe
```

## 2. Verify the issued certificate

Set the path to the full issued certificate chain, not the attestation chain:

```bash
export DAANIO_WINDOWS_CERT="$HOME/issued-windows-code-signing-chain.pem"
openssl x509 -in "$DAANIO_WINDOWS_CERT" -noout -subject -issuer -dates -ext extendedKeyUsage
```

The leaf subject must identify LPB Trading Corporation and extended key usage
must include `Code Signing`.

## 3. Sign with the non-exportable KMS key

Jsign supports Google Cloud KMS directly. Keep the access token in an
environment variable so it is not visible in the command arguments:

```bash
export DAANIO_GCP_KEYRING='projects/PROJECT_ID/locations/global/keyRings/KEY_RING'
export DAANIO_GCP_KEY_ALIAS='KEY_NAME/cryptoKeyVersions/1'
export DAANIO_GCP_ACCESS_TOKEN="$(gcloud auth print-access-token)"
mkdir -p signed
cp unsigned/x86_64/daanio-windows-x86_64.exe signed/
cp unsigned/aarch64/daanio-windows-aarch64.exe signed/
```

Sign each executable with SHA-256 and an RFC 3161 timestamp:

```bash
jsign --storetype GOOGLECLOUD --keystore "$DAANIO_GCP_KEYRING" --storepass env:DAANIO_GCP_ACCESS_TOKEN --alias "$DAANIO_GCP_KEY_ALIAS" --certfile "$DAANIO_WINDOWS_CERT" --alg SHA-256 --tsmode RFC3161 --tsaurl 'http://timestamp.digicert.com,http://timestamp.sectigo.com' --name 'Daanio CLI' --url 'https://daanio.com' signed/daanio-windows-x86_64.exe
jsign --storetype GOOGLECLOUD --keystore "$DAANIO_GCP_KEYRING" --storepass env:DAANIO_GCP_ACCESS_TOKEN --alias "$DAANIO_GCP_KEY_ALIAS" --certfile "$DAANIO_WINDOWS_CERT" --alg SHA-256 --tsmode RFC3161 --tsaurl 'http://timestamp.digicert.com,http://timestamp.sectigo.com' --name 'Daanio CLI' --url 'https://daanio.com' signed/daanio-windows-aarch64.exe
unset DAANIO_GCP_ACCESS_TOKEN
```

Verify both signatures before publishing:

```bash
jsign --verify --verbose signed/daanio-windows-x86_64.exe
jsign --verify --verbose signed/daanio-windows-aarch64.exe
```

Also verify at least the x86_64 file on Windows before public distribution:

```powershell
Get-AuthenticodeSignature .\daanio-windows-x86_64.exe |
  Format-List Status,StatusMessage,SignerCertificate
```

`Status` must be `Valid`.

## 4. Package, checksum, and publish only signed files

```bash
cd "$HOME/daanio-windows-signing/signed"
tar -czf daanio-windows-x86_64.tar.gz daanio-windows-x86_64.exe
tar -czf daanio-windows-aarch64.tar.gz daanio-windows-aarch64.exe
gh release download "$DAANIO_RELEASE_TAG" --repo Daanio-CLI/daanio-cli --pattern SHA256SUMS --clobber
grep -v '  daanio-windows-' SHA256SUMS > SHA256SUMS.new
sha256sum daanio-windows-x86_64.exe daanio-windows-x86_64.tar.gz daanio-windows-aarch64.exe daanio-windows-aarch64.tar.gz >> SHA256SUMS.new
sort -k2 SHA256SUMS.new > SHA256SUMS
gh release upload "$DAANIO_RELEASE_TAG" --repo Daanio-CLI/daanio-cli daanio-windows-x86_64.exe daanio-windows-x86_64.tar.gz daanio-windows-aarch64.exe daanio-windows-aarch64.tar.gz SHA256SUMS --clobber
```

Finally, download the public assets on Windows, verify `SHA256SUMS`, and run
`Get-AuthenticodeSignature` again. Delete the Cloud Shell working directory
after verification if its persisted contents are no longer needed.

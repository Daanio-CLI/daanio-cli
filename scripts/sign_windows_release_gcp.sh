#!/usr/bin/env bash
set -euo pipefail

REPO="${DAANIO_SIGNING_REPO:-Daanio-CLI/daanio-cli}"
GCP_PROJECT="${DAANIO_GCP_PROJECT:-codesigning-491020}"
GCP_LOCATION="${DAANIO_GCP_LOCATION:-global}"
GCP_KEYRING="${DAANIO_GCP_KEYRING_NAME:-codesigning-keyring}"
GCP_KEY="${DAANIO_GCP_KEY_NAME:-codesigning-key}"
GCP_KEY_VERSION="${DAANIO_GCP_KEY_VERSION:-1}"
JSIGN_VERSION="7.5"
JSIGN_SHA256="602a51c3545a6dc4fb99bd2ea7152b26d1345916d0c93ddfbd5936cb735af91c"

release_tag=""
release_run_id=""
certificate_file="${DAANIO_WINDOWS_CERT:-$HOME/issued-windows-code-signing-chain.pem}"
publish=false
keep_workdir=false
workdir=""
workdir_owned=false

info() { printf '\033[1;34m%s\033[0m\n' "$*"; }
success() { printf '\033[1;32m%s\033[0m\n' "$*"; }
err() { printf '\033[1;31merror: %s\033[0m\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Sign Daanio Windows release binaries with the private Google Cloud HSM key.

Usage:
  scripts/sign_windows_release_gcp.sh --tag TAG [options]

Required:
  --tag TAG                 Release tag, for example v0.1.0-daanio.2

Options:
  --certificate PATH        Issued Authenticode PEM chain. The default is
                            $DAANIO_WINDOWS_CERT or
                            ~/issued-windows-code-signing-chain.pem.
  --run-id ID               Release workflow run containing unsigned artifacts.
                            By default the successful run for TAG is discovered.
  --publish                 Upload signed assets and SHA256SUMS to the release.
                            Without this flag, files are only prepared locally.
  --keep-workdir            Keep temporary inputs and outputs after publishing.
  -h, --help                Show this help.

Google KMS defaults:
  project:     codesigning-491020
  location:    global
  key ring:    codesigning-keyring
  key:         codesigning-key
  key version: 1

Override a default with DAANIO_GCP_PROJECT, DAANIO_GCP_LOCATION,
DAANIO_GCP_KEYRING_NAME, DAANIO_GCP_KEY_NAME, or DAANIO_GCP_KEY_VERSION.

The private key and Google access token are never uploaded to GitHub. Only the
signed executables, archives, and their checksum manifest are published.
EOF
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || err "required command not found: $1"
}

sha256_value() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print tolower($1)}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print tolower($1)}'
  else
    openssl dgst -sha256 "$1" | awk '{print tolower($NF)}'
  fi
}

append_sha256() {
  file="$1"
  printf '%s  %s\n' "$(sha256_value "$file")" "$(basename "$file")"
}

cleanup() {
  status=$?
  unset DAANIO_GCP_ACCESS_TOKEN || true

  if [ "$status" -eq 0 ] && [ "$publish" = true ] && [ "$keep_workdir" = false ] && [ "$workdir_owned" = true ]; then
    rm -rf -- "$workdir"
  elif [ -n "$workdir" ] && [ -d "$workdir" ]; then
    printf 'Signing work directory: %s\n' "$workdir" >&2
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

while [ "$#" -gt 0 ]; do
  case "$1" in
    --tag)
      [ "$#" -ge 2 ] || err "--tag requires a value"
      release_tag="$2"
      shift 2
      ;;
    --certificate)
      [ "$#" -ge 2 ] || err "--certificate requires a path"
      certificate_file="$2"
      shift 2
      ;;
    --run-id)
      [ "$#" -ge 2 ] || err "--run-id requires a value"
      release_run_id="$2"
      shift 2
      ;;
    --publish)
      publish=true
      shift
      ;;
    --keep-workdir)
      keep_workdir=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      err "unknown option: $1"
      ;;
  esac
done

[ -n "$release_tag" ] || err "--tag is required"
printf '%s' "$release_tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+([+.-][[:alnum:].-]+)?$' ||
  err "invalid release tag: $release_tag"
[ -z "$release_run_id" ] || printf '%s' "$release_run_id" | grep -Eq '^[0-9]+$' ||
  err "run ID must contain only digits"

require_command curl
require_command gcloud
require_command gh
require_command java
require_command openssl
require_command tar

[ -f "$certificate_file" ] || err "issued Authenticode certificate not found: $certificate_file"
gh auth status >/dev/null 2>&1 || err "GitHub CLI is not authenticated; run: gh auth login -h github.com -w"
active_gcp_account="$(gcloud auth list --filter=status:ACTIVE --format='value(account)' | head -n 1)"
[ -n "$active_gcp_account" ] || err "gcloud has no active account"
info "Google account: $active_gcp_account"

if [ -z "$release_run_id" ]; then
  info "Finding the successful Release workflow for $release_tag..."
  release_run_id="$(
    gh api --method GET "/repos/$REPO/actions/workflows/release.yml/runs" \
      -f branch="$release_tag" -f per_page=20 \
      --jq '.workflow_runs | map(select(.status == "completed" and .conclusion == "success")) | first | .id // empty'
  )"
  [ -n "$release_run_id" ] || err "no successful Release workflow found for $release_tag"
fi

run_status="$(gh run view "$release_run_id" --repo "$REPO" --json status,conclusion,headBranch \
  --jq '[.status, .conclusion, .headBranch] | @tsv')"
IFS=$'\t' read -r workflow_status workflow_conclusion workflow_ref <<<"$run_status"
[ "$workflow_status" = "completed" ] || err "Release workflow $release_run_id is not complete"
[ "$workflow_conclusion" = "success" ] || err "Release workflow $release_run_id did not succeed"
[ "$workflow_ref" = "$release_tag" ] || err "workflow ref $workflow_ref does not match $release_tag"
gh release view "$release_tag" --repo "$REPO" >/dev/null

workdir="$(mktemp -d "${TMPDIR:-/tmp}/daanio-windows-signing.XXXXXX")"
workdir_owned=true
mkdir -p "$workdir/unsigned/x86_64" "$workdir/unsigned/aarch64" "$workdir/signed"

info "Downloading unsigned Windows artifacts from workflow $release_run_id..."
gh run download "$release_run_id" --repo "$REPO" \
  --name windows-unsigned-x86_64 --dir "$workdir/unsigned/x86_64"
gh run download "$release_run_id" --repo "$REPO" \
  --name windows-unsigned-aarch64 --dir "$workdir/unsigned/aarch64"

x64_input="$workdir/unsigned/x86_64/daanio-windows-x86_64.exe"
arm64_input="$workdir/unsigned/aarch64/daanio-windows-aarch64.exe"
[ -f "$x64_input" ] || err "x86_64 unsigned executable is missing from the workflow artifact"
[ -f "$arm64_input" ] || err "ARM64 unsigned executable is missing from the workflow artifact"

info "Checking the Google Cloud HSM key..."
kms_details="$(gcloud kms keys versions describe "$GCP_KEY_VERSION" \
  --project="$GCP_PROJECT" --location="$GCP_LOCATION" --keyring="$GCP_KEYRING" --key="$GCP_KEY" \
  --format='value(state,algorithm,protectionLevel)')"
IFS=$'\t' read -r kms_state kms_algorithm kms_protection <<<"$kms_details"
[ "$kms_state" = "ENABLED" ] || err "KMS key version is not enabled: $kms_state"
[ "$kms_algorithm" = "RSA_SIGN_PKCS1_4096_SHA256" ] || err "unexpected KMS algorithm: $kms_algorithm"
[ "$kms_protection" = "HSM" ] || err "KMS key is not HSM-protected: $kms_protection"

kms_public_key="$workdir/kms-public-key.pem"
gcloud kms keys versions get-public-key "$GCP_KEY_VERSION" \
  --project="$GCP_PROJECT" --location="$GCP_LOCATION" --keyring="$GCP_KEYRING" --key="$GCP_KEY" \
  --output-file="$kms_public_key" >/dev/null

certificate_details="$(openssl x509 -in "$certificate_file" -noout -subject -issuer -dates -ext extendedKeyUsage)"
printf '%s\n' "$certificate_details"
printf '%s\n' "$certificate_details" | grep -Eqi 'Code Signing|codeSigning' ||
  err "certificate does not contain the Code Signing extended-key usage"

certificate_public_key="$workdir/certificate-public-key.der"
kms_public_key_der="$workdir/kms-public-key.der"
openssl x509 -in "$certificate_file" -pubkey -noout |
  openssl pkey -pubin -outform DER -out "$certificate_public_key"
openssl pkey -pubin -in "$kms_public_key" -outform DER -out "$kms_public_key_der"
certificate_fingerprint="$(sha256_value "$certificate_public_key")"
kms_fingerprint="$(sha256_value "$kms_public_key_der")"
[ "$certificate_fingerprint" = "$kms_fingerprint" ] ||
  err "certificate public key does not match the Google Cloud HSM key"
success "Certificate matches the HSM key: $certificate_fingerprint"

jsign_cache_dir="${DAANIO_SIGNING_TOOL_DIR:-$HOME/.local/share/daanio-signing}"
jsign_jar="$jsign_cache_dir/jsign-$JSIGN_VERSION.jar"
if command -v jsign >/dev/null 2>&1; then
  jsign_command=(jsign)
else
  mkdir -p "$jsign_cache_dir"
  chmod 700 "$jsign_cache_dir"
  if [ ! -f "$jsign_jar" ] || [ "$(sha256_value "$jsign_jar")" != "$JSIGN_SHA256" ]; then
    info "Installing pinned Jsign $JSIGN_VERSION in $jsign_cache_dir..."
    jsign_download="$workdir/jsign-$JSIGN_VERSION.jar"
    curl -fL --retry 3 \
      "https://github.com/ebourg/jsign/releases/download/$JSIGN_VERSION/jsign-$JSIGN_VERSION.jar" \
      -o "$jsign_download"
    [ "$(sha256_value "$jsign_download")" = "$JSIGN_SHA256" ] || err "Jsign checksum verification failed"
    install -m 600 "$jsign_download" "$jsign_jar"
  fi
  jsign_command=(java -jar "$jsign_jar")
fi

x64_signed="$workdir/signed/daanio-windows-x86_64.exe"
arm64_signed="$workdir/signed/daanio-windows-aarch64.exe"
cp "$x64_input" "$x64_signed"
cp "$arm64_input" "$arm64_signed"

export DAANIO_GCP_ACCESS_TOKEN="$(gcloud auth print-access-token)"
[ -n "$DAANIO_GCP_ACCESS_TOKEN" ] || err "could not obtain a Google Cloud access token"
gcp_keystore="projects/$GCP_PROJECT/locations/$GCP_LOCATION/keyRings/$GCP_KEYRING"
gcp_alias="$GCP_KEY/cryptoKeyVersions/$GCP_KEY_VERSION"

sign_file() {
  target="$1"
  "${jsign_command[@]}" \
    --storetype GOOGLECLOUD --keystore "$gcp_keystore" \
    --storepass env:DAANIO_GCP_ACCESS_TOKEN --alias "$gcp_alias" \
    --certfile "$certificate_file" --alg SHA-256 --tsmode RFC3161 \
    --tsaurl 'http://timestamp.digicert.com,http://timestamp.sectigo.com' \
    --name 'Daanio CLI' --url 'https://daanio.com' "$target"

  # Jsign 7.5 no longer exposes the historical --verify option. Extracting the
  # signature through Jsign validates that the PE signature is readable. Then
  # parse the PKCS#7 payload, require the exact configured signer public key,
  # and require an embedded RFC 3161 timestamp token. A final native
  # Get-AuthenticodeSignature check is still required on Windows.
  signature_file="$target.sig"
  signature_certificates="$target.sig.certificates.pem"
  signature_certificate_dir="$target.sig.certificates"
  signature_public_key="$target.sig.public-key.der"
  signature_structure="$target.sig.asn1.txt"
  rm -f "$signature_file" "$signature_certificates" "$signature_public_key" "$signature_structure"
  mkdir -p "$signature_certificate_dir"
  "${jsign_command[@]}" extract --format DER "$target"
  [ -s "$signature_file" ] || err "Jsign did not extract an embedded signature from $target"
  openssl pkcs7 -inform DER -in "$signature_file" -print_certs -out "$signature_certificates"
  awk -v output_dir="$signature_certificate_dir" '
    /-----BEGIN CERTIFICATE-----/ {
      certificate_count++
      output_file = sprintf("%s/certificate-%d.pem", output_dir, certificate_count)
    }
    output_file != "" { print > output_file }
    /-----END CERTIFICATE-----/ {
      close(output_file)
      output_file = ""
    }
  ' "$signature_certificates"

  embedded_certificate_matches=false
  for embedded_certificate in "$signature_certificate_dir"/certificate-*.pem; do
    [ -f "$embedded_certificate" ] || continue
    openssl x509 -in "$embedded_certificate" -pubkey -noout |
      openssl pkey -pubin -outform DER -out "$signature_public_key"
    embedded_fingerprint="$(sha256_value "$signature_public_key")"
    if [ "$embedded_fingerprint" = "$certificate_fingerprint" ]; then
      embedded_certificate_matches=true
      break
    fi
  done
  [ "$embedded_certificate_matches" = true ] ||
    err "no embedded signer certificate matches the configured DigiCert certificate"
  openssl asn1parse -inform DER -in "$signature_file" -i > "$signature_structure"
  grep -Fq 'id-smime-ct-TSTInfo' "$signature_structure" || err "signed file has no RFC 3161 timestamp token"
  rm -rf -- "$signature_certificate_dir"
  rm -f "$signature_file" "$signature_certificates" "$signature_public_key" "$signature_structure"
  success "Embedded Authenticode signer and RFC 3161 timestamp validated: $(basename "$target")"
}

info "Signing Windows x86_64 with Google Cloud HSM..."
sign_file "$x64_signed"
info "Signing Windows ARM64 with Google Cloud HSM..."
sign_file "$arm64_signed"
unset DAANIO_GCP_ACCESS_TOKEN

tar -czf "$workdir/signed/daanio-windows-x86_64.tar.gz" -C "$workdir/signed" daanio-windows-x86_64.exe
tar -czf "$workdir/signed/daanio-windows-aarch64.tar.gz" -C "$workdir/signed" daanio-windows-aarch64.exe

info "Updating the release checksum manifest..."
gh release download "$release_tag" --repo "$REPO" --pattern SHA256SUMS --dir "$workdir/signed" --clobber
awk '$2 !~ /^daanio-windows-/' "$workdir/signed/SHA256SUMS" > "$workdir/signed/SHA256SUMS.new"
append_sha256 "$x64_signed" >> "$workdir/signed/SHA256SUMS.new"
append_sha256 "$workdir/signed/daanio-windows-x86_64.tar.gz" >> "$workdir/signed/SHA256SUMS.new"
append_sha256 "$arm64_signed" >> "$workdir/signed/SHA256SUMS.new"
append_sha256 "$workdir/signed/daanio-windows-aarch64.tar.gz" >> "$workdir/signed/SHA256SUMS.new"
sort -k2 "$workdir/signed/SHA256SUMS.new" > "$workdir/signed/SHA256SUMS"
rm -f "$workdir/signed/SHA256SUMS.new"

if [ "$publish" = true ]; then
  info "Publishing signed Windows assets to $REPO release $release_tag..."
  gh release upload "$release_tag" --repo "$REPO" \
    "$x64_signed" "$workdir/signed/daanio-windows-x86_64.tar.gz" \
    "$arm64_signed" "$workdir/signed/daanio-windows-aarch64.tar.gz" \
    "$workdir/signed/SHA256SUMS" --clobber
  success "Signed Windows assets published successfully."
else
  success "Signed Windows assets are ready in $workdir/signed"
  info "Verify them on Windows, then rerun with --publish when ready."
fi

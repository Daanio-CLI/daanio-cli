#!/usr/bin/env bash
set -euo pipefail

GCP_PROJECT="${DAANIO_GCP_PROJECT:-codesigning-491020}"
GCP_LOCATION="${DAANIO_GCP_LOCATION:-global}"
GCP_KEYRING="${DAANIO_GCP_KEYRING_NAME:-codesigning-keyring}"
GCP_KEY="${DAANIO_GCP_KEY_NAME:-codesigning-key}"
GCP_KEY_VERSION="${DAANIO_GCP_KEY_VERSION:-1}"

source_dir=""
output_file="${DAANIO_WINDOWS_CERT:-$HOME/issued-windows-code-signing-chain.pem}"
force=false
workdir=""

info() { printf '\033[1;34m%s\033[0m\n' "$*"; }
success() { printf '\033[1;32m%s\033[0m\n' "$*"; }
err() { printf '\033[1;31merror: %s\033[0m\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Prepare and verify the DigiCert certificate chain used to sign Daanio for Windows.

Usage:
  scripts/setup_windows_signing_certificate.sh [options]

Options:
  --source DIR    Directory containing lpb_trading_corporation.crt,
                  DigiCertCA.crt, and TrustedRoot.crt. If omitted, the script
                  discovers a single ~/lpb_trading_corporation_* directory.
  --output FILE   Final PEM signing chain. The default is
                  ~/issued-windows-code-signing-chain.pem.
  --force         Replace an existing different output file.
  -h, --help      Show this help.

The script verifies the DigiCert chain, Code Signing usage, LPB subject, HSM
state, and certificate-to-KMS public-key match. The trusted root is used for
verification but is intentionally excluded from the final signing chain.
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

convert_certificate() {
  input="$1"
  output="$2"

  if openssl x509 -in "$input" -noout >/dev/null 2>&1; then
    openssl x509 -in "$input" -out "$output"
  elif openssl x509 -inform DER -in "$input" -noout >/dev/null 2>&1; then
    openssl x509 -inform DER -in "$input" -out "$output"
  else
    err "not a readable PEM or DER X.509 certificate: $input"
  fi
}

cleanup() {
  status=$?
  if [ -n "$workdir" ] && [ -d "$workdir" ]; then
    rm -rf -- "$workdir"
  fi
  return "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

while [ "$#" -gt 0 ]; do
  case "$1" in
    --source)
      [ "$#" -ge 2 ] || err "--source requires a directory"
      source_dir="$2"
      shift 2
      ;;
    --output)
      [ "$#" -ge 2 ] || err "--output requires a file"
      output_file="$2"
      shift 2
      ;;
    --force)
      force=true
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

require_command find
require_command gcloud
require_command install
require_command openssl

if [ -z "$source_dir" ]; then
  candidates=()
  while IFS= read -r -d '' candidate; do
    candidates+=("$candidate")
  done < <(find "$HOME" -maxdepth 1 -type d -name 'lpb_trading_corporation_*' -print0)

  case "${#candidates[@]}" in
    0) err "no DigiCert directory found; pass it with --source" ;;
    1) source_dir="${candidates[0]}" ;;
    *)
      printf 'Multiple DigiCert directories found:\n' >&2
      printf '  %s\n' "${candidates[@]}" >&2
      err "select one with --source"
      ;;
  esac
fi

[ -d "$source_dir" ] || err "DigiCert source directory not found: $source_dir"
leaf_input="$source_dir/lpb_trading_corporation.crt"
intermediate_input="$source_dir/DigiCertCA.crt"
root_input="$source_dir/TrustedRoot.crt"
[ -f "$leaf_input" ] || err "missing DigiCert leaf certificate: $leaf_input"
[ -f "$intermediate_input" ] || err "missing DigiCert intermediate certificate: $intermediate_input"
[ -f "$root_input" ] || err "missing DigiCert trusted root certificate: $root_input"

workdir="$(mktemp -d "${TMPDIR:-/tmp}/daanio-certificate-setup.XXXXXX")"
leaf_pem="$workdir/lpb-leaf.pem"
intermediate_pem="$workdir/digicert-intermediate.pem"
root_pem="$workdir/digicert-root.pem"
final_chain="$workdir/issued-windows-code-signing-chain.pem"

info "Converting the DigiCert certificates to PEM..."
convert_certificate "$leaf_input" "$leaf_pem"
convert_certificate "$intermediate_input" "$intermediate_pem"
convert_certificate "$root_input" "$root_pem"

info "Verifying the DigiCert certificate chain..."
openssl verify -CAfile "$root_pem" -untrusted "$intermediate_pem" "$leaf_pem"
openssl x509 -in "$leaf_pem" -checkend 0 -noout >/dev/null || err "DigiCert certificate is expired"

certificate_details="$(openssl x509 -in "$leaf_pem" -noout -subject -issuer -serial -dates -ext extendedKeyUsage)"
printf '%s\n' "$certificate_details"
printf '%s\n' "$certificate_details" | grep -Fqi 'LPB Trading Corporation' ||
  err "leaf subject does not identify LPB Trading Corporation"
printf '%s\n' "$certificate_details" | grep -Eqi 'Code Signing|codeSigning' ||
  err "leaf certificate does not contain the Code Signing extended-key usage"

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

certificate_public_key="$workdir/certificate-public-key.der"
kms_public_key_der="$workdir/kms-public-key.der"
openssl x509 -in "$leaf_pem" -pubkey -noout |
  openssl pkey -pubin -outform DER -out "$certificate_public_key"
openssl pkey -pubin -in "$kms_public_key" -outform DER -out "$kms_public_key_der"
certificate_fingerprint="$(sha256_value "$certificate_public_key")"
kms_fingerprint="$(sha256_value "$kms_public_key_der")"
[ "$certificate_fingerprint" = "$kms_fingerprint" ] ||
  err "DigiCert certificate public key does not match the Google Cloud HSM key"
success "Certificate matches the HSM key: $certificate_fingerprint"

cp "$leaf_pem" "$final_chain"
openssl x509 -in "$intermediate_pem" -outform PEM >> "$final_chain"

if [ -e "$output_file" ] && [ "$force" = false ]; then
  if cmp -s "$final_chain" "$output_file"; then
    success "Signing certificate is already configured: $output_file"
    exit 0
  fi
  err "output already exists and differs: $output_file (use --force to replace it)"
fi

mkdir -p "$(dirname "$output_file")"
install -m 600 "$final_chain" "$output_file"
success "Windows signing certificate is ready: $output_file"
printf '\nNext, sign a completed release with:\n'
printf '  scripts/sign_windows_release_gcp.sh --tag TAG --certificate %q --publish\n' "$output_file"

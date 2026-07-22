#!/usr/bin/env bash
# Update Homebrew tap and AUR package for a new release.
# Usage: scripts/update_packages.sh v0.1.3
set -euo pipefail

VERSION="${1:?Usage: $0 <version-tag>}"
VERSION_NUM="${VERSION#v}"

echo "Updating packages for $VERSION..."

LINUX_URL="https://github.com/Daanio-CLI/daanio-cli/releases/download/${VERSION}/daanio-linux-x86_64.tar.gz"
LINUX_ARM_URL="https://github.com/Daanio-CLI/daanio-cli/releases/download/${VERSION}/daanio-linux-aarch64.tar.gz"
MACOS_ARM_URL="https://github.com/Daanio-CLI/daanio-cli/releases/download/${VERSION}/daanio-macos-aarch64.tar.gz"
MACOS_INTEL_URL="https://github.com/Daanio-CLI/daanio-cli/releases/download/${VERSION}/daanio-macos-x86_64.tar.gz"

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

echo "Downloading assets for checksums..."
curl -sL "$LINUX_URL" -o "$tmpdir/linux.tar.gz"
curl -sL "$LINUX_ARM_URL" -o "$tmpdir/linux-arm.tar.gz"
curl -sL "$MACOS_ARM_URL" -o "$tmpdir/macos-arm.tar.gz"
curl -sL "$MACOS_INTEL_URL" -o "$tmpdir/macos-intel.tar.gz"

LINUX_SHA=$(sha256sum "$tmpdir/linux.tar.gz" | cut -d' ' -f1)
LINUX_ARM_SHA=$(sha256sum "$tmpdir/linux-arm.tar.gz" | cut -d' ' -f1)
MACOS_ARM_SHA=$(sha256sum "$tmpdir/macos-arm.tar.gz" | cut -d' ' -f1)
MACOS_INTEL_SHA=$(sha256sum "$tmpdir/macos-intel.tar.gz" | cut -d' ' -f1)

  echo "  Linux SHA256: $LINUX_SHA"
echo "  Linux ARM64 SHA256: $LINUX_ARM_SHA"
echo "  macOS ARM64 SHA256: $MACOS_ARM_SHA"
echo "  macOS Intel SHA256: $MACOS_INTEL_SHA"

# --- Homebrew tap ---
echo ""
echo "Updating Homebrew tap..."
BREW_DIR="$tmpdir/homebrew-daanio"
git clone --depth 1 git@github.com:1jehuang/homebrew-daanio.git "$BREW_DIR" 2>/dev/null

cat > "$BREW_DIR/Formula/daanio.rb" <<EOF
class Daanio < Formula
  desc "AI coding agent powered by Claude and ChatGPT"
  homepage "https://github.com/Daanio-CLI/daanio-cli"
  version "$VERSION_NUM"
  license "MIT"

  on_macos do
    on_arm do
      url "$MACOS_ARM_URL"
      sha256 "$MACOS_ARM_SHA"

      def install
        bin.install "daanio-macos-aarch64" => "daanio"
      end
    end

    on_intel do
      url "$MACOS_INTEL_URL"
      sha256 "$MACOS_INTEL_SHA"

      def install
        bin.install "daanio-macos-x86_64" => "daanio"
      end
    end
  end

  on_linux do
    on_intel do
      url "$LINUX_URL"
      sha256 "$LINUX_SHA"

      def install
        libexec.install "daanio-linux-x86_64", "daanio-linux-x86_64.bin"
        libexec.install Dir["libssl.so*"], Dir["libcrypto.so*"]
        (bin/"daanio").write <<~SH
          #!/bin/sh
          exec "#{libexec}/daanio-linux-x86_64" "\$@"
        SH
      end
    end

    on_arm do
      url "$LINUX_ARM_URL"
      sha256 "$LINUX_ARM_SHA"

      def install
        bin.install "daanio-linux-aarch64" => "daanio"
      end
    end
  end

  test do
    assert_match "daanio", shell_output("#{bin}/daanio --version")
  end
end
EOF

(cd "$BREW_DIR" && git add -A && git commit -m "Update daanio to $VERSION" && git push origin main)
echo "  ✅ Homebrew tap updated"

# --- AUR ---
echo ""
echo "Updating AUR package..."
AUR_DIR="$tmpdir/daanio-bin-aur"
git clone ssh://aur@aur.archlinux.org/daanio-bin.git "$AUR_DIR" 2>/dev/null

cat > "$AUR_DIR/PKGBUILD" <<EOF
# Maintainer: Jeremy Huang <jeremyhuang55555@gmail.com>
pkgname=daanio-bin
pkgver=$VERSION_NUM
pkgrel=1
pkgdesc="AI coding agent powered by Claude and ChatGPT"
arch=('x86_64')
url="https://github.com/Daanio-CLI/daanio-cli"
license=('MIT')
provides=('daanio')
conflicts=('daanio')
source=("$LINUX_URL")
sha256sums=('$LINUX_SHA')

package() {
    install -Dm755 "\${srcdir}/daanio-linux-x86_64" "\${pkgdir}/usr/lib/daanio/daanio-linux-x86_64"
    install -Dm755 "\${srcdir}/daanio-linux-x86_64.bin" "\${pkgdir}/usr/lib/daanio/daanio-linux-x86_64.bin"
    install -Dm644 "\${srcdir}"/libssl.so* "\${pkgdir}/usr/lib/daanio/"
    install -Dm644 "\${srcdir}"/libcrypto.so* "\${pkgdir}/usr/lib/daanio/"
    mkdir -p "\${pkgdir}/usr/bin"
    ln -s /usr/lib/daanio/daanio-linux-x86_64 "\${pkgdir}/usr/bin/daanio"
}
EOF

(cd "$AUR_DIR" && makepkg --printsrcinfo > .SRCINFO && git add -A && git commit -m "Update to $VERSION" && git push origin master)
echo "  ✅ AUR package updated"

echo ""
echo "Done! Packages updated to $VERSION"

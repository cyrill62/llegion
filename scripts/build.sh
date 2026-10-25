#!/usr/bin/env bash
# Release build + packaging for Linux, macOS and Windows (x86_64 and arm64).
#
# Usage: scripts/build.sh [options] [target...]
#
#   Targets (short names or full Rust triples):
#     linux-x64    x86_64-unknown-linux-gnu      linux-arm64    aarch64-unknown-linux-gnu
#     linux-musl-x64 x86_64-unknown-linux-musl   linux-musl-arm64 aarch64-unknown-linux-musl
#     macos-x64    x86_64-apple-darwin           macos-arm64    aarch64-apple-darwin
#     windows-x64  x86_64-pc-windows-msvc        windows-arm64  aarch64-pc-windows-msvc
#   Groups: linux, linux-musl, macos, windows, all.  Default: both architectures of the host OS.
#
#   Options:
#     --no-package   only build, do not create archives in dist/
#     -h, --help     show this help
#
# Cross-compilation tools (used only when the target differs from the host):
#   Linux arm64 <-> x64 : cargo-zigbuild (+ zig) or cross (+ Docker/Podman)
#   Windows from Linux/macOS : cargo-xwin (downloads the MSVC CRT/SDK, needs clang + lld)
#   macOS : must run on macOS (Xcode command line tools); not cross-compilable from Linux/Windows
#   musl : must run on a musl host of the same arch (e.g. the rust:alpine Docker image)
set -euo pipefail

cd "$(dirname "$0")/.."
APP=llegion
TUI=llegion-tui
APP_NAME="Llegion"
VERSION=$(tr -d '[:space:]' < VERSION)
DIST=dist
PACKAGE=1

die() { echo "error: $*" >&2; exit 1; }
info() { echo "==> $*"; }
have() { command -v "$1" >/dev/null 2>&1; }
# cargo-zigbuild also finds zig installed with `pip install ziglang`.
have_zig() { have zig || python3 -c 'import ziglang' >/dev/null 2>&1; }

usage() { sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0; }

resolve() {
  case "$1" in
    linux-x64) echo x86_64-unknown-linux-gnu ;;
    linux-arm64) echo aarch64-unknown-linux-gnu ;;
    linux-musl-x64) echo x86_64-unknown-linux-musl ;;
    linux-musl-arm64) echo aarch64-unknown-linux-musl ;;
    macos-x64) echo x86_64-apple-darwin ;;
    macos-arm64) echo aarch64-apple-darwin ;;
    windows-x64) echo x86_64-pc-windows-msvc ;;
    windows-arm64) echo aarch64-pc-windows-msvc ;;
    linux) echo x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu ;;
    linux-musl) echo x86_64-unknown-linux-musl aarch64-unknown-linux-musl ;;
    macos) echo x86_64-apple-darwin aarch64-apple-darwin ;;
    windows) echo x86_64-pc-windows-msvc aarch64-pc-windows-msvc ;;
    all) echo x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-unknown-linux-musl \
              aarch64-unknown-linux-musl x86_64-apple-darwin \
              aarch64-apple-darwin x86_64-pc-windows-msvc aarch64-pc-windows-msvc ;;
    *-*-*) echo "$1" ;;
    *) die "unknown target '$1' (see --help)" ;;
  esac
}

HOST_OS=$(uname -s)
case "$(uname -m)" in
  x86_64 | amd64) HOST_ARCH=x86_64 ;;
  arm64 | aarch64) HOST_ARCH=aarch64 ;;
  *) die "unsupported host architecture $(uname -m)" ;;
esac

TARGETS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --no-package) PACKAGE=0 ;;
    -h | --help) usage ;;
    *) for t in $(resolve "$1"); do TARGETS+=("$t"); done ;;
  esac
  shift
done
if [ ${#TARGETS[@]} -eq 0 ]; then
  case "$HOST_OS" in
    Linux) TARGETS=($(resolve linux)) ;;
    Darwin) TARGETS=($(resolve macos)) ;;
    *) die "on Windows use scripts/build.ps1" ;;
  esac
fi

have cargo || die "cargo not found — install Rust from https://rustup.rs"

# Picks the cargo subcommand able to build $1 on this host, or explains what is missing.
builder_for() {
  local target=$1
  case "$target" in
    *-apple-darwin)
      [ "$HOST_OS" = Darwin ] || { echo "skip:macOS targets must be built on macOS (or by the GitHub release workflow)"; return; }
      echo build ;;
    *-linux-gnu)
      if [ "$HOST_OS" = Linux ] && [ "${target%%-*}" = "$HOST_ARCH" ]; then
        echo build
      elif have cargo-zigbuild && have_zig; then
        echo zigbuild
      elif have cross; then
        echo cross
      else
        echo "skip:install cargo-zigbuild + zig (cargo install cargo-zigbuild; pip install ziglang) or cross (cargo install cross; needs Docker)"
      fi ;;
    *-linux-musl)
      if [ -f /etc/alpine-release ] && [ "${target%%-*}" = "$HOST_ARCH" ]; then
        echo musl
      else
        echo "skip:build on a musl host of the same arch, e.g. docker run --rm -v \"\$PWD:/src\" -w /src rust:alpine sh -c 'apk add bash build-base && scripts/build.sh $target'"
      fi ;;
    *-windows-msvc)
      if have cargo-xwin; then
        echo xwin
      else
        echo "skip:install cargo-xwin (cargo install cargo-xwin; needs clang and lld) or build on Windows with scripts/build.ps1"
      fi ;;
    *) echo "skip:unsupported target" ;;
  esac
}

package() {
  local target=$1 bin=$2 tui=$3
  local name="$APP-$VERSION-$target"
  local stage="$DIST/$name"
  rm -rf "$stage" && mkdir -p "$stage"
  cp "$bin" "$tui" README.md "$stage/"
  case "$target" in
    *-windows-msvc)
      if have zip; then
        (cd "$DIST" && rm -f "$name.zip" && zip -qr "$name.zip" "$name")
        info "packaged $DIST/$name.zip"
      else
        tar -C "$DIST" -czf "$DIST/$name.tar.gz" "$name"
        info "zip not found; packaged $DIST/$name.tar.gz"
      fi ;;
    *)
      tar -C "$DIST" -czf "$DIST/$name.tar.gz" "$name"
      info "packaged $DIST/$name.tar.gz" ;;
  esac
  rm -rf "$stage"
}

# macOS .app bundle (universal when both architectures were built).
bundle_macos() {
  local bins=("$@")
  local app="$DIST/$APP_NAME.app"
  rm -rf "$app" && mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  if [ ${#bins[@]} -gt 1 ] && have lipo; then
    lipo -create -output "$app/Contents/MacOS/$APP" "${bins[@]}"
  else
    cp "${bins[0]}" "$app/Contents/MacOS/$APP"
  fi
  cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>fr.lepagnot.llegion</string>
  <key>CFBundleExecutable</key><string>$APP</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
  # Ad-hoc signature so Gatekeeper accepts the (unsigned) universal binary locally.
  have codesign && codesign --force --deep --sign - "$app" >/dev/null 2>&1 || true
  local zip_name="$APP-$VERSION-macos-universal.zip"
  (cd "$DIST" && rm -f "$zip_name" && ditto -c -k --keepParent "$APP_NAME.app" "$zip_name" 2>/dev/null) \
    && info "packaged $DIST/$zip_name" || info "bundle at $app"
}

mkdir -p "$DIST"
BUILT=()
SKIPPED=()
MAC_BINS=()
for target in ${TARGETS[@]+"${TARGETS[@]}"}; do
  builder=$(builder_for "$target")
  if [[ "$builder" == skip:* ]]; then
    SKIPPED+=("$target: ${builder#skip:}")
    continue
  fi
  have rustup && rustup target add "$target" >/dev/null 2>&1 || true
  info "building $target (cargo $builder)"
  case "$builder" in
    build) cargo build --release --locked --features tui --target "$target" ;;
    # Pin an old glibc so the binary runs on older distributions.
    zigbuild) cargo zigbuild --release --locked --features tui --target "$target.2.31" ;;
    cross) cross build --release --locked --features tui --target "$target" ;;
    # Link musl dynamically: winit/glow dlopen X11, Wayland and GL, which a static binary can't.
    musl) RUSTFLAGS="-C target-feature=-crt-static" cargo build --release --locked --features tui --target "$target" ;;
    xwin) cargo xwin build --release --locked --features tui --target "$target" ;;
  esac
  ext=""
  [[ "$target" == *-windows-msvc ]] && ext=".exe"
  bin="target/$target/release/$APP$ext"
  tui="target/$target/release/$TUI$ext"
  for b in "$bin" "$tui"; do [ -f "$b" ] || die "expected binary not found: $b"; done
  BUILT+=("$target")
  [[ "$target" == *-apple-darwin ]] && MAC_BINS+=("$bin")
  [ $PACKAGE -eq 1 ] && package "$target" "$bin" "$tui"
done

if [ $PACKAGE -eq 1 ] && [ ${#MAC_BINS[@]} -gt 0 ]; then
  bundle_macos "${MAC_BINS[@]}"
fi

echo
echo "Built: ${BUILT[*]:-none}"
for s in ${SKIPPED[@]+"${SKIPPED[@]}"}; do echo "Skipped $s"; done
[ ${#BUILT[@]} -gt 0 ] || exit 1

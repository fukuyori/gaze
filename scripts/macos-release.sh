#!/usr/bin/env bash
#
# Builds a signed, notarised macOS installer for Gaze.
#
#   1. builds a universal release binary
#   2. assembles Gaze.app around it
#   3. signs the app with Developer ID Application + hardened runtime
#   4. wraps it in a .pkg signed with Developer ID Installer
#   5. submits the .pkg for notarisation and staples the ticket to it
#
# Run `scripts/macos-release.sh --help` for the settings it takes.

set -euo pipefail

readonly PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly EXECUTABLE="gaze"
readonly APP_NAME="Gaze"

BUNDLE_ID="${GAZE_BUNDLE_ID:-org.spumoni.gaze}"
MIN_MACOS="${GAZE_MIN_MACOS:-10.15}"
APP_CERT="${GAZE_APP_CERT:-}"
INSTALLER_CERT="${GAZE_INSTALLER_CERT:-}"
NOTARY_PROFILE="${GAZE_NOTARY_PROFILE:-notarytool}"
APPLE_ID="${GAZE_APPLE_ID:-}"
TEAM_ID="${GAZE_TEAM_ID:-}"
APP_PASSWORD="${GAZE_APP_PASSWORD:-}"
OUTPUT_DIR="${GAZE_OUTPUT_DIR:-$PROJECT_ROOT/dist}"

universal=1
sign=1
notarize=1

usage() {
	cat <<'EOF'
Usage: scripts/macos-release.sh [options]

Options:
  --unsigned        Build the .app and .pkg without signing or notarising.
                    Produces an installer only usable on this machine; use it
                    to check the packaging itself.
  --skip-notarize   Sign everything but do not submit to Apple.
  --host-arch       Build only for this Mac's architecture instead of a
                    universal binary.
  -h, --help        Show this message.

Settings, all read from the environment:
  GAZE_BUNDLE_ID       Bundle identifier. (default: org.spumoni.gaze)
                       Fork Gaze and this wants to become a reverse-DNS name
                       you own, since it is what macOS identifies the installed
                       app by.
  GAZE_MIN_MACOS       LSMinimumSystemVersion.   (default: 10.15)
  GAZE_OUTPUT_DIR      Where the .pkg is written. (default: <repo>/dist)

  GAZE_APP_CERT        "Developer ID Application: NAME (TEAMID)"
  GAZE_INSTALLER_CERT  "Developer ID Installer: NAME (TEAMID)"
                       Both are found in the keychain on their own, and are
                       only worth setting when you hold more than one Developer
                       ID and have to say which. List what you have with:
                         security find-identity -v

  Notarisation credentials, either a stored keychain profile:
  GAZE_NOTARY_PROFILE  Profile name.            (default: notarytool)
                       Create one once with:
                         xcrun notarytool store-credentials <name> \
                           --apple-id <id> --team-id <team> --password <app-specific-password>
  or the three values directly:
  GAZE_APPLE_ID        Apple ID e-mail address.
  GAZE_TEAM_ID         Ten-character team identifier.
  GAZE_APP_PASSWORD    App-specific password, NOT your Apple ID password.
                       Create one at https://account.apple.com under Sign-In
                       and Security. Prefer the keychain profile: it keeps the
                       password out of shell history, CI logs and this script's
                       environment. GAZE_NOTARY_PROFILE wins when both are set.

An optional 1024x1024 assets/icon.png is turned into the app icon.
EOF
}

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
	case "$1" in
		--unsigned) sign=0; notarize=0 ;;
		--skip-notarize) notarize=0 ;;
		--host-arch) universal=0 ;;
		-h|--help) usage; exit 0 ;;
		*) usage >&2; die "unknown option: $1" ;;
	esac
	shift
done

[ "$(uname -s)" = "Darwin" ] || die "this script only runs on macOS"

for tool in cargo lipo codesign pkgbuild productbuild xcrun security; do
	command -v "$tool" >/dev/null || die "$tool is not on PATH"
done

# Picks the one certificate of the given kind out of the keychain, so that the
# usual case of holding exactly one Developer ID needs no configuration at all.
resolve_identity() {
	local kind="$1" variable="$2" matches count
	matches="$(security find-identity -v 2>/dev/null |
		sed -n "s/.*\"\($kind: [^\"]*\)\".*/\1/p" | sort -u)"
	count="$(printf '%s' "$matches" | grep -c . || true)"

	case "$count" in
		0)
			die "no valid \"$kind\" certificate in the keychain.
  Install one from https://developer.apple.com/account/resources/certificates,
  set $variable to name it explicitly, or pass --unsigned."
			;;
		1) printf '%s' "$matches" ;;
		*)
			die "more than one \"$kind\" certificate is available:
$(printf '%s\n' "$matches" | sed 's/^/    /')
  Set $variable to the one you want."
			;;
	esac
}

if [ "$sign" -eq 1 ]; then
	if [ -z "$APP_CERT" ]; then
		APP_CERT="$(resolve_identity "Developer ID Application" GAZE_APP_CERT)" || exit 1
	fi
	if [ -z "$INSTALLER_CERT" ]; then
		INSTALLER_CERT="$(resolve_identity "Developer ID Installer" GAZE_INSTALLER_CERT)" || exit 1
	fi
	log "app signed by:       $APP_CERT"
	log "installer signed by: $INSTALLER_CERT"
fi

if [ "$notarize" -eq 1 ]; then
	if [ -z "$NOTARY_PROFILE" ]; then
		[ -n "$APPLE_ID" ] && [ -n "$TEAM_ID" ] && [ -n "$APP_PASSWORD" ] ||
			die "set GAZE_NOTARY_PROFILE, or all of GAZE_APPLE_ID, GAZE_TEAM_ID and GAZE_APP_PASSWORD (or pass --skip-notarize)"
	fi
fi

cd "$PROJECT_ROOT"

version="$(awk -F'"' '/^version = / { print $2; exit }' Cargo.toml)"
[ -n "$version" ] || die "could not read the version out of Cargo.toml"
log "packaging $APP_NAME $version"

# ---------------------------------------------------------------- build

build_dir="$(mktemp -d)"
trap 'rm -rf "$build_dir"' EXIT

binary="$build_dir/$EXECUTABLE"
if [ "$universal" -eq 1 ]; then
	for target in aarch64-apple-darwin x86_64-apple-darwin; do
		rustup target list --installed 2>/dev/null | grep -qx "$target" ||
			die "target $target is missing. Add it with:
    rustup target add $target
  or build for this Mac only with --host-arch."
	done

	log "building a universal binary"
	cargo build --release --target aarch64-apple-darwin
	cargo build --release --target x86_64-apple-darwin
	lipo -create -output "$binary" \
		"target/aarch64-apple-darwin/release/$EXECUTABLE" \
		"target/x86_64-apple-darwin/release/$EXECUTABLE"
else
	warn "building for this Mac's architecture only; the installer will not run on other Macs"
	log "building a release binary"
	cargo build --release
	cp "target/release/$EXECUTABLE" "$binary"
fi
lipo -info "$binary"

# ------------------------------------------------------------ app bundle

app="$build_dir/root/Applications/$APP_NAME.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
mv "$binary" "$app/Contents/MacOS/$EXECUTABLE"
chmod 755 "$app/Contents/MacOS/$EXECUTABLE"

# LSUIElement keeps Gaze out of the Dock and the app switcher, the way it
# already keeps itself out of the Windows taskbar. Click the eyes to make it
# the active application, then Command-Q to quit.
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>$EXECUTABLE</string>
	<key>CFBundleIdentifier</key>
	<string>$BUNDLE_ID</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>$APP_NAME</string>
	<key>CFBundleDisplayName</key>
	<string>$APP_NAME</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>CFBundleIconFile</key>
	<string>$APP_NAME</string>
	<key>LSMinimumSystemVersion</key>
	<string>$MIN_MACOS</string>
	<key>LSUIElement</key>
	<true/>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
</dict>
</plist>
EOF

icon_source="$PROJECT_ROOT/assets/icon.png"
if [ -f "$icon_source" ]; then
	log "building the app icon"
	iconset="$build_dir/$APP_NAME.iconset"
	mkdir -p "$iconset"
	for size in 16 32 128 256 512; do
		sips -z "$size" "$size" "$icon_source" --out "$iconset/icon_${size}x${size}.png" >/dev/null
		sips -z "$((size * 2))" "$((size * 2))" "$icon_source" \
			--out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
	done
	iconutil -c icns "$iconset" -o "$app/Contents/Resources/$APP_NAME.icns"
else
	warn "no assets/icon.png, so the app gets the generic icon"
fi

# ---------------------------------------------------------------- signing

if [ "$sign" -eq 1 ]; then
	log "signing $APP_NAME.app"
	# The hardened runtime and a secure timestamp are both required before
	# Apple will notarise anything.
	codesign --force --timestamp --options runtime \
		--sign "$APP_CERT" "$app"
	codesign --verify --deep --strict --verbose=2 "$app"
else
	warn "skipping code signing"
fi

# ---------------------------------------------------------------- packaging

mkdir -p "$OUTPUT_DIR"
component_pkg="$build_dir/$APP_NAME-component.pkg"
distribution="$build_dir/distribution.xml"
final_pkg="$OUTPUT_DIR/$APP_NAME-$version.pkg"

log "building the installer"
pkgbuild \
	--root "$build_dir/root" \
	--identifier "$BUNDLE_ID" \
	--version "$version" \
	--ownership recommended \
	--install-location / \
	"$component_pkg"

productbuild --synthesize --package "$component_pkg" "$distribution"

if [ "$sign" -eq 1 ]; then
	productbuild --distribution "$distribution" --package-path "$build_dir" \
		--sign "$INSTALLER_CERT" "$final_pkg"
	pkgutil --check-signature "$final_pkg"
else
	productbuild --distribution "$distribution" --package-path "$build_dir" "$final_pkg"
fi

# ------------------------------------------------------------ notarisation

if [ "$notarize" -eq 1 ]; then
	log "submitting to Apple; this usually takes a few minutes"
	if [ -n "$NOTARY_PROFILE" ]; then
		xcrun notarytool submit "$final_pkg" \
			--keychain-profile "$NOTARY_PROFILE" --wait
	else
		xcrun notarytool submit "$final_pkg" \
			--apple-id "$APPLE_ID" --team-id "$TEAM_ID" --password "$APP_PASSWORD" --wait
	fi

	log "stapling the ticket"
	# Stapling lets the installer verify offline; without it a machine with no
	# network shows the unidentified-developer warning.
	xcrun stapler staple "$final_pkg"
	xcrun stapler validate "$final_pkg"

	log "checking the installer the way Gatekeeper will"
	spctl --assess --type install --verbose=4 "$final_pkg"
else
	warn "skipping notarisation; macOS will refuse this installer on other machines"
fi

log "done: $final_pkg"

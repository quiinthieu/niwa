#!/usr/bin/env bash
# Build universal (arm64 + x86_64) macOS menu bar app bundle, sign (Developer ID +
# notarytool + stapler + spctl -a when secrets are present, or $0 ad-hoc codesign
# otherwise), and package a mount-verified compressed UDZO .dmg (PLAN.md §10 T17).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

OVERRIDE_VERSION=""
OUT_DMG=""
INPUT_APP_BUNDLE=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            OVERRIDE_VERSION="$2"
            shift 2
            ;;
        --output)
            OUT_DMG="$2"
            shift 2
            ;;
        --app-bundle)
            INPUT_APP_BUNDLE="$2"
            shift 2
            ;;
        -h|--help)
            cat <<'EOF'
Usage: ./scripts/package-macos-dmg.sh [--version <ver>] [--output <path.dmg>] [--app-bundle <path.app>]
EOF
            exit 0
            ;;
        *)
            echo "ERROR: Unknown argument '$1'" >&2
            exit 1
            ;;
    esac
done

DISPLAY_NAME="$(python3 - "$REPO_ROOT" <<'PY'
import pathlib, re, sys
src = (pathlib.Path(sys.argv[1]) / "crates/tg-core/src/product.rs").read_text(encoding="utf-8")
m = re.search(r'pub const DISPLAY_NAME:\s*&str\s*=\s*"([^"]+)";', src)
if not m:
    raise SystemExit("ERROR: DISPLAY_NAME missing in product.rs")
print(m.group(1))
PY
)"

DEFAULT_VERSION="$(python3 - "$REPO_ROOT" <<'PY'
import pathlib, re, sys
src = (pathlib.Path(sys.argv[1]) / "Cargo.toml").read_text(encoding="utf-8")
m = re.search(r'^version\s*=\s*"([^"]+)"', src, flags=re.MULTILINE)
if not m:
    raise SystemExit("ERROR: version missing in Cargo.toml")
print(m.group(1))
PY
)"

VERSION="${OVERRIDE_VERSION:-$DEFAULT_VERSION}"
VERSION="${VERSION#v}"

if [ -z "$OUT_DMG" ]; then
    OUT_DMG="$REPO_ROOT/dist/${DISPLAY_NAME}-v${VERSION}-macos.dmg"
fi

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/niwa-dmg.XXXXXX")"
MOUNT_DIR="$WORK_DIR/mnt"
MOUNTED=0

cleanup() {
    if [ "$MOUNTED" -eq 1 ] && [ -d "$MOUNT_DIR" ]; then
        hdiutil detach "$MOUNT_DIR" -quiet 2>/dev/null || hdiutil detach "$MOUNT_DIR" -force -quiet 2>/dev/null || true
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

if [ -n "$INPUT_APP_BUNDLE" ]; then
    if [ ! -d "$INPUT_APP_BUNDLE" ]; then
        echo "ERROR: Provided --app-bundle '$INPUT_APP_BUNDLE' does not exist." >&2
        exit 1
    fi
    APP_BUNDLE="$WORK_DIR/${DISPLAY_NAME}.app"
    ditto "$INPUT_APP_BUNDLE" "$APP_BUNDLE"
else
    DERIVED_DATA_DIR="$WORK_DIR/DerivedData"
    xcodegen generate --spec "$REPO_ROOT/macos/Project.yml" --project "$REPO_ROOT/macos"
    xcodebuild \
        -project "$REPO_ROOT/macos/TerminalGarden.xcodeproj" \
        -scheme TerminalGarden \
        -configuration Release \
        -derivedDataPath "$DERIVED_DATA_DIR" \
        ARCHS="arm64 x86_64" \
        ONLY_ACTIVE_ARCH=NO \
        MARKETING_VERSION="$VERSION" \
        NIWA_VERSION="$VERSION" \
        CODE_SIGNING_ALLOWED=NO \
        CODE_SIGNING_REQUIRED=NO \
        build

    BUILT_APP="$DERIVED_DATA_DIR/Build/Products/Release/${DISPLAY_NAME}.app"
    if [ ! -d "$BUILT_APP" ]; then
        echo "ERROR: Expected built app bundle at '$BUILT_APP' was not found." >&2
        exit 1
    fi
    APP_BUNDLE="$WORK_DIR/${DISPLAY_NAME}.app"
    ditto "$BUILT_APP" "$APP_BUNDLE"
fi

APP_EXECUTABLE="$APP_BUNDLE/Contents/MacOS/$DISPLAY_NAME"
if [ ! -f "$APP_EXECUTABLE" ]; then
    echo "ERROR: Missing app executable '$APP_EXECUTABLE'." >&2
    exit 1
fi

ARCHS_OUT="$(lipo -archs "$APP_EXECUTABLE")"
echo "Verified ${DISPLAY_NAME}.app architectures: $ARCHS_OUT"
case " $ARCHS_OUT " in
    *" arm64 "*) ;;
    *)
        echo "ERROR: ${DISPLAY_NAME}.app is missing arm64 slice (got: $ARCHS_OUT)" >&2
        exit 1
        ;;
esac
case " $ARCHS_OUT " in
    *" x86_64 "*) ;;
    *)
        echo "ERROR: ${DISPLAY_NAME}.app is missing x86_64 slice (got: $ARCHS_OUT)" >&2
        exit 1
        ;;
esac

submit_notarytool() {
    target_path="$1"
    if [ -n "${APPLE_API_KEY_PATH:-}" ] && [ -n "${APPLE_API_KEY_ID:-}" ] && [ -n "${APPLE_API_ISSUER_ID:-}" ]; then
        xcrun notarytool submit "$target_path" \
            --key "$APPLE_API_KEY_PATH" \
            --key-id "$APPLE_API_KEY_ID" \
            --issuer "$APPLE_API_ISSUER_ID" \
            --wait
        return 0
    elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
        xcrun notarytool submit "$target_path" \
            --apple-id "$APPLE_ID" \
            --password "$APPLE_APP_SPECIFIC_PASSWORD" \
            --team-id "$APPLE_TEAM_ID" \
            --wait
        return 0
    fi
    return 1
}

HAS_NOTARIZATION_CREDS=0
if { [ -n "${APPLE_API_KEY_PATH:-}" ] && [ -n "${APPLE_API_KEY_ID:-}" ] && [ -n "${APPLE_API_ISSUER_ID:-}" ]; } || \
   { [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_APP_SPECIFIC_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; }; then
    HAS_NOTARIZATION_CREDS=1
fi

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    echo "Signing ${DISPLAY_NAME}.app with Apple Developer ID identity: $APPLE_SIGNING_IDENTITY"
    codesign --force --deep --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$APP_BUNDLE"
    codesign --verify --deep --strict --verbose=2 "$APP_BUNDLE"

    if [ "$HAS_NOTARIZATION_CREDS" -eq 1 ]; then
        echo "Submitting ${DISPLAY_NAME}.app to Apple Notary Service..."
        APP_ZIP="$WORK_DIR/${DISPLAY_NAME}-notarize.zip"
        ditto -c -k --keepParent "$APP_BUNDLE" "$APP_ZIP"
        submit_notarytool "$APP_ZIP"
        xcrun stapler staple "$APP_BUNDLE"
        spctl -a -vvv -t exec "$APP_BUNDLE"
    else
        echo "NOTICE: APPLE_SIGNING_IDENTITY is set, but notarization credentials are absent; skipping notarytool and spctl -a."
    fi
else
    echo "NOTICE: APPLE_SIGNING_IDENTITY is unset (\$0 open-source mode)."
    echo "Signing ${DISPLAY_NAME}.app ad-hoc and verifying Mach-O/bundle signature integrity..."
    codesign --force --deep --sign - "$APP_BUNDLE"
    codesign --verify --deep --strict --verbose=2 "$APP_BUNDLE"
fi

STAGE_DIR="$WORK_DIR/dmg-stage"
mkdir -p "$STAGE_DIR"
ditto "$APP_BUNDLE" "$STAGE_DIR/${DISPLAY_NAME}.app"
ln -s /Applications "$STAGE_DIR/Applications"

mkdir -p "$(dirname "$OUT_DMG")"
rm -f "$OUT_DMG"
hdiutil create \
    -volname "$DISPLAY_NAME" \
    -srcfolder "$STAGE_DIR" \
    -ov \
    -format UDZO \
    "$OUT_DMG"

if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then
    codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$OUT_DMG"
    if [ "$HAS_NOTARIZATION_CREDS" -eq 1 ]; then
        echo "Notarizing and stapling DMG..."
        submit_notarytool "$OUT_DMG"
        xcrun stapler staple "$OUT_DMG"
    fi
fi

mkdir -p "$MOUNT_DIR"
hdiutil attach -readonly -nobrowse -noautoopen -mountpoint "$MOUNT_DIR" "$OUT_DMG"
MOUNTED=1

if [ ! -f "$MOUNT_DIR/${DISPLAY_NAME}.app/Contents/Info.plist" ]; then
    echo "ERROR: Mounted DMG is missing ${DISPLAY_NAME}.app/Contents/Info.plist" >&2
    exit 1
fi
if [ ! -L "$MOUNT_DIR/Applications" ]; then
    echo "ERROR: Mounted DMG is missing /Applications symlink" >&2
    exit 1
fi
codesign --verify --deep --strict "$MOUNT_DIR/${DISPLAY_NAME}.app"

hdiutil detach "$MOUNT_DIR" -quiet
MOUNTED=0

echo "Packaged and mount-verified DMG: $OUT_DMG"

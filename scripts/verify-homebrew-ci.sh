#!/usr/bin/env bash
# Verify Niwa's Homebrew Formula and Cask end-to-end:
#   1. Render Formula/<formula>.rb and Casks/<cask>.rb into a temporary local tap
#      ($(brew --repository)/Library/Taps/<owner>/homebrew-tap) using real SHA-256 digests.
#   2. Run `brew style` on both Ruby manifests.
#   3. Run `brew audit --strict` on both the formula and the cask.
#   4. Seed Homebrew's download cache with a `git archive` tarball of HEAD, run
#      `brew install --build-from-source`, and run `brew test`.
# Restores any pre-existing tap state on exit so no residue is left behind.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

AUDIT_ONLY=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --audit-only)
            AUDIT_ONLY=1
            shift
            ;;
        -h|--help)
            echo "Usage: ./scripts/verify-homebrew-ci.sh [--audit-only]"
            exit 0
            ;;
        *)
            echo "ERROR: Unknown argument '$1'" >&2
            exit 1
            ;;
    esac
done

if ! command -v brew >/dev/null 2>&1; then
    echo "ERROR: Homebrew ('brew') is required to run verify-homebrew-ci.sh" >&2
    exit 1
fi

META="$(python3 - "$REPO_ROOT" <<'PY'
import pathlib, re, sys
root = pathlib.Path(sys.argv[1])
prod = (root / "crates/tg-core/src/product.rs").read_text(encoding="utf-8")
cargo = (root / "Cargo.toml").read_text(encoding="utf-8")

def get_const(name: str) -> str:
    m = re.search(rf'pub const {name}:\s*&str\s*=\s*"([^"]+)";', prod)
    if not m:
        raise SystemExit(f"ERROR: Missing {name} in product.rs")
    return m.group(1)

ver = re.search(r'^version\s*=\s*"([^"]+)"', cargo, flags=re.MULTILINE)
repo = re.search(r'^repository\s*=\s*"([^"]+)"', cargo, flags=re.MULTILINE)
if not ver or not repo:
    raise SystemExit("ERROR: Missing version/repository in Cargo.toml")
repository = repo.group(1).rstrip("/")
owner_m = re.search(r"github\.com/([^/]+)/", repository)
if not owner_m:
    raise SystemExit(f"ERROR: Unsupported repository URL {repository}")

print(get_const("CLI_BIN"))
print(get_const("HOMEBREW_FORMULA"))
print(get_const("HOMEBREW_CASK"))
print(ver.group(1))
print(repository)
print(owner_m.group(1))
PY
)"

CLI_BIN="$(printf '%s\n' "$META" | sed -n '1p')"
FORMULA_NAME="$(printf '%s\n' "$META" | sed -n '2p')"
CASK_TOKEN="$(printf '%s\n' "$META" | sed -n '3p')"
VERSION="$(printf '%s\n' "$META" | sed -n '4p')"
REPOSITORY="$(printf '%s\n' "$META" | sed -n '5p')"
REPO_OWNER="$(printf '%s\n' "$META" | sed -n '6p')"

export HOMEBREW_NO_AUTO_UPDATE=1
export HOMEBREW_NO_INSTALL_CLEANUP=1
export HOMEBREW_NO_ENV_HINTS=1

BREW_REPO="$(brew --repository)"
TAP_PARENT="$BREW_REPO/Library/Taps/$REPO_OWNER"
TAP_DIR="$TAP_PARENT/homebrew-tap"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/niwa-brew-ci.XXXXXX")"
export XDG_CONFIG_HOME="$WORK_DIR/xdg-config"
mkdir -m 0700 -p "$XDG_CONFIG_HOME" "$XDG_CONFIG_HOME/homebrew"
HAD_EXISTING_TAP=0
INSTALLED_FORMULA=0
CACHED_TARBALL=""

cleanup() {
    if [ "$INSTALLED_FORMULA" -eq 1 ]; then
        brew uninstall --force "${REPO_OWNER}/tap/${FORMULA_NAME}" >/dev/null 2>&1 || true
    fi
    if [ -n "$CACHED_TARBALL" ] && [ -f "$CACHED_TARBALL" ]; then
        rm -f "$CACHED_TARBALL"
    fi
    rm -rf "$TAP_DIR"
    if [ "$HAD_EXISTING_TAP" -eq 1 ] && [ -d "$WORK_DIR/tap-backup" ]; then
        mkdir -p "$TAP_PARENT"
        mv "$WORK_DIR/tap-backup" "$TAP_DIR"
    else
        rmdir "$TAP_PARENT" 2>/dev/null || true
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

if [ -d "$TAP_DIR" ]; then
    HAD_EXISTING_TAP=1
    mv "$TAP_DIR" "$WORK_DIR/tap-backup"
fi

SOURCE_TARBALL="$WORK_DIR/${CLI_BIN}-${VERSION}.tar.gz"
git -C "$REPO_ROOT" archive \
    --format=tar.gz \
    --prefix="${CLI_BIN}-${VERSION}/" \
    HEAD \
    -o "$SOURCE_TARBALL"

FORMULA_URL="${REPOSITORY}/archive/refs/tags/v${VERSION}.tar.gz"
FORMULA_SHA256="$(shasum -a 256 "$SOURCE_TARBALL" | awk '{print $1}')"
CASK_SHA256="$(printf 'niwa-cask-audit-%s\n' "$VERSION" | shasum -a 256 | awk '{print $1}')"

mkdir -p "$TAP_DIR"
git -C "$TAP_DIR" init -q
git -C "$TAP_DIR" config user.name "niwa-ci"
git -C "$TAP_DIR" config user.email "niwa-ci@users.noreply.github.com"

bash "$REPO_ROOT/scripts/render-homebrew.sh" \
    --output-dir "$TAP_DIR" \
    --version "$VERSION" \
    --formula-url "$FORMULA_URL" \
    --formula-sha256 "$FORMULA_SHA256" \
    --cask-sha256 "$CASK_SHA256"

git -C "$TAP_DIR" add Formula Casks
git -C "$TAP_DIR" commit -q -m "chore: seed tap for CI verification"

if brew trust --help >/dev/null 2>&1; then
    brew trust --tap "${REPO_OWNER}/tap"
fi

echo "==> Running brew style on rendered Formula and Cask..."
brew style \
    "$TAP_DIR/Formula/${FORMULA_NAME}.rb" \
    "$TAP_DIR/Casks/${CASK_TOKEN}.rb"

echo "==> Running brew audit --strict on formula ${REPO_OWNER}/tap/${FORMULA_NAME}..."
brew audit --strict --formula "${REPO_OWNER}/tap/${FORMULA_NAME}"

echo "==> Running brew audit --strict on cask ${REPO_OWNER}/tap/${CASK_TOKEN}..."
brew audit --strict --cask "${REPO_OWNER}/tap/${CASK_TOKEN}"

if [ "$AUDIT_ONLY" -eq 1 ]; then
    echo "Homebrew strict audit passed (--audit-only)."
    exit 0
fi

BREW_CACHE_DIR="$(brew --cache)"
URL_SHA256="$(printf '%s' "$FORMULA_URL" | shasum -a 256 | awk '{print $1}')"
mkdir -p "$BREW_CACHE_DIR/downloads"
rm -f "$BREW_CACHE_DIR/downloads/${URL_SHA256}--"*
CACHED_TARBALL="$BREW_CACHE_DIR/downloads/${URL_SHA256}--${CLI_BIN}-${VERSION}.tar.gz"
cp "$SOURCE_TARBALL" "$CACHED_TARBALL"

echo "==> Running brew install --build-from-source ${REPO_OWNER}/tap/${FORMULA_NAME}..."
brew install --build-from-source "${REPO_OWNER}/tap/${FORMULA_NAME}"
INSTALLED_FORMULA=1

echo "==> Running brew test ${REPO_OWNER}/tap/${FORMULA_NAME}..."
brew test "${REPO_OWNER}/tap/${FORMULA_NAME}"

echo "Homebrew CI verification passed (brew style, brew audit --strict, brew install --build-from-source, brew test)."

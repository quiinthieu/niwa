#!/usr/bin/env bash
# Generate Homebrew Formula and Cask definitions from single-source product constants
# in crates/tg-core/src/product.rs and Cargo.toml (PLAN.md §0, §1.3 & §10 T17).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

OUTPUT_DIR="$REPO_ROOT/packaging/homebrew"
OVERRIDE_VERSION=""
FORMULA_URL=""
FORMULA_SHA256="0000000000000000000000000000000000000000000000000000000000000000"
CASK_URL=""
CASK_SHA256="0000000000000000000000000000000000000000000000000000000000000000"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --version)
            OVERRIDE_VERSION="$2"
            shift 2
            ;;
        --formula-url)
            FORMULA_URL="$2"
            shift 2
            ;;
        --formula-sha256)
            FORMULA_SHA256="$2"
            shift 2
            ;;
        --cask-url)
            CASK_URL="$2"
            shift 2
            ;;
        --cask-sha256)
            CASK_SHA256="$2"
            shift 2
            ;;
        -h|--help)
            cat <<'EOF'
Usage: ./scripts/render-homebrew.sh [options]

Options:
  --output-dir <dir>        Target tap root directory (default: packaging/homebrew)
  --version <ver>           Release version without leading 'v' (default: Cargo.toml version)
  --formula-url <url>       Source archive URL for Formula
  --formula-sha256 <hex>    64-char SHA-256 digest for Formula archive
  --cask-url <url>          DMG download URL for Cask (supports #{version})
  --cask-sha256 <hex>       64-char SHA-256 digest for Cask DMG
EOF
            exit 0
            ;;
        *)
            echo "ERROR: Unknown argument '$1'" >&2
            exit 1
            ;;
    esac
done

python3 - \
    "$REPO_ROOT" \
    "$OUTPUT_DIR" \
    "$OVERRIDE_VERSION" \
    "$FORMULA_URL" \
    "$FORMULA_SHA256" \
    "$CASK_URL" \
    "$CASK_SHA256" <<'PY'
import pathlib
import re
import sys

(
    repo_root_raw,
    output_dir_raw,
    override_version,
    formula_url_arg,
    formula_sha256,
    cask_url_arg,
    cask_sha256,
) = sys.argv[1:8]

repo_root = pathlib.Path(repo_root_raw)
output_dir = pathlib.Path(output_dir_raw)

product_rs = (repo_root / "crates/tg-core/src/product.rs").read_text(encoding="utf-8")
cargo_toml = (repo_root / "Cargo.toml").read_text(encoding="utf-8")

constants = {}
for key in (
    "DISPLAY_NAME",
    "CLI_BIN",
    "CLI_CRATE",
    "HOMEBREW_FORMULA",
    "HOMEBREW_CASK",
    "BUNDLE_ID",
):
    match = re.search(rf'pub const {key}:\s*&str\s*=\s*"([^"]+)";', product_rs)
    if not match:
        raise SystemExit(f"ERROR: Could not parse {key} from crates/tg-core/src/product.rs")
    constants[key] = match.group(1)

ver_match = re.search(r'^version\s*=\s*"([^"]+)"', cargo_toml, flags=re.MULTILINE)
repo_match = re.search(r'^repository\s*=\s*"([^"]+)"', cargo_toml, flags=re.MULTILINE)
if not ver_match or not repo_match:
    raise SystemExit("ERROR: Could not parse workspace version/repository from Cargo.toml")

version = (override_version or ver_match.group(1)).lstrip("v")
repository = repo_match.group(1).rstrip("/")

repo_owner_match = re.search(r"github\.com/([^/]+)/", repository)
if not repo_owner_match:
    raise SystemExit(f"ERROR: Expected GitHub repository URL in Cargo.toml, got {repository}")
repo_owner = repo_owner_match.group(1)

empty_sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
for label, digest in (("formula-sha256", formula_sha256), ("cask-sha256", cask_sha256)):
    if not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise SystemExit(f"ERROR: --{label} must be 64 lowercase hexadecimal characters, got '{digest}'")
    if digest == empty_sha256:
        raise SystemExit(f"ERROR: --{label} cannot be the empty-string SHA-256 ({empty_sha256})")

display_name = constants["DISPLAY_NAME"]
cli_bin = constants["CLI_BIN"]
formula_name = constants["HOMEBREW_FORMULA"]
cask_token = constants["HOMEBREW_CASK"]
bundle_id = constants["BUNDLE_ID"]

formula_class = "".join(part.capitalize() for part in formula_name.split("-"))
formula_url = formula_url_arg or f"{repository}/archive/refs/tags/v{version}.tar.gz"
cask_url = (
    cask_url_arg
    or f"{repository}/releases/download/v#{{version}}/{display_name}-v#{{version}}-macos.dmg"
)

# Homebrew's ResourceAuditor#audit_version rejects an explicit `version` stanza when the
# same version is already detectable from a tag tarball URL, whereas commit-SHA archive
# URLs require an explicit `version` stanza so the 40-hex commit SHA isn't treated as the version.
url_embeds_version = bool(
    re.search(rf"(?:^|[/-])v?{re.escape(version)}\.tar\.gz$", formula_url)
)
optional_formula_version = "" if url_embeds_version else f'  version "{version}"\n'

formula_rb = (
    f"class {formula_class} < Formula\n"
    f'  desc "Cozy idle garden for your terminal and menu bar"\n'
    f'  homepage "{repository}"\n'
    f'  url "{formula_url}"\n'
    f"{optional_formula_version}"
    f'  sha256 "{formula_sha256}"\n'
    f'  license any_of: ["MIT", "Apache-2.0"]\n'
    f'  head "{repository}.git", branch: "main"\n'
    f"\n"
    f'  depends_on "rust" => :build\n'
    f"\n"
    f"  def install\n"
    f'    system "cargo", "install", *std_cargo_args(path: "crates/{cli_bin}")\n'
    f"  end\n"
    f"\n"
    f"  test do\n"
    f'    ENV["NIWA_HOME"] = (testpath/"{cli_bin}-home").to_s\n'
    f'    assert_match version.to_s, shell_output("#{{bin}}/{cli_bin} --version")\n'
    f'    status_json = shell_output("#{{bin}}/{cli_bin} status --json")\n'
    f'    assert_match "\\"schema_version\\"", status_json\n'
    f'    system bin/"{cli_bin}", "peek"\n'
    f'    assert_path_exists testpath/"{cli_bin}-home"\n'
    f"  end\n"
    f"end\n"
)

cask_rb = (
    f'cask "{cask_token}" do\n'
    f'  version "{version}"\n'
    f'  sha256 "{cask_sha256}"\n'
    f"\n"
    f'  url "{cask_url}"\n'
    f'  name "{display_name}"\n'
    f'  desc "Menu bar companion for the cozy idle terminal garden"\n'
    f'  homepage "{repository}"\n'
    f"\n"
    f'  depends_on formula: "{formula_name}"\n'
    f"  depends_on macos: :sonoma\n"
    f"\n"
    f'  app "{display_name}.app"\n'
    f"\n"
    f"  zap trash: [\n"
    f'    "~/Library/Application Support/{display_name}",\n'
    f'    "~/Library/Caches/{bundle_id}",\n'
    f'    "~/Library/Preferences/{bundle_id}.plist",\n'
    f'    "~/Library/Saved Application State/{bundle_id}.savedState",\n'
    f"  ]\n"
    f"\n"
    f"  caveats <<~EOS\n"
    f"    #{{token}} is signed ad-hoc when built without an Apple Developer ID certificate.\n"
    f"    If Gatekeeper blocks first launch, install with `--no-quarantine` or clear the\n"
    f"    quarantine attribute manually:\n"
    f"      brew install --cask --no-quarantine {repo_owner}/tap/#{{token}}\n"
    f'      xattr -dr com.apple.quarantine "/Applications/{display_name}.app"\n'
    f"  EOS\n"
    f"end\n"
)

formula_dir = output_dir / "Formula"
casks_dir = output_dir / "Casks"
formula_dir.mkdir(parents=True, exist_ok=True)
casks_dir.mkdir(parents=True, exist_ok=True)

formula_path = formula_dir / f"{formula_name}.rb"
cask_path = casks_dir / f"{cask_token}.rb"
formula_path.write_text(formula_rb, encoding="utf-8")
cask_path.write_text(cask_rb, encoding="utf-8")
print(f"Rendered {formula_path} and {cask_path}")
PY

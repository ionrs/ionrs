#!/usr/bin/env bash
# Package the Ion Zed extension for release.
#
# Usage:
#   ./release.sh              # build and package
#   ./release.sh --clean      # clean first
#
# Output: dist/ion-zed-v{VERSION}.tar.gz
#
# The archive contains the complete extension ready for:
#   1. Manual install: extract to ~/.local/share/zed/extensions/installed/ion/
#   2. Registry submission: reference from zed-industries/extensions

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TS_GRAMMAR_DIR="$REPO_ROOT/tree-sitter-ion"
ZED_EXT_DIR="$SCRIPT_DIR"
DIST_DIR="$ZED_EXT_DIR/dist"

# Read version from extension.toml
VERSION=$(grep '^version' "$ZED_EXT_DIR/extension.toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')

# ── Colors ───────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${BLUE}==>${NC} ${BOLD}$1${NC}"; }
ok()    { echo -e "${GREEN}  ✓${NC} $1"; }
err()   { echo -e "${RED}  ✗${NC} $1" >&2; }

# ── Clean ────────────────────────────────────────────────────
if [[ "${1:-}" == "--clean" ]]; then
    info "Cleaning..."
    rm -rf "$ZED_EXT_DIR/target" "$DIST_DIR"
    rm -f "$TS_GRAMMAR_DIR/tree-sitter-ion.wasm"
    ok "Clean complete"
fi

# ── Prerequisites ────────────────────────────────────────────
info "Checking prerequisites..."

if ! command -v rustup &>/dev/null; then
    err "rustup not found. Install from https://rustup.rs"
    exit 1
fi

if ! command -v npx &>/dev/null; then
    err "npx not found. Install Node.js from https://nodejs.org"
    exit 1
fi

if ! rustup target list --installed | grep -q wasm32-wasip2; then
    rustup target add wasm32-wasip2
    ok "Added wasm32-wasip2 target"
else
    ok "wasm32-wasip2 ready"
fi

# ── Build extension WASM ─────────────────────────────────────
info "Building extension WASM..."
cd "$ZED_EXT_DIR"
RUSTC_WRAPPER="" cargo build --target wasm32-wasip2 --release 2>&1
WASM_FILE="$ZED_EXT_DIR/target/wasm32-wasip2/release/zed_ion.wasm"
ok "Extension: $(du -h "$WASM_FILE" | cut -f1)"

# ── Build tree-sitter grammar WASM ───────────────────────────
info "Building tree-sitter grammar WASM..."
cd "$TS_GRAMMAR_DIR"

# Regenerate parser from grammar.js (ensures src/parser.c is up to date)
npx tree-sitter-cli generate 2>&1

# Build WASM
npx tree-sitter-cli build --wasm 2>&1
GRAMMAR_WASM="$TS_GRAMMAR_DIR/tree-sitter-ion.wasm"
ok "Grammar: $(du -h "$GRAMMAR_WASM" | cut -f1)"

# ── Validate ─────────────────────────────────────────────────
info "Validating..."

# Test parser on example files
ERRORS=0
for f in "$REPO_ROOT"/examples/*.ion "$REPO_ROOT"/tests/scripts/*.ion; do
    if [[ -f "$f" ]]; then
        COUNT=$(npx tree-sitter-cli parse "$f" 2>&1 | grep -c "ERROR" || true)
        if [[ "$COUNT" -gt 0 ]]; then
            err "Parse errors in $(basename "$f"): $COUNT"
            ERRORS=$((ERRORS + 1))
        fi
    fi
done
if [[ "$ERRORS" -eq 0 ]]; then
    ok "All .ion files parse cleanly"
else
    err "$ERRORS files had parse errors (non-fatal, continuing)"
fi

# ── Package ──────────────────────────────────────────────────
info "Packaging v${VERSION}..."
STAGING="$DIST_DIR/ion"
rm -rf "$DIST_DIR"
mkdir -p "$STAGING/languages/ion"
mkdir -p "$STAGING/grammars"

# Manifest
cp "$ZED_EXT_DIR/extension.toml" "$STAGING/"

# Compiled WASM
cp "$WASM_FILE" "$STAGING/extension.wasm"

# Grammar WASM
cp "$GRAMMAR_WASM" "$STAGING/grammars/ion.wasm"

# Language files
cp "$ZED_EXT_DIR/languages/ion/config.toml"    "$STAGING/languages/ion/"
cp "$ZED_EXT_DIR/languages/ion/highlights.scm"  "$STAGING/languages/ion/"
cp "$ZED_EXT_DIR/languages/ion/brackets.scm"    "$STAGING/languages/ion/"
cp "$ZED_EXT_DIR/languages/ion/indents.scm"     "$STAGING/languages/ion/"
cp "$ZED_EXT_DIR/languages/ion/outline.scm"     "$STAGING/languages/ion/"

# Create tarball
ARCHIVE="$DIST_DIR/ion-zed-v${VERSION}.tar.gz"
cd "$DIST_DIR"
tar czf "$ARCHIVE" ion/
ok "Archive: $ARCHIVE"

# Show contents
echo ""
info "Package contents:"
tar tzf "$ARCHIVE" | sed 's/^/  /'

# Size summary
echo ""
info "Release v${VERSION} ready!"
echo "  Archive:   $ARCHIVE ($(du -h "$ARCHIVE" | cut -f1))"
echo "  Extension: $(du -h "$STAGING/extension.wasm" | cut -f1)"
echo "  Grammar:   $(du -h "$STAGING/grammars/ion.wasm" | cut -f1)"
echo ""
echo "  Manual install:"
echo "    tar xzf $(basename "$ARCHIVE") -C \"\$(zed --extensions-dir)/installed/\""
echo "    # or on Linux:"
echo "    tar xzf $(basename "$ARCHIVE") -C ~/.local/share/zed/extensions/installed/"
echo "    # or on macOS:"
echo "    tar xzf $(basename "$ARCHIVE") -C ~/Library/Application\\ Support/Zed/extensions/installed/"
echo ""
echo "  Registry submission:"
echo "    See https://github.com/zed-industries/extensions#adding-an-extension"

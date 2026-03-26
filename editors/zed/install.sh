#!/usr/bin/env bash
# Install the Ion language extension for Zed editor.
#
# Usage:
#   ./install.sh          # build and install
#   ./install.sh --clean  # clean build artifacts first
#
# Prerequisites:
#   - rustup with wasm32-wasip2 target (added automatically)
#   - tree-sitter-cli (installed via npx if needed)
#   - Node.js (for tree-sitter grammar build)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TS_GRAMMAR_DIR="$REPO_ROOT/tree-sitter-ion"
ZED_EXT_DIR="$SCRIPT_DIR"

# ── Platform detection ───────────────────────────────────────
detect_zed_dir() {
    case "$(uname -s)" in
        Darwin)
            echo "$HOME/Library/Application Support/Zed/extensions"
            ;;
        Linux)
            echo "${XDG_DATA_HOME:-$HOME/.local/share}/zed/extensions"
            ;;
        *)
            echo >&2 "Error: unsupported platform $(uname -s)"
            exit 1
            ;;
    esac
}

ZED_EXTENSIONS_DIR="$(detect_zed_dir)"
INSTALL_DIR="$ZED_EXTENSIONS_DIR/installed/ion"

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
    info "Cleaning build artifacts..."
    rm -rf "$ZED_EXT_DIR/target"
    rm -f "$TS_GRAMMAR_DIR/tree-sitter-ion.wasm"
    ok "Clean complete"
fi

# ── Step 1: Ensure wasm32-wasip2 target ──────────────────────
info "Checking Rust WASM target..."
if ! rustup target list --installed | grep -q wasm32-wasip2; then
    rustup target add wasm32-wasip2
    ok "Added wasm32-wasip2 target"
else
    ok "wasm32-wasip2 already installed"
fi

# ── Step 2: Build extension WASM ─────────────────────────────
info "Building Zed extension (wasm32-wasip2 release)..."
cd "$ZED_EXT_DIR"
RUSTC_WRAPPER="" cargo build --target wasm32-wasip2 --release 2>&1
WASM_FILE="$ZED_EXT_DIR/target/wasm32-wasip2/release/zed_ion.wasm"
if [[ ! -f "$WASM_FILE" ]]; then
    err "Extension WASM not found at $WASM_FILE"
    exit 1
fi
ok "Extension compiled ($(du -h "$WASM_FILE" | cut -f1))"

# ── Step 3: Build tree-sitter grammar WASM ───────────────────
info "Building tree-sitter-ion grammar (WASM)..."
cd "$TS_GRAMMAR_DIR"
npx tree-sitter-cli build --wasm 2>&1
GRAMMAR_WASM="$TS_GRAMMAR_DIR/tree-sitter-ion.wasm"
if [[ ! -f "$GRAMMAR_WASM" ]]; then
    err "Grammar WASM not found at $GRAMMAR_WASM"
    exit 1
fi
ok "Grammar compiled ($(du -h "$GRAMMAR_WASM" | cut -f1))"

# ── Step 4: Assemble extension directory ─────────────────────
info "Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR/languages/ion"
mkdir -p "$INSTALL_DIR/grammars"

# Extension manifest
cp "$ZED_EXT_DIR/extension.toml" "$INSTALL_DIR/"

# Compiled extension WASM
cp "$WASM_FILE" "$INSTALL_DIR/extension.wasm"

# Language configuration and queries
cp "$ZED_EXT_DIR/languages/ion/config.toml"    "$INSTALL_DIR/languages/ion/"
cp "$ZED_EXT_DIR/languages/ion/highlights.scm"  "$INSTALL_DIR/languages/ion/"
cp "$ZED_EXT_DIR/languages/ion/brackets.scm"    "$INSTALL_DIR/languages/ion/"
cp "$ZED_EXT_DIR/languages/ion/indents.scm"     "$INSTALL_DIR/languages/ion/"
cp "$ZED_EXT_DIR/languages/ion/outline.scm"     "$INSTALL_DIR/languages/ion/"

# Compiled grammar
cp "$GRAMMAR_WASM" "$INSTALL_DIR/grammars/ion.wasm"

ok "Extension installed"

# ── Summary ──────────────────────────────────────────────────
echo ""
info "Installation complete!"
echo "  Extension: $INSTALL_DIR/extension.wasm"
echo "  Grammar:   $INSTALL_DIR/grammars/ion.wasm"
echo "  Languages: $INSTALL_DIR/languages/ion/"
echo ""
echo "  Restart Zed to load the extension."
echo "  For LSP support, ensure ion-lsp is on your PATH:"
echo "    cargo install --path $REPO_ROOT/ion-lsp"

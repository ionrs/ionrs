#!/usr/bin/env bash
# Publish Ion crates to crates.io in dependency order.
#
# Usage:
#   ./publish.sh          # publish all crates
#   ./publish.sh --dry-run  # dry-run only
#
# Prerequisites:
#   cargo login <your-crates.io-token>

set -euo pipefail

DRY_RUN=""
if [[ "${1:-}" == "--dry-run" ]]; then
    DRY_RUN="--dry-run"
    echo "==> Dry run mode"
fi

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${BLUE}==>${NC} ${BOLD}$1${NC}"; }
ok()    { echo -e "${GREEN}  ✓${NC} $1"; }
err()   { echo -e "${RED}  ✗${NC} $1" >&2; }

# Verify clean working directory
if [[ -n "$(git status --porcelain -- '*.toml' '*.rs')" ]]; then
    err "Uncommitted changes in .toml or .rs files. Commit first."
    exit 1
fi

# Verify tests pass
info "Running tests..."
cargo test --quiet 2>&1
ok "All tests pass"

# Publish order: ion-derive → ion-core → ionlang-cli, ion-lsp
# Each crate must be available on crates.io before dependents can publish.

CRATES=("ion-derive" "ion-core" "ionlang-cli" "ion-lsp")
WAIT_SECS=30

for i in "${!CRATES[@]}"; do
    crate="${CRATES[$i]}"
    info "Publishing $crate..."

    if cargo publish -p "$crate" $DRY_RUN 2>&1; then
        ok "$crate published"
    else
        err "Failed to publish $crate"
        exit 1
    fi

    # Wait for crates.io index to update (skip after last crate and in dry-run)
    if [[ -z "$DRY_RUN" && $i -lt $(( ${#CRATES[@]} - 1 )) ]]; then
        info "Waiting ${WAIT_SECS}s for crates.io index..."
        sleep "$WAIT_SECS"
    fi
done

echo ""
info "All crates published!"
echo "  https://crates.io/crates/ion-derive"
echo "  https://crates.io/crates/ion-core"
echo "  https://crates.io/crates/ionlang-cli"
echo "  https://crates.io/crates/ion-lsp"
echo ""
echo "  Install: cargo install ionlang-cli"
echo "  LSP:     cargo install ion-lsp"

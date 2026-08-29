#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Black Wall Core — Full Workspace Build Script
#
# Usage:
#   ./scripts/build.sh [--release]
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="debug"
CARGO_FLAGS=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE="release"; CARGO_FLAGS="--release"; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

echo "╔══════════════════════════════════════════════╗"
echo "║       Black Wall Core — Build System       ║"
echo "╚══════════════════════════════════════════════╝"
echo ""
echo "Profile: ${PROFILE}"
echo ""

cd "${ROOT}"

# ── Step 1: Build the bare-metal userspace init ───────────────────────────────
echo "[ 1/4 ] Building userspace/init (x86_64-unknown-none)..."
cd "${ROOT}/userspace/init"
cargo build ${CARGO_FLAGS} --target x86_64-unknown-none
echo "  OK"
cd "${ROOT}"

# ── Step 2: Build the main workspace ─────────────────────────────────────────
echo "[ 2/4 ] Building main workspace..."
cargo build ${CARGO_FLAGS}
echo "  OK"

# ── Step 3: Build host-side tools ─────────────────────────────────────────────
echo "[ 3/4 ] Building host tools..."
for tool in package-manager/anx tools/firewall tools/updater tools/iso-builder \
            tools/package-builder tools/bwctl tools/cluster services/ssh services/cron \
            services/snapshot services/backup services/monitor \
            userspace/shell boot/init; do
    if [[ -f "${ROOT}/${tool}/Cargo.toml" ]]; then
        echo "  Building ${tool}..."
        cd "${ROOT}/${tool}"
        cargo build ${CARGO_FLAGS} 2>&1 | tail -2
        cd "${ROOT}"
    fi
done

# ── Step 4: Summary ───────────────────────────────────────────────────────────
echo ""
echo "[ 4/4 ] Build complete!"
echo ""
echo "Artifacts:"
find "${ROOT}/target/${PROFILE}" -maxdepth 1 -executable -type f 2>/dev/null | sort | sed 's/^/  /'
echo ""
echo "Run 'scripts/run-qemu.sh' to test in QEMU."
echo "Run 'scripts/build-iso.sh' to create a bootable ISO."

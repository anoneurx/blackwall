#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Black Wall Core — QEMU Test Runner
#
# Boots the Black Wall ISO in QEMU for development testing.
#
# Usage:
#   ./scripts/run-qemu.sh [--iso <path>] [--version VERSION] [--headless] [--debug] [--memory SIZE]
#
# Requirements:
#   sudo apt install qemu-system-x86 ovmf
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ISO=""
HEADLESS=false
DEBUG=false
MEMORY="512M"
VERSION=""
BIOS_PATH=""

# Find OVMF (UEFI firmware).
for path in \
    "/usr/share/OVMF/OVMF_CODE.fd" \
    "/usr/share/ovmf/OVMF.fd" \
    "/usr/share/edk2-ovmf/OVMF_CODE.fd"; do
    if [[ -f "${path}" ]]; then
        BIOS_PATH="${path}"
        break
    fi
done

while [[ $# -gt 0 ]]; do
    case "$1" in
        --iso) ISO="$2"; shift 2 ;;
        --version) VERSION="$2"; shift 2 ;;
        --headless) HEADLESS=true; shift ;;
        --debug) DEBUG=true; shift ;;
        --memory) MEMORY="$2"; shift 2 ;;
        --help|-h)
            echo "Usage: $0 [--iso <path>] [--version VERSION] [--headless] [--debug] [--memory SIZE]"
            echo ""
            echo "Options:"
            echo "  --iso PATH       Path to ISO file (default: auto-detect from dist/)"
            echo "  --version VER    Version to run (e.g. v2.0), used to find ISO automatically"
            echo "  --headless       Run without display (serial only)"
            echo "  --debug          Start paused for GDB debugging"
            echo "  --memory SIZE    Memory allocation (default: 512M)"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Auto-detect ISO from version or latest in dist/
if [[ -z "${ISO}" ]]; then
    if [[ -n "${VERSION}" ]]; then
        ISO="${ROOT}/dist/blackwall-server-${VERSION}.iso"
    else
        # Find the latest ISO
        ISO=$(ls -t "${ROOT}/dist/"blackwall-server-*.iso 2>/dev/null | head -1 || true)
        if [[ -z "${ISO}" ]]; then
            echo "No ISO found in ${ROOT}/dist/"
            echo "Build one first with: ./scripts/build-iso.sh --version v2.0"
            exit 1
        fi
        echo "Auto-detected ISO: ${ISO}"
    fi
fi

if [[ ! -f "${ISO}" ]]; then
    echo "ISO not found: ${ISO}"
    echo "Build it first with: ./scripts/build-iso.sh --version ${VERSION:-v2.0}"
    exit 1
fi

echo "╔══════════════════════════════════════════════╗"
echo "║      Black Wall Core — QEMU Runner         ║"
echo "╚══════════════════════════════════════════════╝"
echo ""
echo "  ISO    : ${ISO}"
echo "  Memory : ${MEMORY}"
echo "  SMP    : 2 cores"
echo "  UEFI   : ${BIOS_PATH:-'(no OVMF found — using legacy BIOS)'}"
echo "  Debug  : ${DEBUG}"
echo ""

QEMU_ARGS=(
    -m "${MEMORY}"
    -smp 2
    -cdrom "${ISO}"
    -boot d
    -serial stdio
    -net nic,model=virtio
    -net user
    -vga virtio
)

if [[ -n "${BIOS_PATH}" ]]; then
    QEMU_ARGS+=(-bios "${BIOS_PATH}")
fi

if [[ "${HEADLESS}" == "true" ]]; then
    QEMU_ARGS+=(-nographic)
else
    QEMU_ARGS+=(-display sdl)
fi

if [[ "${DEBUG}" == "true" ]]; then
    QEMU_ARGS+=(-s -S)
    echo "  GDB: connect with 'gdb -ex \"target remote :1234\"'"
    echo ""
fi

exec qemu-system-x86_64 "${QEMU_ARGS[@]}"

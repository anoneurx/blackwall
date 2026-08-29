#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Black Wall Core — Secure Boot MOK Enrollment
#
# Generates or enrolls a Machine Owner Key for Secure Boot signing.
# Must be run BEFORE building the release ISO.
#
# Usage:
#   sudo ./scripts/enroll-mok.sh [--generate] [--enroll]
#
# Requirements:
#   sudo apt install mokutil sbsigntool openssl
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
KEYS_DIR="${ROOT}/keys/secure-boot"

GENERATE=false
ENROLL=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --generate) GENERATE=true; shift ;;
        --enroll) ENROLL=true; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

mkdir -p "${KEYS_DIR}"

if [[ "${GENERATE}" == "true" ]]; then
    echo "Generating Machine Owner Key (MOK)..."

    openssl req -new -x509 \
        -newkey rsa:4096 \
        -keyout "${KEYS_DIR}/mok.key" \
        -out "${KEYS_DIR}/mok.crt" \
        -days 3650 \
        -nodes \
        -subj "/CN=Black Wall Core MOK/O=Black Wall Core/C=US"

    # Convert to DER for mokutil.
    openssl x509 \
        -in "${KEYS_DIR}/mok.crt" \
        -outform DER \
        -out "${KEYS_DIR}/mok.der"

    echo "Keys generated:"
    echo "  Private key : ${KEYS_DIR}/mok.key"
    echo "  Certificate : ${KEYS_DIR}/mok.crt"
    echo "  DER cert    : ${KEYS_DIR}/mok.der"
    echo ""
    echo "IMPORTANT: Keep mok.key secure. Do not commit it to version control."
fi

if [[ "${ENROLL}" == "true" ]]; then
    if [[ ! -f "${KEYS_DIR}/mok.der" ]]; then
        echo "ERROR: MOK not generated. Run with --generate first."
        exit 1
    fi
    echo "Enrolling MOK into firmware..."
    echo "(You will be prompted to set a password. Enter it on next reboot.)"
    mokutil --import "${KEYS_DIR}/mok.der"
    echo ""
    echo "Reboot the system to complete MOK enrollment."
fi

# Sign the bootloader if both flags are used together.
if [[ "${GENERATE}" == "true" ]] && [[ -f "${ROOT}/dist/blackwall-server-v1.0.iso" ]]; then
    echo "NOTE: Re-run 'scripts/build-iso.sh' to create a Secure Boot signed ISO."
fi

if [[ "${GENERATE}" == "false" ]] && [[ "${ENROLL}" == "false" ]]; then
    echo "Usage:"
    echo "  sudo ./scripts/enroll-mok.sh --generate   # Generate MOK key pair"
    echo "  sudo ./scripts/enroll-mok.sh --enroll     # Enroll into firmware"
    echo "  sudo ./scripts/enroll-mok.sh --generate --enroll  # Both"
fi

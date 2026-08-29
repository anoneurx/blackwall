#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Black Wall Core — ISO Builder
# Assembles a bootable ISO image from the compiled kernel + installer.
#
# Usage:
#   ./scripts/build-iso.sh [--release] [--version VERSION]
#
# Output:
#   dist/blackwall-server-<VERSION>.iso
#
# Versions from Publish.md:
#   v0.1  Alpha       — Bootable ISO, installer, basic shell (15-20 packages)
#   v0.5  Beta        — Functional package manager, networking, SSH (25-35 packages)
#   v1.0  Stable      — Production-ready core server (40-60 packages)
#   v1.5  Stable      — Expanded repositories and tooling (80-120 packages)
#   v2.0  Enterprise  — Enterprise services and virtualization (150-200 packages)
#   v2.5  Stable      — Cluster management, monitoring, automation (250-350 packages)
#   v3.0  Ecosystem   — Cloud-native platform (500+ packages)
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="${ROOT}/dist"
BUILD_DIR="${ROOT}/_isobuild"
PROFILE="debug"
VERSION=""

usage() {
    echo "Usage: $0 [--release] [--version VERSION]"
    echo ""
    echo "Versions from Publish.md:"
    echo "  v0.1  - Alpha: Bootable ISO, installer, basic shell (15-20 packages)"
    echo "  v0.5  - Beta: Functional package manager, networking, SSH (25-35 packages)"
    echo "  v1.0  - Stable: Production-ready core server (40-60 packages)"
    echo "  v1.5  - Stable: Expanded repositories and tooling (80-120 packages)"
    echo "  v2.0  - Enterprise: Enterprise services and virtualization (150-200 packages)"
    echo "  v2.5  - Stable: Cluster management, monitoring, automation (250-350 packages)"
    echo "  v3.0  - Ecosystem: Cloud-native platform (500+ packages)"
    exit 1
}

version_ge() {
    # Returns 0 if $1 >= $2 (simple string comparison for our known versions)
    local order=("v0.1" "v0.5" "v1.0" "v1.5" "v2.0" "v2.5" "v3.0")
    local idx1=-1 idx2=-1 i=0
    for v in "${order[@]}"; do
        [[ "$v" == "$1" ]] && idx1=$i
        [[ "$v" == "$2" ]] && idx2=$i
        ((i++))
    done
    (( idx1 >= idx2 ))
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE="release"; shift ;;
        --version) VERSION="$2"; shift 2 ;;
        --help|-h) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

if [[ -z "${VERSION}" ]]; then
    echo "ERROR: --version is required."
    usage
fi

echo "╔══════════════════════════════════════════════╗"
echo "║      Black Wall Core — ISO Builder         ║"
echo "╚══════════════════════════════════════════════╝"
echo ""
echo "Version  : ${VERSION}"
echo "Profile  : ${PROFILE}"
echo "Root     : ${ROOT}"
echo ""

# ── Dependency checks ─────────────────────────────────────────────────────────
for cmd in xorriso grub-mkrescue; do
    if ! command -v "${cmd}" &>/dev/null; then
        echo "ERROR: '${cmd}' is required but not found."
        echo "  Install with: sudo apt install ${cmd}"
        exit 1
    fi
done

# ── Step 1: Build the workspace ──────────────────────────────────────────────
echo "[ 1/7 ] Building workspace..."
cd "${ROOT}"

if [[ "${PROFILE}" == "release" ]]; then
    cargo build --release 2>&1 | tail -5
else
    cargo build 2>&1 | tail -5
fi

# ── Step 2: Build userspace init (bare-metal target) ─────────────────────────
echo "[ 2/7 ] Building userspace init (x86_64-unknown-none)..."
cd "${ROOT}/userspace/init"
if [[ "${PROFILE}" == "release" ]]; then
    cargo build --release --target x86_64-unknown-none 2>&1 | tail -3
else
    cargo build --target x86_64-unknown-none 2>&1 | tail -3
fi
cd "${ROOT}"

# ── Step 3: Assemble ISO tree ────────────────────────────────────────────────
echo "[ 3/7 ] Assembling ISO tree..."
rm -rf "${BUILD_DIR}"
mkdir -p "${BUILD_DIR}/boot/grub"
mkdir -p "${BUILD_DIR}/EFI/BOOT"
mkdir -p "${BUILD_DIR}/blackwall/bin"
mkdir -p "${BUILD_DIR}/blackwall/lib"
mkdir -p "${BUILD_DIR}/blackwall/etc/systemd"
mkdir -p "${BUILD_DIR}/blackwall/etc/bwfw"
mkdir -p "${BUILD_DIR}/blackwall/etc/anx"
mkdir -p "${BUILD_DIR}/blackwall/repo/enterprise"

# ── Kernel + init ────────────────────────────────────────────────────────────
echo "  Copying kernel and init..."
KERNEL_BIN="${ROOT}/target/${PROFILE}/blackwall-kernel"
if [[ -f "${KERNEL_BIN}" ]]; then
    cp "${KERNEL_BIN}" "${BUILD_DIR}/blackwall/bin/kernel"
    echo "  Copied blackwall-kernel → blackwall/bin/kernel"
else
    echo "  WARNING: Kernel binary not found at ${KERNEL_BIN}"
fi

BOOTINIT="${ROOT}/userspace/init/target/x86_64-unknown-none/${PROFILE}/init"
if [[ -f "${BOOTINIT}" ]]; then
    cp "${BOOTINIT}" "${BUILD_DIR}/blackwall/bin/init"
    echo "  Copied userspace init → blackwall/bin/init"
fi

# ── UEFI bootloader ──────────────────────────────────────────────────────────
BOOTLOADER_EFI="${ROOT}/target/x86_64-unknown-uefi/${PROFILE}/blackwall-bootloader.efi"
if [[ -f "${BOOTLOADER_EFI}" ]]; then
    cp "${BOOTLOADER_EFI}" "${BUILD_DIR}/EFI/BOOT/BOOTX64.EFI"
    echo "  Copied UEFI bootloader → EFI/BOOT/BOOTX64.EFI"
else
    echo "  WARNING: UEFI bootloader not found (expected at ${BOOTLOADER_EFI})"
fi

# ── Installer ────────────────────────────────────────────────────────────────
INSTALLER_BIN="${ROOT}/target/${PROFILE}/blackwall-installer"
if [[ -f "${INSTALLER_BIN}" ]]; then
    cp "${INSTALLER_BIN}" "${BUILD_DIR}/blackwall/bin/installer"
    echo "  Copied installer → blackwall/bin/installer"
fi

# ── Step 4: Copy binaries by version tier ────────────────────────────────────
echo "[ 4/7 ] Copying binaries for ${VERSION}..."

# v0.1+: Core — kernel, installer, shell, init, anx, login, coreutils
CORE_BINS="anx bwsh bwinit bwlogin"
CORE_UTILS="ls cat cp mv rm mkdir rmdir touch chmod chown echo find grep head tail wc sort uniq ps kill id whoami hostname date df du uname pwd"

for bin in ${CORE_BINS}; do
    if [[ -f "${ROOT}/target/${PROFILE}/${bin}" ]]; then
        cp "${ROOT}/target/${PROFILE}/${bin}" "${BUILD_DIR}/blackwall/bin/"
        echo "  Copied ${bin}"
    else
        echo "  WARNING: ${bin} not found"
    fi
done

for cmd in ${CORE_UTILS}; do
    if [[ -f "${ROOT}/target/${PROFILE}/${cmd}" ]]; then
        cp "${ROOT}/target/${PROFILE}/${cmd}" "${BUILD_DIR}/blackwall/bin/"
    fi
done
echo "  Copied coreutils (25 tools)"

# v0.5+: Networking, SSH, cron, updater, firewall
V05_BINS="bwssh bwcron anxd bwfw"
if version_ge "${VERSION}" "v0.5"; then
    for bin in ${V05_BINS}; do
        if [[ -f "${ROOT}/target/${PROFILE}/${bin}" ]]; then
            cp "${ROOT}/target/${PROFILE}/${bin}" "${BUILD_DIR}/blackwall/bin/"
            echo "  Copied ${bin}"
        else
            echo "  WARNING: ${bin} not found"
        fi
    done
fi

# v1.0+: Package builder
V10_BINS="bw-pkg"
if version_ge "${VERSION}" "v1.0"; then
    for bin in ${V10_BINS}; do
        if [[ -f "${ROOT}/target/${PROFILE}/${bin}" ]]; then
            cp "${ROOT}/target/${PROFILE}/${bin}" "${BUILD_DIR}/blackwall/bin/"
            echo "  Copied ${bin}"
        fi
    done
fi

# v1.5+: Repo server
V15_BINS="anx-repo-server"
if version_ge "${VERSION}" "v1.5"; then
    for bin in ${V15_BINS}; do
        if [[ -f "${ROOT}/target/${PROFILE}/${bin}" ]]; then
            cp "${ROOT}/target/${PROFILE}/${bin}" "${BUILD_DIR}/blackwall/bin/"
            echo "  Copied ${bin}"
        fi
    done
fi

# v2.0+: Enterprise — API, monitor, snapshot, backup, cluster, bwctl
V20_BINS="bw-api bwmonitor bwsnap bwbackup bwcluster bwctl"
if version_ge "${VERSION}" "v2.0"; then
    for bin in ${V20_BINS}; do
        if [[ -f "${ROOT}/target/${PROFILE}/${bin}" ]]; then
            cp "${ROOT}/target/${PROFILE}/${bin}" "${BUILD_DIR}/blackwall/bin/"
            echo "  Copied ${bin}"
        else
            echo "  WARNING: ${bin} not found"
        fi
    done
fi

# ── Step 5: Config files + systemd units ─────────────────────────────────────
echo "[ 5/7 ] Installing default configs and service units..."

# Default configs (always included)
if [[ -f "${ROOT}/assets/defaults/etc/bwfw/rules.toml" ]]; then
    cp "${ROOT}/assets/defaults/etc/bwfw/rules.toml" "${BUILD_DIR}/blackwall/etc/bwfw/rules.toml"
    echo "  Copied bwfw default rules"
fi
if [[ -f "${ROOT}/assets/defaults/etc/anx/anx.toml" ]]; then
    cp "${ROOT}/assets/defaults/etc/anx/anx.toml" "${BUILD_DIR}/blackwall/etc/anx/anx.toml"
    echo "  Copied anx default config"
fi
if [[ -f "${ROOT}/assets/defaults/etc/anx/update.toml" ]]; then
    cp "${ROOT}/assets/defaults/etc/anx/update.toml" "${BUILD_DIR}/blackwall/etc/anx/update.toml"
    echo "  Copied update daemon config"
fi

# Systemd units — version gated
install_unit() {
    local src="${ROOT}/installer/systemd/$1"
    if [[ -f "${src}" ]]; then
        cp "${src}" "${BUILD_DIR}/blackwall/etc/systemd/$1"
        echo "  Installed $1"
    fi
}

# v0.5+: Firewall, updater, SSH, cron services
if version_ge "${VERSION}" "v0.5"; then
    install_unit bwfw.service
    install_unit anxd.service
    install_unit bwssh.service
    install_unit bwcron.service
fi

# v1.0+: Monitor service
if version_ge "${VERSION}" "v1.0"; then
    install_unit bwmonitor.service
fi

# v2.0+: Snapshot, backup, API, cluster
if version_ge "${VERSION}" "v2.0"; then
    install_unit bwsnap.service
    install_unit bwsnap.timer
    install_unit bwbackup.service
    install_unit bwbackup.timer
    install_unit bw-api.service
    install_unit bwcluster.service
fi

# ── Package repository ───────────────────────────────────────────────────────
echo "  Configuring package repository..."

if [[ -f "${ROOT}/package-manager/server/index.toml" ]]; then
    cp "${ROOT}/package-manager/server/index.toml" "${BUILD_DIR}/blackwall/repo/index.toml"
    echo "  Copied repo index → blackwall/repo/index.toml"
fi

# All versions: include the version's own milestone tier
for tier in v0.1 v0.5; do
    if [[ -d "${ROOT}/packages/${tier}" ]] && version_ge "${VERSION}" "${tier}"; then
        mkdir -p "${BUILD_DIR}/blackwall/repo/${tier}"
        cp -r "${ROOT}/packages/${tier}/." "${BUILD_DIR}/blackwall/repo/${tier}/"
        echo "  Copied ${tier} packages → blackwall/repo/${tier}/"
    fi
done

# v1.0+: Include core package recipes (system, networking, security, dev, admin)
if version_ge "${VERSION}" "v1.0"; then
    for tier in system networking security development administration; do
        if [[ -d "${ROOT}/packages/${tier}" ]]; then
            mkdir -p "${BUILD_DIR}/blackwall/repo/${tier}"
            cp -r "${ROOT}/packages/${tier}/." "${BUILD_DIR}/blackwall/repo/${tier}/"
            echo "  Copied ${tier} packages → blackwall/repo/${tier}/"
        fi
    done
fi

# v1.5+: Include expanded package recipes
if version_ge "${VERSION}" "v1.5"; then
    if [[ -d "${ROOT}/packages/v1.5" ]]; then
        mkdir -p "${BUILD_DIR}/blackwall/repo/expanded"
        cp -r "${ROOT}/packages/v1.5/." "${BUILD_DIR}/blackwall/repo/expanded/"
        echo "  Copied v1.5 expanded packages → blackwall/repo/expanded/"
    fi
fi

# v2.0+: Include enterprise package recipes
if version_ge "${VERSION}" "v2.0"; then
    for tier in containers databases web virtualization monitoring automation security languages; do
        if [[ -d "${ROOT}/packages/${tier}" ]]; then
            mkdir -p "${BUILD_DIR}/blackwall/repo/enterprise/${tier}"
            cp -r "${ROOT}/packages/${tier}/." "${BUILD_DIR}/blackwall/repo/enterprise/${tier}/"
            echo "  Copied enterprise/${tier} packages"
        fi
    done
fi

# v3.0+: Include ecosystem package recipes (cloud, AI, desktop, storage, devops)
if version_ge "${VERSION}" "v3.0"; then
    if [[ -d "${ROOT}/packages/v3.0" ]]; then
        mkdir -p "${BUILD_DIR}/blackwall/repo/ecosystem"
        cp -r "${ROOT}/packages/v3.0/." "${BUILD_DIR}/blackwall/repo/ecosystem/"
        echo "  Copied v3.0 ecosystem packages → blackwall/repo/ecosystem/"
    fi
fi

# ── Step 6: GRUB config ─────────────────────────────────────────────────────
echo "[ 6/7 ] Writing GRUB config..."

cat > "${BUILD_DIR}/boot/grub/grub.cfg" << EOF
set timeout=5
set default=0

menuentry "Black Wall Core ${VERSION}" {
    echo "Loading Black Wall Core ${VERSION}..."
    insmod all_video
    terminal_output console
    chainloader /EFI/BOOT/BOOTX64.EFI
    boot
}

menuentry "Black Wall Core ${VERSION} (Recovery)" {
    echo "Loading Recovery Console..."
    chainloader /EFI/BOOT/BOOTX64.EFI
    boot
}

menuentry "Black Wall Core ${VERSION} (Serial Console)" {
    echo "Loading Black Wall Core ${VERSION} on serial..."
    serial --speed=115200 --unit=0 --word=8 --parity=no --stop=1
    terminal_input serial console
    terminal_output serial console
    chainloader /EFI/BOOT/BOOTX64.EFI
    boot
}
EOF

# ── Step 7: Create ISO ──────────────────────────────────────────────────────
echo "[ 7/7 ] Creating ISO image..."
mkdir -p "${DIST}"
ISO_PATH="${DIST}/blackwall-server-${VERSION}.iso"

grub-mkrescue \
    --output="${ISO_PATH}" \
    "${BUILD_DIR}" \
    -- \
    -volid "BLACKWALL_$(echo "${VERSION}" | tr '.' '_' | tr 'v' 'V')" \
    2>&1 | grep -v "^$" || true

if [[ -f "${ISO_PATH}" ]]; then
    SIZE=$(du -sh "${ISO_PATH}" | cut -f1)
    echo ""
    echo "Computing checksum..."
    SHA=$(sha256sum "${ISO_PATH}" | cut -d' ' -f1)
    echo "SHA256: ${SHA}" > "${ISO_PATH}.sha256"

    echo ""
    echo "╔══════════════════════════════════════════════╗"
    echo "║              Build Complete!                  ║"
    echo "╚══════════════════════════════════════════════╝"
    echo ""
    echo "  ISO     : ${ISO_PATH}"
    echo "  Version : ${VERSION}"
    echo "  Size    : ${SIZE}"
    echo "  SHA256  : ${SHA}"
    echo ""
    echo "  Contents:"
    echo "    blackwall/bin/     $(ls "${BUILD_DIR}/blackwall/bin/" 2>/dev/null | wc -l) binaries"
    echo "    blackwall/etc/     configs + systemd units"
    echo "    blackwall/repo/    package repository"
    echo "    EFI/BOOT/          UEFI bootloader"
    echo ""
    echo "  To test in QEMU:"
    echo "    ./scripts/run-qemu.sh --iso ${ISO_PATH}"
else
    echo "ERROR: ISO creation failed — grub-mkrescue did not produce output."
    exit 1
fi

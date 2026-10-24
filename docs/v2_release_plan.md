# Black Wall v2.0.0 — Release Plan (Enterprise Server)

Created 2026-10-24 to resolve the reference in `CHANGELOG.md` ([Unreleased] v2.0.0).

Status: PLANNING — nothing in this file is implemented yet unless noted.

## Phase V2-B — Enterprise Packages [PARTIAL, done]
- `tools/package-builder` (`bw-pkg`) — done
- `packages/` recipe tree — 27 recipes across 8 categories — done
- Recipe features: dir/prebuilt staging, tarball sources w/ SHA-256, optional
  top-level flattening, `bwinit` service units, install/remove hooks — done

## Planned phases (order tentative)
1. Server API (`api/`) — authenticated REST administration API
   - Sessions/tokens, read-only endpoints first (system info, services, packages)
   - HTTPS-only, token auth, no telemetry, rate-limited
2. Snapshot Manager — FS-level snapshots on the VFS/Ext2 or block layer
3. Automatic Rollback — boot-time rollback selection, depends on snapshots
4. Backup Manager — scheduled + on-demand backup jobs, restore verification
5. Cluster Tools — multi-node administration (design doc first)
6. Performance Dashboard — depends on tracing counters (Phase 4 groundwork)
7. Web Repository — browsing UI over `anx-repo-server`

## Prerequisites / dependencies on core
- Real mmap/VMA tracking (Phase 2) before snapshot restore over active mappings
- Panic backtraces + kernel test harness (Phase 1.4) before hardening the API
- SMEP/SMAP/NX enforcement (Phase 3) before exposing any privileged network service

## Honest constraints
- The kernel boots a single RamFS/Ext2 world; enterprise recipes target the
  host-side toolchain and are staged payloads — none are tested to run inside
  the kernel's userland yet.

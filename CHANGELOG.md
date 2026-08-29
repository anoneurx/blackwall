# Black Wall Core — CHANGELOG

All notable changes to Black Wall Core are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

---

## [Unreleased] — v2.0.0 (Enterprise Server)

### Added — Phase V2-B (Enterprise Packages)
- `tools/package-builder` (`bw-pkg`): builds `.anxpkg` archives from declarative
  `recipe.toml` files, generates the repository `index.toml`, and signs releases
- `packages/` recipe tree — 27 curated enterprise recipes across 8 categories:
  containers (docker, podman, containerd), databases (postgresql, mariadb, mysql,
  mongodb, redis), web (nginx, apache, caddy), virtualization (qemu, kvm, libvirt),
  monitoring (prometheus, grafana, netdata), automation (ansible, terraform, opentofu),
  security (clamav, wireguard, openvpn), languages (nodejs, go, java, php)
- Recipes support `dir`/`prebuilt` payload staging, `tarball` sources with SHA-256
  verification and optional top-level flattening, `bwinit` service units, and
  post-install/pre-remove hooks

### Planned (see `docs/v2_release_plan.md`)
- Server API (`api/`) — authenticated REST administration API
- Snapshot Manager, Automatic Rollback, Backup Manager, Cluster Tools
- Performance Dashboard, Web Repository, Enterprise Repository

---

## [v1.0.0] — Core Server

### Added

#### Package Manager (`anx`)
- Full `anx` CLI: `install`, `remove`, `update`, `refresh`, `search`, `list`, `info`, `rollback`, `list-transactions`, `key`
- `.anxpkg` format: zstd-compressed tar with `MANIFEST.toml`, `files/`, `sig.gpg`
- SHA-256 checksum verification against repository index
- GPG signature verification via system `gpg` binary
- Transaction log in `/var/lib/anx/transactions/` (TOML, timestamped)
- Rollback support: `anx rollback <tx-id>` removes installed files
- Keyring management: `anx key add/list/remove`
- Process-level lockfile at `/var/lib/anx/lock`
- `anxd` automatic update daemon with configurable interval

#### Init System (`bwinit`)
- PID 1 init daemon: mounts `/proc`, `/sys`, `/dev`, `/run`, `/dev/pts`
- Sets hostname from `/etc/hostname`
- Starts and supervises system services: firewall, sshd, cron, anxd
- Automatic service restart on exit (configurable)
- Zombie process reaper loop

#### Shell (`bwsh`)
- Full interactive shell with coloured prompt (`user@host:cwd $`)
- Built-ins: `cd`, `echo`, `pwd`, `export`, `unset`, `alias`, `history`, `source`, `clear`, `help`, `exit`
- Variable expansion (`$VAR`, `$?`)
- Quote handling (single and double)
- Pipeline support (`cmd1 | cmd2`)
- Script file execution (`bwsh script.sh`)
- Sources `/etc/bwshrc` and `~/.bwshrc` on startup
- Shell commands module: I/O redirection (`<`, `>`, `>>`) and pipeline executor
- Shell builtins module with `is_builtin()` predicate

#### Coreutils (`bwcoreutils`)
25 utilities implemented:
`ls`, `cat`, `cp`, `mv`, `rm`, `mkdir`, `rmdir`, `touch`, `echo`, `pwd`,
`hostname`, `uname`, `whoami`, `id`, `ps`, `kill`, `grep`, `find`,
`head`, `tail`, `wc`, `df`, `du`, `date`, `sort`, `uniq`, `chmod`, `chown`

#### Firewall (`bwfw`)
- Userspace firewall CLI backed by `net/src/firewall.rs`
- Persistent rules in `/etc/bwfw/rules.toml`
- Default policy: deny-all inbound, allow SSH (TCP/22) and ICMP
- Commands: `list`, `status`, `enable`, `disable`, `add`, `del`, `reset`, `apply`

#### SSH Service (`bwssh`)
- Generates RSA-4096 and Ed25519 host keys if missing
- Writes hardened `sshd_config` (root login via key only, X11 off, verbose logging)
- Writes `/etc/motd` with Black Wall Core banner
- Exec()s `sshd -D` for supervisor-compatible foreground operation

#### Cron Service (`bwcron`)
- Reads `/etc/crontab` and `/etc/cron.d/*.cron`
- Standard 5-field cron syntax with `*`, exact, list, range, and step (`*/n`) fields
- Reloads crontab every minute (live edits supported)
- Runs jobs in background threads

#### Login Process (`bwlogin`)
- Displays `/etc/motd`
- Prompts for username + password (echo disabled)
- Authenticates against `/etc/passwd` + `/etc/shadow` (SHA-512 via openssl)
- Locks out after 3 failed attempts
- Sets user environment (`USER`, `HOME`, `PATH`, `SHELL`) and execs `bwsh`

#### Build System
- `scripts/build.sh` — full workspace orchestration
- `scripts/build-iso.sh` — bootable ISO assembly
- `scripts/run-qemu.sh` — QEMU test runner (with OVMF UEFI support)
- `scripts/enroll-mok.sh` — Secure Boot MOK generation and enrollment
- `tools/bw-iso-builder` — Rust ISO builder tool (Rust alternative to shell script)

#### Default Configurations
- `/etc/anx/anx.toml` — package manager config (repo URL)
- `/etc/anx/update.toml` — anxd update daemon config
- `/etc/bwfw/rules.toml` — firewall rules (deny-all / allow SSH)

### Changed
- `Cargo.toml`: workspace now includes all v1.0 host-side crates
- Removed `rusqlite` dependency from `anx` (using flat TOML storage)
- Removed `gpgme` C-library dependency (using system `gpg` binary instead)

### Deferred to v2.0
- Docker / Podman / containerd
- PostgreSQL / MariaDB / Redis
- Nginx / Apache / Caddy
- QEMU / KVM / Libvirt
- Prometheus / Grafana / Netdata
- Ansible / Terraform
- Web repository UI
- Server API / REST API

---

## [0.1.0] — Initial Development

- Black Wall Kernel (memory, process, scheduler, drivers, security, SMP, IPC, net, fs)
- UEFI bootloader loading kernel via boot services
- Installer (Slint GUI: 18 screens) + backend (disk manager, partitioner, installer logic)
- Network stack (TCP, UDP, IPv4, IPv6, ARP, DNS, ICMP, firewall kernel module)
- Filesystem layer (VFS + Ext2)
- Userspace init ELF (bare-metal, x86_64-unknown-none)

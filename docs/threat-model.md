# Threat Model (Draft)

## Scope
Black Wall Core x86_64 UEFI OS targeting server/embedded use cases.

## Assets
- Kernel code/data
- User processes and memory
- Filesystem (RamFS/Ext2)
- Network stack
- Package manager (anx)
- Services (ssh/cron/...)

## Threats
- Privilege escalation via syscalls
- Memory corruption (use-after-free, buffer overflow)
- TOCTOU in filesystem/syscall paths
- Malicious ELF loading
- Network exploits
- Supply chain (packages)

## Mitigations (Planned/In Progress)
- Syscall allow-list + per-process policies
- `access_ok` + bounds checking
- W^X, NX, SMEP/SMAP
- Capability system
- Address space isolation
- Secure boot / package signing
- Fuzzing

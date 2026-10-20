# Security Policy

## Reporting a Vulnerability
If you discover a security vulnerability in Black Wall Core, please report it responsibly.

- **Email**: security@blackwall.example.org (replace with actual contact)
- **Do not** open public GitHub issues for security vulnerabilities
- **Include**: description, reproduction steps, affected versions, potential impact

We aim to acknowledge reports within 48 hours and provide updates as we investigate.

## Security Features (Status)
- Syscall allow-list per process (in progress)
- User pointer validation via `access_ok`
- `PAGE_OWNED` reclamation to prevent leaks
- NX bit support, W^X enforcement in progress
- SMEP/SMAP/KASLR planned
- Stack canaries planned
- CSPRNG from RDRAND/RDSEED/virtio-rng planned

## Hardening Guidelines
- All `unsafe` blocks must have `// SAFETY:` comments
- Never trust user pointers; use `copy_from_user`/`copy_to_user`
- Enforce least privilege via capabilities

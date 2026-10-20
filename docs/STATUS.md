# Blackwall OS — Audit Status (Updated)

## Overall Status
- Kernel builds cleanly (no warnings) and the ISO boots through all init phases.
- Userspace shell (`blackwall>`) reached over the OVMF/QEMU serial console;
  interactive input (echo, backspace, command dispatch) verified.
- Full process lifecycle works end to end: `fork` + `execve` + `waitpid`.
- Address spaces are fully reclaimed on `execve` and process exit — free-frame
  count is stable across repeated run/reap cycles (no leak).

## Verified Working (QEMU 11.1, KVM, OVMF 4M, `-serial stdio -sandbox on`)
- Boot to `blackwall>` shell through all init phases.
- Shell builtins: `echo`, `help`, `ls`, `pwd`, `clear`, input echo/backspace.
- `run <path>`:
  - `run /bin/init` forks, child `execve`s `/bin/init`, prints its own banner
    and prompt, runs commands, then `exit`;
  - the parent blocks in `waitpid`, is woken by the scheduler, reaps the child
    and prints `run: pid N exited with status S`, then resumes its own prompt;
  - a bad path (`run /nope`) prints `run: execve failed` and yields status 127
    to the waiting parent.
- Build scripts/Makefile fixed (`ROOT` resolution, headless mode, ISO fallback
  to the pure-Rust `bw-iso-builder` when `xorriso`/`grub-mkrescue` are absent).

## Milestone 1: Userspace Input — DONE
- Shell input works end-to-end under `-serial stdio` + KVM/OVMF; commands are
  read via the 16550 UART driver and dispatched.
- `read_line` performs echo and backspace handling.

## Milestone 2: Process Lifecycle — DONE
- `sys_fork`: eagerly clones the address space; kernel mappings are shared,
  user pages privatised; child resumes from a copied trap frame with `rax=0`.
- `sys_execve`: loads the ELF into a fresh address space, switches `CR3` and
  rewrites the live trap frame to enter the new image.
- `sys_waitpid`: fast path reaps an already-exited child, otherwise blocks the
  caller; the scheduler completes the wait when a matching child exits.
- Fixes landed this session:
  - `clone_address_space`/`create_address_space` now classify a subtree as
    "user" via a bounded window plus a read-only user-page predicate, instead
    of a naive range test.  This stops `fork` from descending into firmware
    MMIO mappings (e.g. at 512 GiB) and hitting malformed table entries — the
    root cause of the `#GP (vector 13)` crash on first `fork`.
  - `sys_waitpid` released the scheduler lock before `block_current` (was a
    self-deadlock).
  - `wake_sleeping` now reaps the exited child, writes the exit status into the
    parent's address space (switching `CR3` for the write) and returns the
    child PID in the parent's `rax` — previously `waitpid` returned 0/status 0.
  - **Address-space reclamation (execve/exit leak closed).**  Every frame a
    process allocates is marked `PAGE_OWNED` (AVL bit 9) and shared kernel
    tables are copy-on-write: `get_or_alloc_entry` never mutates a shared
    table in place, `clone_space`/`share_kernel_root` clear the bit on share
    branches, and the init image shares the permanent boot root instead of
    deep-cloning it.  `destroy_address_space` walks the tree and frees only
    OWNED frames (refusing the boot root, captured as `BOOT_CR3`); it runs
    after the `CR3` switch in `sys_execve` and in `free_stack` on reap.
    Verified in QEMU: free-frame count returns to exactly the same value
    after every fork → exec → exit → reap cycle.

## Key Files Modified
- `kernel/src/process/loader.rs` — `clone_address_space`, `create_address_space`,
  `clone_space`, `has_user_page`, `USER_IMAGE_TOP` user-range classification;
  `PAGE_OWNED` ownership: `shared_entry`, copy-on-write `get_or_alloc_entry`,
  `share_kernel_root`, `destroy_address_space`/`free_owned_table`;
  `[DBG]` loader prints gated behind `LOADER_DEBUG`.
- `kernel/src/syscall/process.rs` — `sys_fork`, `sys_execve` (destroys the old
  address space after the `CR3` switch), `sys_waitpid` and the scheduler-lock
  lifetime fix.
- `kernel/src/scheduler/round_robin.rs` — `wake_sleeping` child reaping/waiter
  completion.
- `kernel/src/process/pcb.rs` — `wait_child`, `wait_status_ptr`.
- `kernel/src/process/manager.rs` — `remove(pid)`; `free_stack` also destroys
  the task's address space.
- `kernel/src/memory/paging.rs` — `PAGE_OWNED`.
- `kernel/src/memory/physical.rs` — `PhysFrame::containing`.
- `kernel/src/memory/mod.rs`, `kernel/src/init.rs` — published `MEMORY` global
  and `BOOT_CR3`.
- `kernel/src/security/mod.rs`, `kernel/src/syscall/{mod,fs}.rs` — syscall
  allow-list and dispatch.
- `shared/shared/src/lib.rs` — `ECHILD`.
- `userspace/init/src/main.rs` — `builtin_run` (fork/execve/wait).
- `build/scripts/*`, `build/Makefile`; `docs/build.md` corrected
  (`--output` flag, no xorriso/grub needed, OVMF pflash QEMU invocation).

## Next Steps
1. Subsystem audits: filesystem benchmarks, network fuzzing, security review.
2. Copy-on-write fork (fork currently copies user pages eagerly).

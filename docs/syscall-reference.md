# Syscall Reference (x86_64)

| Number | Name | Description |
|--------|------|-------------|
| 0 | read | Read from fd |
| 1 | write | Write to fd |
| 2 | open | Open file |
| 3 | close | Close fd |
| 4 | stat | Get file stat |
| 8 | lseek | Seek |
| 9 | mmap | Map memory (stub) |
| 11 | munmap | Unmap memory (stub) |
| 24 | yield | Yield CPU |
| 35 | sleep | Sleep |
| 39 | getpid | Get PID |
| 57 | fork | Fork process |
| 59 | execve | Exec program |
| 60 | exit | Exit process |
| 61 | waitpid | Wait for child |
| 78 | readdir | Read directory |
| 83 | mkdir | Make directory |
| 84 | rmdir | Remove directory |
| 87 | unlink | Unlink file |
| 110 | getppid | Get parent PID |
| 300 | net | Network control (blackwall-specific) |

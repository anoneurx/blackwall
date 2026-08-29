//! Package signature and checksum verification.
//!
//! Every `.anxpkg` is verified against:
//! 1. SHA-256 checksum (from the repository index)
//! 2. GPG detached signature (`sig.gpg` inside the archive) — optional in v1.0
//!    but enabled by default when trusted keys are present.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

// ─── SHA-256 verification ─────────────────────────────────────────────────────

/// Compute SHA-256 of a file and return the hex digest.
pub fn sha256_file(path: &Path) -> Result<String> {
    let data = std::fs::read(path)
        .with_context(|| format!("Failed to read file for hashing: {}", path.display()))?;
    Ok(sha256_bytes(&data))
}

/// Compute SHA-256 of a byte slice and return the hex digest.
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Verify a file's SHA-256 checksum against an expected hex string.
/// Returns `Ok(())` on match, `Err` on mismatch or read error.
pub fn verify_checksum(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    if actual != expected {
        anyhow::bail!(
            "Checksum mismatch for {}:\n  expected: {}\n  got:      {}",
            path.display(),
            expected,
            actual
        );
    }
    Ok(())
}

// ─── GPG verification (feature-gated) ────────────────────────────────────────

/// Verify a GPG detached signature.
///
/// `data_path`  — the file being verified  
/// `sig_path`   — the detached `.gpg` signature file  
/// `keyring_dir`— directory containing trusted public keys
///
/// In v1.0 this is a best-effort check. If no trusted keys are enrolled the
/// function logs a warning and returns `Ok(())`. If keys ARE enrolled, the
/// signature MUST pass.
pub fn verify_gpg(data_path: &Path, sig_path: &Path, keyring_dir: &Path) -> Result<()> {
    // Check whether any keys are enrolled.
    let key_files: Vec<_> = std::fs::read_dir(keyring_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |x| x == "gpg" || x == "asc"))
                .collect()
        })
        .unwrap_or_default();

    if key_files.is_empty() {
        // No keys enrolled — allow but warn.
        eprintln!(
            "\x1b[33m[WARN]\x1b[0m No trusted keys enrolled; skipping GPG verification. \
             Run 'anx key add <keyfile>' to enable signature checking."
        );
        return Ok(());
    }

    if !sig_path.exists() {
        anyhow::bail!(
            "Package is missing signature file '{}'. \
             Trusted keys are enrolled — refusing unsigned package.",
            sig_path.display()
        );
    }

    // Build a temporary GNUPGHOME, import every trusted key, then verify.
    // Importing handles both armored (`.asc`) and binary (`.gpg`) keys, which
    // gpg's deprecated `--keyring` option does not.
    let tmp_home = std::env::temp_dir().join(format!("bw-anx-gpg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_home);
    std::fs::create_dir_all(&tmp_home).context("failed to create temporary GNUPGHOME")?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp_home, std::fs::Permissions::from_mode(0o700));
    }

    let key_paths: Vec<String> = key_files.iter().map(|k| k.path().display().to_string()).collect();
    let import = std::process::Command::new("gpg")
        .env("GNUPGHOME", &tmp_home)
        .arg("--batch")
        .arg("--import")
        .args(&key_paths)
        .output()
        .context("gpg binary not found; cannot verify package signature")?;

    if !import.status.success() {
        let _ = std::fs::remove_dir_all(&tmp_home);
        let stderr = String::from_utf8_lossy(&import.stderr);
        anyhow::bail!("failed to import trusted keys:\n{}", stderr);
    }

    let verify = std::process::Command::new("gpg")
        .env("GNUPGHOME", &tmp_home)
        .arg("--verify")
        .arg(sig_path)
        .arg(data_path)
        .output()
        .context("gpg binary not found; cannot verify package signature")?;

    let _ = std::fs::remove_dir_all(&tmp_home);

    if !verify.status.success() {
        let stderr = String::from_utf8_lossy(&verify.stderr);
        anyhow::bail!("GPG signature verification FAILED:\n{}", stderr);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Generate a throwaway RSA test signing key inside `home`.
    fn gen_test_key(home: &Path) {
        let batch = format!(
            "Key-Type: RSA\nKey-Length: 2048\nName-Real: bw-test\nName-Email: test@bw.local\n\
             Expire-Date: 0\n%no-protection\n%commit\n"
        );
        let batch_file = home.join("keybatch");
        std::fs::write(&batch_file, batch).unwrap();
        let status = Command::new("gpg")
            .env("GNUPGHOME", home)
            .args(["--batch", "--gen-key", batch_file.to_str().unwrap()])
            .status()
            .expect("keygen failed");
        assert!(status.success(), "GPG key generation failed");
    }

    fn gpg_sign(home: &Path, data: &Path, sig: &Path) {
        let status = Command::new("gpg")
            .env("GNUPGHOME", home)
            .args(["--batch", "--yes", "--pinentry-mode", "loopback", "--passphrase", ""])
            .arg("--detach-sign")
            .arg("--output")
            .arg(sig)
            .arg(data)
            .status()
            .expect("signing failed");
        assert!(status.success(), "GPG signing failed");
    }

    fn export_pubkey(home: &Path, dest: &Path) {
        let out = Command::new("gpg")
            .env("GNUPGHOME", home)
            .arg("--armor")
            .arg("--export")
            .output()
            .expect("export failed");
        assert!(out.status.success());
        std::fs::write(dest, &out.stdout).unwrap();
    }

    #[test]
    fn checksum_verifies_and_rejects() {
        let dir = std::env::temp_dir().join("bw-anx-verify-checksum");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("pkg.bin");
        std::fs::write(&f, b"hello black wall").unwrap();

        let good = sha256_file(&f).unwrap();
        assert!(verify_checksum(&f, &good).is_ok());
        assert!(verify_checksum(
            &f,
            "0000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gpg_signature_passes_with_trusted_key() {
        let dir = std::env::temp_dir().join("bw-anx-verify-gpg");
        let home = dir.join("home");
        let keyring = dir.join("keys");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&keyring).unwrap();

        gen_test_key(&home);

        let data = dir.join("nginx.anxpkg");
        let sig = dir.join("nginx.sig.gpg");
        std::fs::write(&data, b"fake package payload").unwrap();
        gpg_sign(&home, &data, &sig);

        export_pubkey(&home, &keyring.join("test-key.asc"));
        assert!(verify_gpg(&data, &sig, &keyring).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gpg_signature_refused_when_unsigned_with_keys_enrolled() {
        let dir = std::env::temp_dir().join("bw-anx-verify-gpg-missing-sig");
        let home = dir.join("home");
        let keyring = dir.join("keys");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&keyring).unwrap();

        gen_test_key(&home);
        export_pubkey(&home, &keyring.join("test-key.asc"));

        let data = dir.join("redis.anxpkg");
        std::fs::write(&data, b"fake package payload").unwrap();

        let missing_sig = dir.join("missing.sig.gpg");
        assert!(verify_gpg(&data, &missing_sig, &keyring).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_empty_input() {
        assert_eq!(
            sha256_bytes(&[]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_single_byte() {
        assert_eq!(
            sha256_bytes(&[0xFF]),
            "a8100ae6aa1940d0b663bb31cd466142ebbdbd5187131b92d93818987832eb89"
        );
    }

    #[test]
    fn sha256_deterministic() {
        let input = b"deterministic input for hashing";
        assert_eq!(sha256_bytes(input), sha256_bytes(input));
    }

    #[test]
    fn sha256_different_inputs_different_hashes() {
        assert_ne!(sha256_bytes(b"input A"), sha256_bytes(b"input B"));
    }

    #[test]
    fn verify_checksum_wrong_length() {
        let dir = std::env::temp_dir().join("bw-anx-verify-wrong-length");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("pkg.bin");
        std::fs::write(&f, b"payload").unwrap();

        assert!(verify_checksum(&f, "abc123").is_err());
        assert!(verify_checksum(
            &f,
            "00000000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gpg_verify_skips_when_no_keys() {
        let dir = std::env::temp_dir().join("bw-anx-verify-gpg-no-keys");
        let keyring = dir.join("keys");
        std::fs::create_dir_all(&keyring).unwrap();

        let data = dir.join("pkg.anxpkg");
        std::fs::write(&data, b"unsigned payload").unwrap();
        let sig = dir.join("nonexistent.sig.gpg");

        assert!(verify_gpg(&data, &sig, &keyring).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

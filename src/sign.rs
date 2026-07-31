//! Producing a commit or tag signature by shelling out to the configured
//! signer, exactly as git's `gpg-interface.c` does. gitoxide bundles no
//! crypto, so re-signing a rewritten commit or a moved tag means running
//! `gpg` or `ssh-keygen` over its payload.
//!
//! The payload is whatever git signs (see `crate::rewrite`): for a
//! commit, its serialized content without the `gpgsig` header, which is
//! where the armored signature goes back; for a tag, the serialized
//! content up to and including the newline that ends the message, with
//! the signature appended after it.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// The signature scheme, from `gpg.format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignFormat {
    OpenPgp,
    Ssh,
    X509,
}

impl SignFormat {
    /// Parse `gpg.format` (default OpenPGP, matching git).
    pub fn parse(value: &str) -> SignFormat {
        match value.trim().to_ascii_lowercase().as_str() {
            "ssh" => SignFormat::Ssh,
            "x509" => SignFormat::X509,
            _ => SignFormat::OpenPgp,
        }
    }
}

/// Signature block markers git recognizes at the end of a tag payload
/// (`gpg-interface.c`).
const SIGNATURE_MARKERS: [&[u8]; 4] = [
    b"-----BEGIN PGP SIGNATURE-----",
    b"-----BEGIN PGP MESSAGE-----",
    b"-----BEGIN SSH SIGNATURE-----",
    b"-----BEGIN SIGNED MESSAGE-----",
];

/// Byte offset where a signature block embedded in a tag message
/// starts, if any. The block must begin at the start of a line, as in
/// git. gix only splits PGP SIGNATURE blocks out of tag messages when
/// decoding; SSH (and other) signature blocks stay inside the message,
/// so callers must detect and strip them with this.
pub fn embedded_signature(message: &[u8]) -> Option<usize> {
    let line_starts = std::iter::once(0).chain(
        message
            .iter()
            .enumerate()
            .filter(|&(_, &b)| b == b'\n')
            .map(|(i, _)| i + 1),
    );
    for start in line_starts {
        let rest = &message[start..];
        if SIGNATURE_MARKERS.iter().any(|m| rest.starts_with(m)) {
            return Some(start);
        }
    }
    None
}

/// A configured signer.
#[derive(Debug, Clone)]
pub struct Signer {
    pub format: SignFormat,
    /// `user.signingkey` (a key id, a key path, or a literal SSH key).
    pub key: String,
    /// The signer program (`gpg` / `ssh-keygen` / ...).
    pub program: OsString,
}

/// Why signing could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum SignError {
    #[error("x509 (gpgsm) signing is not supported; re-run with --no-sign")]
    Unsupported,
    #[error("no signing key configured (git config user.signingkey); re-run with --no-sign")]
    NoKey,
    #[error("could not run the signer '{0}': {1}")]
    Spawn(String, String),
    #[error("the signer '{0}' failed: {1}")]
    Failed(String, String),
    #[error("i/o error while signing: {0}")]
    Io(String),
}

impl Signer {
    /// Sign `payload`, returning the armored signature to store as the
    /// `gpgsig` header.
    pub fn sign(&self, payload: &[u8]) -> Result<Vec<u8>, SignError> {
        if self.key.trim().is_empty() {
            return Err(SignError::NoKey);
        }
        match self.format {
            SignFormat::OpenPgp => self.sign_openpgp(payload),
            SignFormat::Ssh => self.sign_ssh(payload),
            SignFormat::X509 => Err(SignError::Unsupported),
        }
    }

    fn program_name(&self) -> String {
        self.program.to_string_lossy().into_owned()
    }

    /// `gpg --status-fd=2 -bsau <key>`: payload on stdin, armored
    /// detached signature on stdout, success confirmed by SIG_CREATED.
    fn sign_openpgp(&self, payload: &[u8]) -> Result<Vec<u8>, SignError> {
        let mut child = Command::new(&self.program)
            .args(openpgp_args(&self.key))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SignError::Spawn(self.program_name(), e.to_string()))?;
        child
            .stdin
            .take()
            .expect("stdin piped")
            .write_all(payload)
            .map_err(|e| SignError::Io(e.to_string()))?;
        let out = child
            .wait_with_output()
            .map_err(|e| SignError::Io(e.to_string()))?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() || !stderr.contains("SIG_CREATED") {
            return Err(SignError::Failed(self.program_name(), stderr.into_owned()));
        }
        Ok(out.stdout)
    }

    /// `ssh-keygen -Y sign -n git -f <key> <payload-file>`, reading the
    /// signature back from `<payload-file>.sig`.
    fn sign_ssh(&self, payload: &[u8]) -> Result<Vec<u8>, SignError> {
        let tmp = TempPaths::new();
        std::fs::write(&tmp.payload, payload).map_err(|e| SignError::Io(e.to_string()))?;

        // A literal public key is written to a temp file; otherwise the
        // key is a path (with a possible leading ~).
        let key_path: PathBuf = if is_literal_ssh_key(&self.key) {
            std::fs::write(&tmp.key, self.key.as_bytes())
                .map_err(|e| SignError::Io(e.to_string()))?;
            tmp.key.clone()
        } else {
            expand_tilde(&self.key)
        };

        let status = Command::new(&self.program)
            .args(ssh_args(key_path.as_os_str(), tmp.payload.as_os_str()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| SignError::Spawn(self.program_name(), e.to_string()))?;

        let sig_path = sig_path(&tmp.payload);
        let result = if status.status.success() {
            std::fs::read(&sig_path).map_err(|e| SignError::Io(e.to_string()))
        } else {
            Err(SignError::Failed(
                self.program_name(),
                String::from_utf8_lossy(&status.stderr).into_owned(),
            ))
        };
        let _ = std::fs::remove_file(&sig_path);
        let sig = result?;
        if !sig.starts_with(b"-----BEGIN SSH SIGNATURE-----") {
            return Err(SignError::Failed(
                self.program_name(),
                "ssh-keygen did not produce an SSH signature".to_string(),
            ));
        }
        Ok(sig)
    }
}

/// argv for the OpenPGP detached-armored signature.
fn openpgp_args(key: &str) -> Vec<String> {
    vec![
        "--status-fd=2".to_string(),
        "-bsau".to_string(),
        key.to_string(),
    ]
}

/// argv for `ssh-keygen -Y sign` over a payload file.
fn ssh_args(key: &OsStr, payload: &OsStr) -> Vec<OsString> {
    vec![
        OsString::from("-Y"),
        OsString::from("sign"),
        OsString::from("-n"),
        OsString::from("git"),
        OsString::from("-f"),
        key.to_os_string(),
        payload.to_os_string(),
    ]
}

/// Whether `user.signingkey` is a literal SSH public key rather than a
/// path (git checks the same prefixes).
fn is_literal_ssh_key(key: &str) -> bool {
    let k = key.trim_start();
    k.starts_with("ssh-") || k.starts_with("sk-") || k.starts_with("key::")
}

/// Expand a leading `~/` using $HOME; leave other paths untouched.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn sig_path(payload: &std::path::Path) -> PathBuf {
    let mut s = payload.as_os_str().to_os_string();
    s.push(".sig");
    PathBuf::from(s)
}

/// Unique temp file paths for one SSH signing run, cleaned up on drop.
struct TempPaths {
    payload: PathBuf,
    key: PathBuf,
}

impl TempPaths {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!("git-redate-sign-{}-{n}", std::process::id()));
        TempPaths {
            payload: base.with_extension("payload"),
            key: base.with_extension("key"),
        }
    }
}

impl Drop for TempPaths {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.payload);
        let _ = std::fs::remove_file(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parse_defaults_to_openpgp() {
        assert_eq!(SignFormat::parse("ssh"), SignFormat::Ssh);
        assert_eq!(SignFormat::parse("SSH"), SignFormat::Ssh);
        assert_eq!(SignFormat::parse("x509"), SignFormat::X509);
        assert_eq!(SignFormat::parse("openpgp"), SignFormat::OpenPgp);
        assert_eq!(SignFormat::parse(""), SignFormat::OpenPgp);
        assert_eq!(SignFormat::parse("gpg"), SignFormat::OpenPgp);
    }

    #[test]
    fn literal_ssh_key_detection() {
        assert!(is_literal_ssh_key("ssh-ed25519 AAAAC3Nz..."));
        assert!(is_literal_ssh_key("sk-ssh-ed25519@openssh.com AAAA..."));
        assert!(is_literal_ssh_key("key::ssh-ed25519 AAAA..."));
        assert!(!is_literal_ssh_key("~/.ssh/id_ed25519.pub"));
        assert!(!is_literal_ssh_key("/home/u/.ssh/id_ed25519"));
        assert!(!is_literal_ssh_key("ABCD1234")); // a gpg key id
    }

    #[test]
    fn tilde_expands_with_home() {
        std::env::set_var("HOME", "/home/tester");
        assert_eq!(
            expand_tilde("~/.ssh/k"),
            PathBuf::from("/home/tester/.ssh/k")
        );
        assert_eq!(expand_tilde("/abs/k"), PathBuf::from("/abs/k"));
        assert_eq!(expand_tilde("rel/k"), PathBuf::from("rel/k"));
    }

    #[test]
    fn openpgp_argv_matches_git() {
        assert_eq!(
            openpgp_args("ABCD"),
            vec![
                "--status-fd=2".to_string(),
                "-bsau".to_string(),
                "ABCD".to_string()
            ]
        );
    }

    #[test]
    fn ssh_argv_uses_git_namespace() {
        let args = ssh_args(OsStr::new("/k"), OsStr::new("/p"));
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["-Y", "sign", "-n", "git", "-f", "/k", "/p"]);
    }

    #[test]
    fn sig_path_appends_sig() {
        assert_eq!(
            sig_path(std::path::Path::new("/tmp/x.payload")),
            PathBuf::from("/tmp/x.payload.sig")
        );
    }

    #[test]
    fn x509_is_unsupported() {
        let s = Signer {
            format: SignFormat::X509,
            key: "whatever".to_string(),
            program: "gpgsm".into(),
        };
        assert!(matches!(s.sign(b"data"), Err(SignError::Unsupported)));
    }

    #[test]
    fn empty_key_is_rejected() {
        let s = Signer {
            format: SignFormat::Ssh,
            key: "  ".to_string(),
            program: "ssh-keygen".into(),
        };
        assert!(matches!(s.sign(b"data"), Err(SignError::NoKey)));
    }

    // Full SSH round-trip: generate an ephemeral key, sign a payload,
    // and verify with `ssh-keygen -Y verify`. Skips if ssh-keygen is
    // absent.
    #[test]
    fn ssh_sign_round_trips() {
        let dir = std::env::temp_dir().join(format!("git-redate-sshtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("id");
        let gen = Command::new("ssh-keygen")
            .args(["-t", "ed25519", "-N", "", "-C", "redate@test", "-f"])
            .arg(&key)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let Ok(status) = gen else {
            eprintln!("skipping: ssh-keygen not available");
            return;
        };
        if !status.success() {
            eprintln!("skipping: ssh-keygen keygen failed");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let signer = Signer {
            format: SignFormat::Ssh,
            key: key.to_string_lossy().into_owned(),
            program: "ssh-keygen".into(),
        };
        let payload = b"payload bytes to sign\n";
        let sig = signer.sign(payload).expect("sign should succeed");
        assert!(sig.starts_with(b"-----BEGIN SSH SIGNATURE-----"));

        // Verify: allowed_signers maps a principal to the public key.
        let pubkey = std::fs::read_to_string(key.with_extension("pub")).unwrap();
        let allowed = dir.join("allowed_signers");
        std::fs::write(&allowed, format!("redate@test {}", pubkey.trim())).unwrap();
        let sig_file = dir.join("payload.sig");
        std::fs::write(&sig_file, &sig).unwrap();

        let verify = Command::new("ssh-keygen")
            .args(["-Y", "verify", "-f"])
            .arg(&allowed)
            .args(["-I", "redate@test", "-n", "git", "-s"])
            .arg(&sig_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .and_then(|mut c| {
                c.stdin.take().unwrap().write_all(payload)?;
                c.wait()
            })
            .unwrap();
        assert!(
            verify.success(),
            "ssh-keygen -Y verify should accept our signature"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn embedded_signature_finds_blocks_at_line_starts() {
        let at_start = b"-----BEGIN SSH SIGNATURE-----\nabc";
        assert_eq!(embedded_signature(at_start), Some(0));

        let after_msg = b"a tag\n-----BEGIN SSH SIGNATURE-----\nabc";
        assert_eq!(embedded_signature(after_msg), Some(6));

        let pgp = b"msg\n-----BEGIN PGP SIGNATURE-----\nabc";
        assert_eq!(embedded_signature(pgp), Some(4));

        assert_eq!(embedded_signature(b"no signature here"), None);
        // Not at a line start: not a signature block.
        let mid_line = b"see -----BEGIN SSH SIGNATURE----- for details";
        assert_eq!(embedded_signature(mid_line), None);
    }
}

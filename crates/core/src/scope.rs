//! Workholding: which paths a machine may read and which it may cut.

use anyhow::{bail, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::{Component, Path, PathBuf};

/// Paths no machine reads or edits through the factory, whatever the
/// traveler says. Keeps credentials out of model context.
const SECRET_GLOBS: &[&str] = &[
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*.key",
    "**/id_rsa*",
    "**/id_ed25519*",
    "**/.git/**",
    ".git/**",
];

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allowed,
    /// Matched a deny glob or fell outside every allow glob.
    OutOfScope(String),
    /// The factory's own files or secrets.
    Forbidden(String),
}

pub struct Scope {
    root: PathBuf,
    allow: Option<GlobSet>,
    deny: GlobSet,
    secret: GlobSet,
    /// Exact paths a human approved for this job; they beat the denylist.
    widened: Vec<String>,
    allow_src: Vec<String>,
    deny_src: Vec<String>,
}

fn build(globs: &[impl AsRef<str>]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for g in globs {
        let g = g.as_ref();
        b.add(Glob::new(g)?);
        // "src" should mean "src/**" as people expect.
        if !g.contains('*') && !g.contains('?') && !g.contains('[') {
            b.add(Glob::new(&format!("{}/**", g.trim_end_matches('/')))?);
        }
    }
    Ok(b.build()?)
}

impl Scope {
    pub fn new(root: &Path, allow: &[String], deny: &[String]) -> Result<Self> {
        Self::with_widened(root, allow, deny, &[])
    }

    pub fn with_widened(root: &Path, allow: &[String], deny: &[String], widened: &[String]) -> Result<Self> {
        let root = root.canonicalize()?;
        Ok(Self {
            widened: widened.to_vec(),
            allow: if allow.is_empty() { None } else { Some(build(allow)?) },
            deny: build(deny)?,
            secret: build(SECRET_GLOBS)?,
            root,
            allow_src: allow.to_vec(),
            deny_src: deny.to_vec(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Turn a user or model supplied path into a repo-relative one with
    /// forward slashes, refusing anything that escapes the repo.
    pub fn rel(&self, p: &str) -> Result<String> {
        let p = p.replace('\\', "/");
        let path = Path::new(&p);
        let joined = if path.is_absolute() { path.to_path_buf() } else { self.root.join(path) };
        let mut out = PathBuf::new();
        for c in joined.components() {
            match c {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => {}
                other => out.push(other),
            }
        }
        // Resolve symlinks on the longest existing prefix so a link inside
        // the repo cannot point the machine outside it.
        let mut probe = out.clone();
        let mut tail = Vec::new();
        while !probe.exists() {
            match probe.file_name() {
                Some(n) => {
                    tail.push(n.to_os_string());
                    probe.pop();
                }
                None => break,
            }
        }
        let mut real = probe.canonicalize().unwrap_or(probe);
        for n in tail.into_iter().rev() {
            real.push(n);
        }
        let rel = match real.strip_prefix(&self.root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => bail!("{} is outside the repo", p),
        };
        let s = rel.to_string_lossy().replace('\\', "/");
        if s.is_empty() {
            bail!("path names the repo root, not a file");
        }
        Ok(s)
    }

    pub fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn can_read(&self, rel: &str) -> Verdict {
        if self.secret.is_match(rel) {
            return Verdict::Forbidden(format!("{rel} looks like a secret or VCS internals"));
        }
        Verdict::Allowed
    }

    pub fn can_edit(&self, rel: &str) -> Verdict {
        if let v @ Verdict::Forbidden(_) = self.can_read(rel) {
            return v;
        }
        if rel == ".ndstrl" || rel.starts_with(".ndstrl/") {
            return Verdict::Forbidden("the traveler and factory state are not editable by machines".into());
        }
        if self.widened.iter().any(|w| w == rel) {
            return Verdict::Allowed;
        }
        if self.deny.is_match(rel) {
            return Verdict::OutOfScope(format!("{rel} matches the denylist {:?}", self.deny_src));
        }
        if let Some(a) = &self.allow {
            if !a.is_match(rel) {
                return Verdict::OutOfScope(format!("{rel} is outside the allowed paths {:?}", self.allow_src));
            }
        }
        Verdict::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(allow: &[&str], deny: &[&str]) -> (tempfile::TempDir, Scope) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src/billing")).unwrap();
        let s = Scope::new(
            d.path(),
            &allow.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &deny.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
        .unwrap();
        (d, s)
    }

    #[test]
    fn allow_deny_and_bare_dirs() {
        let (_d, s) = scope(&["src", "tests/**"], &["src/billing"]);
        assert_eq!(s.can_edit("src/invoice.rs"), Verdict::Allowed);
        assert_eq!(s.can_edit("tests/a.rs"), Verdict::Allowed);
        assert!(matches!(s.can_edit("src/billing/tax.rs"), Verdict::OutOfScope(_)));
        assert!(matches!(s.can_edit("README.md"), Verdict::OutOfScope(_)));
        assert!(matches!(s.can_edit(".ndstrl/job.toml"), Verdict::Forbidden(_)));
        assert!(matches!(s.can_edit(".env"), Verdict::Forbidden(_)));
    }

    #[test]
    fn escapes_are_refused() {
        let (_d, s) = scope(&[], &[]);
        assert!(s.rel("../etc/passwd").is_err());
        assert!(s.rel("/etc/passwd").is_err());
        assert_eq!(s.rel("src/../src/new.rs").unwrap(), "src/new.rs");
        assert_eq!(s.rel("./src\\billing\\x.rs").unwrap(), "src/billing/x.rs");
    }
}

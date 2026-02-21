use crate::tracy;

use std::path::{Path, PathBuf};

use anyhow::Result;

/// Ignore matcher loaded from `.mogged`.
///
/// Rules are repo-root-relative and use `/` separators.
/// This is intentionally very simple and flat so we can add a bloom-filter precheck later.
pub struct Ignore {
    root: PathBuf,
    exact: Vec<Vec<u8>>,
    prefixes: Vec<Vec<u8>>,
    globs: Vec<SimpleGlob>,
}

impl Ignore {
    pub fn load(repo_root: &Path) -> Result<Self> {
        let root = repo_root.canonicalize()?;

        let mut exact = Vec::new();
        let mut prefixes = Vec::new();
        let mut globs = Vec::new();

        //
        // Builtins: always ignore VCS metadata + our own store.
        //
        prefixes.push(b".mog/".into());
        prefixes.push(b".git/".into());
        exact.push(b".mog".into());
        exact.push(b".git".into());

        let path = root.join(".mogged");
        if let Ok(content) = std::fs::read_to_string(&path) {
            for raw in content.lines() {
                let line = raw.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                let mut p = line.replace('\\', "/");
                while p.starts_with('/') {
                    p.remove(0);
                }

                if p.is_empty() {
                    continue;
                }

                //
                // Directory rule: `foo/` => ignore prefix `foo/`.
                //
                if p.ends_with('/') {
                    prefixes.push(p.into_bytes());
                    continue;
                }

                //
                // Glob rule.
                //
                if p.as_bytes().iter().any(|&b| matches!(b, b'*' | b'?' | b'[' | b']')) {
                    globs.push(SimpleGlob::new(&p));
                    continue;
                }

                //
                // Exact rule, and also a directory prefix rule of the same name.
                //
                exact.push(p.as_bytes().into());
                let mut dir = p.into_bytes();
                dir.push(b'/');
                prefixes.push(dir);
            }
        }

        exact.sort_unstable();
        exact.dedup();
        prefixes.sort_unstable();
        prefixes.dedup();

        Ok(Self {
            root,
            exact,
            prefixes,
            globs,
        })
    }

    #[inline]
    #[must_use]
    pub fn empty() -> Self {
        Self {
            root:     PathBuf::from("/mock"),
            exact:    Vec::new(),
            prefixes: Vec::new(),
            globs:    Vec::new(),
        }
    }

    #[inline]
    #[must_use]
    pub fn is_ignored_abs(&self, abs: &Path) -> bool {
        let Ok(rel) = abs.strip_prefix(&self.root) else { return false };
        if rel.as_os_str().is_empty() {
            return false;
        }
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        self.is_ignored_rel(&rel_str)
    }

    #[must_use]
    pub fn is_ignored_rel(&self, rel: &str) -> bool {
        let _span = tracy::span!("Ignore::is_ignored_rel");

        let rel = rel.trim_start_matches('/');
        if rel.is_empty() {
            return false;
        }

        let bytes = rel.as_bytes();

        if self.exact.binary_search_by(|e| e.as_slice().cmp(bytes)).is_ok() {
            return true;
        }

        for p in &self.prefixes {
            if bytes.starts_with(p) {
                return true;
            }
        }

        for g in &self.globs {
            if g.is_match(bytes) {
                return true;
            }
        }

        false
    }
}

/// Minimal glob matcher for `*` and `?` (and `[]` treated literally for now).
/// Matches across `/` as well (repo-relative path string).
pub struct SimpleGlob {
    pat: Vec<u8>,
}

impl SimpleGlob {
    #[must_use]
    pub fn new(pat: &str) -> Self {
        Self { pat: pat.as_bytes().to_vec() }
    }

    #[must_use]
    pub fn is_match(&self, text: &[u8]) -> bool {
        let pat = &self.pat;

        //
        // Two-pointer with backtracking for `*`.
        //
        let (mut pi, mut ti) = (0usize, 0usize);
        let (mut star, mut star_text) = (None::<usize>, 0usize);

        while ti < text.len() {
            if pi < pat.len() && (pat[pi] == text[ti] || pat[pi] == b'?') {
                pi += 1;
                ti += 1;
                continue;
            }

            if pi < pat.len() && pat[pi] == b'*' {
                star = Some(pi);
                pi += 1;
                star_text = ti;
                continue;
            }

            if let Some(star_pi) = star {
                // Try to extend the `*` match by one more character.
                pi = star_pi + 1;
                star_text += 1;
                ti = star_text;
                continue;
            }

            return false;
        }

        // Trailing
        while pi < pat.len() && pat[pi] == b'*' {
            pi += 1;
        }

        pi == pat.len()
    }
}

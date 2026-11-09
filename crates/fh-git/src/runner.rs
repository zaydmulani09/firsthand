//! The single entry point for running git. Hardened environment, byte output,
//! no pager, no prompts, no locale dependence.

use fh_core::{Error, Result};
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Minimum supported git version.
pub const MIN_GIT: (u32, u32) = (2, 30);

/// Runs git in a working directory.
#[derive(Debug, Clone)]
pub struct GitRunner {
    cwd: PathBuf,
}

/// Inherited variables that would redirect git away from the working directory.
const STRIP_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
];

impl GitRunner {
    /// Runner rooted at `cwd`.
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        GitRunner { cwd: cwd.into() }
    }

    /// Working directory.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// A configured `git` command. Callers add arguments.
    pub fn command(&self) -> Command {
        let mut c = Command::new("git");
        c.current_dir(&self.cwd)
            .args(["-c", "core.quotePath=false", "-c", "color.ui=never"])
            .env("LC_ALL", "C")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GCM_INTERACTIVE", "never")
            .stdin(Stdio::null());
        for k in STRIP_ENV {
            c.env_remove(k);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        c
    }

    /// Run with extra env and optional stdin. Returns the raw output whatever the exit code.
    pub fn output_with<S: AsRef<OsStr>>(
        &self,
        args: &[S],
        env: &[(&str, &OsStr)],
        stdin: Option<&[u8]>,
    ) -> Result<Output> {
        let mut c = self.command();
        c.args(args);
        for (k, v) in env {
            c.env(k, v);
        }
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        if stdin.is_some() {
            c.stdin(Stdio::piped());
        }
        let mut child = c.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::Environment("git not found on PATH (install git 2.30 or newer)".to_owned())
            } else {
                Error::io(&self.cwd, e)
            }
        })?;
        if let Some(data) = stdin
            && let Some(mut s) = child.stdin.take()
        {
            s.write_all(data).map_err(|e| Error::io(&self.cwd, e))?;
        }
        child
            .wait_with_output()
            .map_err(|e| Error::io(&self.cwd, e))
    }

    /// Run and require exit 0. Returns stdout bytes.
    pub fn run<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<Vec<u8>> {
        self.run_with(args, &[], None)
    }

    /// Run with env and stdin and require exit 0.
    pub fn run_with<S: AsRef<OsStr>>(
        &self,
        args: &[S],
        env: &[(&str, &OsStr)],
        stdin: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let out = self.output_with(args, env, stdin)?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(git_err(args, &out))
        }
    }

    /// Run and return trimmed stdout as UTF-8.
    pub fn run_str<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<String> {
        let b = self.run(args)?;
        Ok(String::from_utf8_lossy(&b)
            .trim_end_matches(['\n', '\r'])
            .to_owned())
    }

    /// Run and return `Some(stdout)` on success, `None` on a non-zero exit.
    pub fn run_opt<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<Option<String>> {
        let out = self.output_with(args, &[], None)?;
        if out.status.success() {
            Ok(Some(
                String::from_utf8_lossy(&out.stdout)
                    .trim_end_matches(['\n', '\r'])
                    .to_owned(),
            ))
        } else {
            Ok(None)
        }
    }

    /// `git --version`, checked against [`MIN_GIT`].
    pub fn version(&self) -> Result<String> {
        let v = self.run_str(&["--version"])?;
        let (maj, min) = parse_version(&v)
            .ok_or_else(|| Error::Environment(format!("could not parse git version from {v:?}")))?;
        if (maj, min) < MIN_GIT {
            return Err(Error::Environment(format!(
                "git {maj}.{min} is too old; firsthand needs git {}.{} or newer",
                MIN_GIT.0, MIN_GIT.1
            )));
        }
        Ok(v)
    }
}

fn git_err<S: AsRef<OsStr>>(args: &[S], out: &Output) -> Error {
    let args: Vec<String> = args
        .iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect();
    let mut stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    if stderr.len() > 2000 {
        stderr.truncate(2000);
    }
    Error::Git {
        args: args.join(" "),
        code: out.status.code(),
        stderr,
    }
}

/// Parse `git version 2.53.0.windows.1` into `(2, 53)`.
pub fn parse_version(s: &str) -> Option<(u32, u32)> {
    let v = s
        .split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let mut it = v.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// Split NUL-delimited output into UTF-8 strings, dropping the trailing empty item.
pub fn split_nul(b: &[u8]) -> Vec<String> {
    b.split(|c| *c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("git version 2.53.0.windows.1"), Some((2, 53)));
        assert_eq!(parse_version("git version 2.30.1"), Some((2, 30)));
        assert_eq!(parse_version("nonsense"), None);
        assert_eq!(
            split_nul(b"a\0b c\0"),
            vec!["a".to_owned(), "b c".to_owned()]
        );
    }
}

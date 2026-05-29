mod path_policy;

use std::path::{Path, PathBuf};

pub use path_policy::check_write_allowed;
use path_policy::{canonical_workdir, resolve_existing, resolve_for_create, verify_resolved_path};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use globset::GlobBuilder;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

/// Default shell command timeout (seconds). Configurable later via settings.
pub const BASH_TIMEOUT_SECS: u64 = 120;

/// Max captured stdout/stderr per Bash invocation.
pub const BASH_OUTPUT_LIMIT: usize = 256 * 1024;

const GLOB_MAX_MATCHES: usize = 1000;
const GREP_MAX_MATCHES: usize = 100;

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone)]
pub struct LocalSandbox {
    workdir: PathBuf,
}

impl LocalSandbox {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self {
            workdir: workdir.into(),
        }
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    pub fn resolve(&self, path: &str) -> Result<PathBuf> {
        resolve_existing(&self.workdir, path)
    }

    pub async fn read_file_bytes(&self, path: &str, _hint_max: usize) -> Result<Vec<u8>> {
        let resolved = self.resolve(path)?;
        verify_resolved_path(&self.workdir, &resolved)?;
        read_file_bytes_nofollow(&resolved)
            .await
            .with_context(|| format!("read file: {}", resolved.display()))
    }

    pub async fn read_text(&self, path: &str) -> Result<String> {
        let bytes = self.read_file_bytes(path, 0).await?;
        if is_binary_content(&bytes) {
            anyhow::bail!(
                "cannot read binary file: {path}. Read supports text files only."
            );
        }
        String::from_utf8(bytes).context("file is not valid UTF-8 text")
    }

    pub async fn write_text(&self, path: &str, content: &str) -> Result<()> {
        if let Err(msg) = path_policy::check_write_allowed(path) {
            anyhow::bail!(msg);
        }
        let resolved = self.resolve_parent(path)?;
        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("create parent dir: {}", parent.display()))?;
        }
        tokio::fs::write(&resolved, content)
            .await
            .with_context(|| format!("write file: {}", resolved.display()))
    }

    pub async fn exec(
        &self,
        command: &str,
        cwd: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<ExecResult> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("aborted");
        }

        let work = self.exec_inner(command, cwd);
        let timed = tokio::time::timeout(Duration::from_secs(BASH_TIMEOUT_SECS), work);

        if let Some(token) = cancel {
            tokio::select! {
                _ = token.cancelled() => anyhow::bail!("aborted"),
                result = timed => match result {
                    Ok(inner) => inner,
                    Err(_) => anyhow::bail!(
                        "command timed out after {} seconds",
                        BASH_TIMEOUT_SECS
                    ),
                },
            }
        } else {
            match timed.await {
                Ok(inner) => inner,
                Err(_) => anyhow::bail!(
                    "command timed out after {} seconds",
                    BASH_TIMEOUT_SECS
                ),
            }
        }
    }

    async fn exec_inner(&self, command: &str, cwd: Option<&str>) -> Result<ExecResult> {
        let cwd = match cwd {
            Some(path) => self.resolve(path)?,
            None => canonical_workdir(&self.workdir)?,
        };

        #[cfg(unix)]
        let mut cmd = {
            let mut cmd = Command::new("bash");
            cmd.arg("-lc").arg(command);
            cmd
        };

        #[cfg(windows)]
        let mut cmd = {
            let mut cmd = Command::new("cmd");
            cmd.arg("/C").arg(command);
            cmd
        };

        cmd.current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().context("spawn shell command")?;
        let mut stdout = String::new();
        let mut stderr = String::new();

        if let Some(mut out) = child.stdout.take() {
            stdout = read_limited(&mut out, BASH_OUTPUT_LIMIT).await?;
        }
        if let Some(mut err) = child.stderr.take() {
            stderr = read_limited(&mut err, BASH_OUTPUT_LIMIT).await?;
        }

        let status = child.wait().await.context("wait for shell command")?;
        Ok(ExecResult {
            stdout,
            stderr,
            exit_code: status.code().unwrap_or(-1),
        })
    }

    pub fn glob(&self, pattern: &str) -> Result<Vec<String>> {
        let pattern = pattern.trim_start_matches("./");
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid glob pattern: {pattern}"))?;
        let matcher = glob.compile_matcher();

        let mut matches = Vec::new();
        for entry in WalkDir::new(&self.workdir).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&self.workdir)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            if matcher.is_match(&rel) {
                matches.push(rel);
            }
            if matches.len() >= GLOB_MAX_MATCHES {
                break;
            }
        }
        matches.sort();
        Ok(matches)
    }

    pub async fn grep(
        &self,
        pattern: &str,
        path: Option<&str>,
        case_insensitive: bool,
    ) -> Result<Vec<GrepMatch>> {
        if let Some(matches) = self.grep_with_rg(pattern, path, case_insensitive).await? {
            return Ok(matches);
        }
        self.grep_fallback(pattern, path, case_insensitive).await
    }

    async fn grep_with_rg(
        &self,
        pattern: &str,
        path: Option<&str>,
        case_insensitive: bool,
    ) -> Result<Option<Vec<GrepMatch>>> {
        let workdir = canonical_workdir(&self.workdir)?;

        let mut cmd = Command::new("rg");
        cmd.arg("--line-number")
            .arg("--no-heading")
            .arg("--color=never")
            .arg("--max-count")
            .arg(GREP_MAX_MATCHES.to_string())
            .current_dir(&workdir);
        if case_insensitive {
            cmd.arg("-i");
        }
        cmd.arg(pattern);
        if let Some(p) = path {
            let resolved = self.resolve(p)?;
            let rel = resolved
                .strip_prefix(&workdir)
                .unwrap_or(&resolved)
                .to_string_lossy()
                .into_owned();
            cmd.arg(rel);
        } else {
            cmd.arg(".");
        }

        let output = match cmd.output().await {
            Ok(output) => output,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err).context("spawn rg"),
        };

        if !output.status.success() && output.status.code() != Some(1) {
            return Ok(None);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut matches = Vec::new();
        for line in stdout.lines() {
            if let Some(m) = parse_rg_line(line) {
                matches.push(m);
            }
            if matches.len() >= GREP_MAX_MATCHES {
                break;
            }
        }
        Ok(Some(matches))
    }

    async fn grep_fallback(
        &self,
        pattern: &str,
        path: Option<&str>,
        case_insensitive: bool,
    ) -> Result<Vec<GrepMatch>> {
        let regex = if case_insensitive {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .context("invalid grep pattern")?
        } else {
            regex::Regex::new(pattern).context("invalid grep pattern")?
        };

        let root = match path {
            Some(p) => self.resolve(p)?,
            None => canonical_workdir(&self.workdir)?,
        };

        let mut matches = Vec::new();
        let files: Vec<PathBuf> = if root.is_file() {
            vec![root]
        } else {
            WalkDir::new(root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .map(|e| e.into_path())
                .collect()
        };

        for file in files {
            let content = match tokio::fs::read_to_string(&file).await {
                Ok(content) => content,
                Err(_) => continue,
            };
            let rel = file
                .strip_prefix(&self.workdir)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            for (idx, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(GrepMatch {
                        path: rel.clone(),
                        line_number: idx + 1,
                        line: line.chars().take(500).collect(),
                    });
                    if matches.len() >= GREP_MAX_MATCHES {
                        return Ok(matches);
                    }
                }
            }
        }

        Ok(matches)
    }

    fn resolve_parent(&self, path: &str) -> Result<PathBuf> {
        resolve_for_create(&self.workdir, path)
    }
}

async fn read_file_bytes_nofollow(path: &Path) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::io::Read;
        use std::os::unix::fs::OpenOptionsExt;

        const O_NOFOLLOW: i32 = 0x0100;

        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(O_NOFOLLOW)
                .open(&path)
                .with_context(|| format!("open file: {}", path.display()))?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)
                .with_context(|| format!("read file: {}", path.display()))?;
            Ok(buf)
        })
        .await
        .context("read file task")?
    }

    #[cfg(not(unix))]
    {
        tokio::fs::read(path)
            .await
            .with_context(|| format!("read file: {}", path.display()))
    }
}

/// True if the buffer looks like binary (NUL byte in the first 8KB).
pub fn is_binary_content(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    sample.contains(&0)
}

#[derive(Debug, Clone)]
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

fn parse_rg_line(line: &str) -> Option<GrepMatch> {
    let (path, rest) = line.split_once(':')?;
    let (line_number, content) = rest.split_once(':')?;
    let line_number: usize = line_number.parse().ok()?;
    Some(GrepMatch {
        path: path.replace('\\', "/"),
        line_number,
        line: content.to_string(),
    })
}

async fn read_limited(reader: &mut (impl AsyncReadExt + Unpin), limit: usize) -> Result<String> {
    let mut buf = vec![0u8; 8192];
    let mut out = Vec::new();
    loop {
        let n = reader.read(&mut buf).await.context("read process output")?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() >= limit {
            out.truncate(limit);
            out.extend_from_slice(b"\n...[truncated]");
            break;
        }
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[async_trait]
pub trait Sandbox: Send + Sync {
    async fn read_text(&self, path: &str) -> Result<String>;
    async fn write_text(&self, path: &str, content: &str) -> Result<()>;
    async fn exec(
        &self,
        command: &str,
        cwd: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<ExecResult>;
}

#[async_trait]
impl Sandbox for LocalSandbox {
    async fn read_text(&self, path: &str) -> Result<String> {
        LocalSandbox::read_text(self, path).await
    }

    async fn write_text(&self, path: &str, content: &str) -> Result<()> {
        LocalSandbox::write_text(self, path, content).await
    }

    async fn exec(
        &self,
        command: &str,
        cwd: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<ExecResult> {
        LocalSandbox::exec(self, command, cwd, cancel).await
    }
}

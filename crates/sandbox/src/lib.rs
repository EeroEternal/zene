mod options;
mod path_policy;

use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use options::{url_host_port, SandboxOptions};
pub use path_policy::{check_read_allowed, check_write_allowed};
use path_policy::{canonical_workdir, resolve_existing, resolve_for_create, verify_resolved_path};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use globset::GlobBuilder;
use keel_core::backend_process_guard;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use keel_core::{backend_local_process, LocalProcessOptions};
use keel_core::{
    check_egress, ManagedProcess, NetworkPolicy, Space, SpaceHandle, SpawnRequest, StdioMode,
    TerminationReason,
};
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

pub struct InteractiveProcess {
    inner: InteractiveProcessInner,
}

enum InteractiveProcessInner {
    Local(tokio::process::Child),
    Keel(ManagedProcess),
}

impl InteractiveProcess {
    pub fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        match &mut self.inner {
            InteractiveProcessInner::Local(child) => child.stdin.take(),
            InteractiveProcessInner::Keel(process) => process.take_stdin(),
        }
    }

    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        match &mut self.inner {
            InteractiveProcessInner::Local(child) => child.stdout.take(),
            InteractiveProcessInner::Keel(process) => process.take_stdout(),
        }
    }

    pub async fn cancel(self) -> Result<()> {
        match self.inner {
            InteractiveProcessInner::Local(mut child) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            InteractiveProcessInner::Keel(process) => {
                process.cancel().await.context("cancel Keel process")?;
            }
        }
        Ok(())
    }
}

/// Optional remote text filesystem (e.g. ACP client `fs/read_text_file`).
///
/// When set on a [`LocalSandbox`], text read/write is delegated after local
/// path policy checks. Glob/exec/resolve still use the local workspace.
#[async_trait]
pub trait RemoteTextFs: Send + Sync {
    fn can_read(&self) -> bool {
        true
    }
    fn can_write(&self) -> bool {
        true
    }
    async fn read_text(&self, absolute_path: &Path) -> Result<String>;
    async fn write_text(&self, absolute_path: &Path, content: &str) -> Result<()>;
}

/// Optional remote terminal (e.g. ACP client `terminal/*`).
///
/// When set, [`LocalSandbox::exec_with_timeout`] delegates shell execution to
/// the client after resolving the working directory.
#[async_trait]
pub trait RemoteTerminal: Send + Sync {
    async fn exec(
        &self,
        command: &str,
        cwd: &Path,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
        output_byte_limit: usize,
    ) -> Result<ExecResult>;
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn linux_or_macos_backend() -> std::sync::Arc<dyn keel_core::EnforceBackend> {
    #[cfg(target_os = "linux")]
    {
        // Keel ≥0.0.12 always merges baseline credential denies. On Linux those
        // need a working bubblewrap for kernel read-deny. `auto_bwrap=false` does
        // not skip spawn-time wrapping when `bwrap` is on PATH, so fall back to
        // the soft process-guard backend when Keel-style binds cannot run.
        if !options::bubblewrap_usable_for_keel() {
            tracing::warn!(
                "bubblewrap unusable for Keel baseline denies; using process-guard backend \
                 (host path_policy still enforced)"
            );
            return backend_process_guard();
        }
    }
    backend_local_process(LocalProcessOptions::default())
}

#[derive(Clone)]
pub struct LocalSandbox {
    workdir: PathBuf,
    space: Option<SpaceHandle>,
    network: NetworkPolicy,
    profile: String,
    remote_fs: Option<Arc<dyn RemoteTextFs>>,
    remote_terminal: Option<Arc<dyn RemoteTerminal>>,
}

impl LocalSandbox {
    /// Construct a local-only sandbox, primarily for isolated unit tests.
    ///
    /// Production agent entry points should use [`Self::with_options`] / [`Self::with_keel`].
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self {
            workdir: workdir.into(),
            space: None,
            network: NetworkPolicy::Unrestricted,
            profile: "off".to_string(),
            remote_fs: None,
            remote_terminal: None,
        }
    }

    /// Attach a remote text FS bridge (ACP client filesystem).
    pub fn with_remote_fs(mut self, remote: Arc<dyn RemoteTextFs>) -> Self {
        self.remote_fs = Some(remote);
        self
    }

    pub fn set_remote_fs(&mut self, remote: Option<Arc<dyn RemoteTextFs>>) {
        self.remote_fs = remote;
    }

    /// Attach a remote terminal bridge (ACP client terminals).
    pub fn with_remote_terminal(mut self, remote: Arc<dyn RemoteTerminal>) -> Self {
        self.remote_terminal = Some(remote);
        self
    }

    pub fn set_remote_terminal(&mut self, remote: Option<Arc<dyn RemoteTerminal>>) {
        self.remote_terminal = remote;
    }

    /// Construct a sandbox with the default `workspace` Keel profile.
    pub async fn with_keel(workdir: impl Into<PathBuf>) -> Result<Self> {
        Self::with_options(workdir, SandboxOptions::default()).await
    }

    /// Construct a sandbox from explicit profile / network options.
    pub async fn with_options(workdir: impl Into<PathBuf>, opts: SandboxOptions) -> Result<Self> {
        let workdir = canonical_workdir(&workdir.into())?;
        let profile = opts.normalized_profile();

        if opts.is_off() {
            return Ok(Self {
                workdir,
                space: None,
                network: NetworkPolicy::Unrestricted,
                profile,
                remote_fs: None,
                remote_terminal: None,
            });
        }

        let policy = options::resolve_policy(&workdir, &opts)?;
        let network = policy.network.clone();

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let backend = linux_or_macos_backend();
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let backend = backend_process_guard();

        let space = Space::create(policy, backend)
            .await
            .context("create Keel execution space")?;
        Ok(Self {
            workdir,
            space: Some(space),
            network,
            profile,
            remote_fs: None,
            remote_terminal: None,
        })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn network_policy(&self) -> &NetworkPolicy {
        &self.network
    }

    /// True when a Keel execution space is active (profile is not `off`).
    pub fn is_enforced(&self) -> bool {
        self.space.is_some()
    }

    /// Host-side egress gate for tools (FetchUrl / WebSearch / HTTP MCP).
    pub async fn authorize_egress(&self, url: &str) -> Result<()> {
        let (host, port) = url_host_port(url)?;
        if let Some(space) = &self.space {
            if !space
                .check_egress(&host, port)
                .await
                .context("check Keel egress policy")?
            {
                anyhow::bail!("sandbox denied network access to {host}:{port}");
            }
            return Ok(());
        }

        let decision = check_egress(&self.network, &host, port);
        if !decision.is_allowed() {
            let reason = match decision {
                keel_core::EgressDecision::Deny { reason } => reason,
                keel_core::EgressDecision::Allow => "denied".to_string(),
            };
            anyhow::bail!("sandbox denied network access to {host}:{port}: {reason}");
        }
        Ok(())
    }

    /// Re-scope a child agent to a directory while retaining the same Keel policy.
    pub fn scoped_to(&self, workdir: impl Into<PathBuf>) -> Result<Self> {
        let requested = workdir.into();
        let resolved = self.resolve(requested.to_string_lossy().as_ref())?;
        if !resolved.is_dir() {
            anyhow::bail!("sandbox cwd is not a directory: {}", resolved.display());
        }
        Ok(Self {
            workdir: resolved,
            space: self.space.clone(),
            network: self.network.clone(),
            profile: self.profile.clone(),
            remote_fs: self.remote_fs.clone(),
            remote_terminal: self.remote_terminal.clone(),
        })
    }

    pub async fn shutdown(&self) -> Result<()> {
        if let Some(space) = &self.space {
            space
                .clone()
                .destroy()
                .await
                .context("destroy Keel execution space")?;
        }
        Ok(())
    }

    pub fn events_path(&self) -> Option<&Path> {
        self.space.as_ref().and_then(SpaceHandle::events_path)
    }

    /// Spawn an interactive stdio process such as an MCP server.
    pub async fn spawn_stdio(
        &self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<InteractiveProcess> {
        if let Some(space) = &self.space {
            let mut request = SpawnRequest::new(program)
                .args(args.iter().cloned())
                .cwd(canonical_workdir(&self.workdir)?)
                .stdin(StdioMode::Piped)
                .stdout(StdioMode::Piped)
                .stderr(StdioMode::Inherit)
                .audit_args(false);
            request.env = env.to_vec();
            let process = space
                .spawn(request)
                .await
                .context("spawn interactive process with Keel")?;
            return Ok(InteractiveProcess {
                inner: InteractiveProcessInner::Keel(process),
            });
        }

        let mut command = Command::new(program);
        command
            .args(args)
            .envs(env.iter().cloned())
            .current_dir(canonical_workdir(&self.workdir)?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let child = command.spawn().context("spawn interactive process")?;
        Ok(InteractiveProcess {
            inner: InteractiveProcessInner::Local(child),
        })
    }

    pub fn resolve(&self, path: &str) -> Result<PathBuf> {
        resolve_existing(&self.workdir, path)
    }

    pub async fn read_file_bytes(&self, path: &str, _hint_max: usize) -> Result<Vec<u8>> {
        if let Err(msg) = path_policy::check_read_allowed(path) {
            anyhow::bail!(msg);
        }
        let resolved = self.resolve(path)?;
        verify_resolved_path(&self.workdir, &resolved)?;
        if let Err(msg) = path_policy::check_read_allowed_resolved(&resolved) {
            anyhow::bail!(msg);
        }

        if let Some(space) = &self.space {
            let bytes = space
                .fs()
                .read(&resolved)
                .await
                .with_context(|| format!("Keel SpaceFs read: {}", resolved.display()))?;
            return Ok(bytes);
        }

        self.authorize_fs(&resolved, false).await?;
        read_file_bytes_nofollow(&resolved)
            .await
            .with_context(|| format!("read file: {}", resolved.display()))
    }

    pub async fn read_text(&self, path: &str) -> Result<String> {
        if let Err(msg) = path_policy::check_read_allowed(path) {
            anyhow::bail!(msg);
        }
        let resolved = self.resolve(path)?;
        verify_resolved_path(&self.workdir, &resolved)?;
        if let Err(msg) = path_policy::check_read_allowed_resolved(&resolved) {
            anyhow::bail!(msg);
        }
        if let Some(remote) = &self.remote_fs {
            if remote.can_read() {
                self.authorize_fs(&resolved, false).await?;
                return remote.read_text(&resolved).await;
            }
        }

        let bytes = self.read_file_bytes(path, 0).await?;
        if is_binary_content(&bytes) {
            anyhow::bail!("cannot read binary file: {path}. Read supports text files only.");
        }
        String::from_utf8(bytes).context("file is not valid UTF-8 text")
    }

    pub async fn write_text(&self, path: &str, content: &str) -> Result<()> {
        if let Err(msg) = path_policy::check_write_allowed(path) {
            anyhow::bail!(msg);
        }
        let resolved = self.resolve_parent(path)?;

        if let Some(remote) = &self.remote_fs {
            if remote.can_write() {
                self.authorize_fs(&resolved, true).await?;
                return remote.write_text(&resolved, content).await;
            }
        }

        if let Some(space) = &self.space {
            space
                .fs()
                .write(&resolved, content.as_bytes())
                .await
                .with_context(|| format!("Keel SpaceFs write: {}", resolved.display()))?;
            return Ok(());
        }

        self.authorize_fs(&resolved, true).await?;
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
        self.exec_with_timeout(command, cwd, cancel, Duration::from_secs(BASH_TIMEOUT_SECS))
            .await
    }

    /// Like [`Self::exec`] but with an explicit timeout (used by background Bash).
    pub async fn exec_with_timeout(
        &self,
        command: &str,
        cwd: Option<&str>,
        cancel: Option<&CancellationToken>,
        timeout: Duration,
    ) -> Result<ExecResult> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("aborted");
        }

        let timeout_secs = timeout.as_secs().max(1);
        if let Some(remote) = &self.remote_terminal {
            let cwd = match cwd {
                Some(path) => self.resolve(path)?,
                None => canonical_workdir(&self.workdir)?,
            };
            return remote
                .exec(command, &cwd, timeout, cancel, BASH_OUTPUT_LIMIT)
                .await;
        }
        if self.space.is_some() {
            let cwd = match cwd {
                Some(path) => self.resolve(path)?,
                None => canonical_workdir(&self.workdir)?,
            };
            #[cfg(unix)]
            let (program, args) = ("bash", vec!["-lc".to_string(), command.to_string()]);
            #[cfg(windows)]
            let (program, args) = ("cmd", vec!["/C".to_string(), command.to_string()]);

            return self
                .exec_keel(program, args, &cwd, cancel, timeout, false)
                .await;
        }

        let work = self.exec_inner(command, cwd);
        let timed = tokio::time::timeout(timeout, work);

        if let Some(token) = cancel {
            tokio::select! {
                _ = token.cancelled() => anyhow::bail!("aborted"),
                result = timed => match result {
                    Ok(inner) => inner,
                    Err(_) => anyhow::bail!(
                        "command timed out after {timeout_secs} seconds"
                    ),
                },
            }
        } else {
            match timed.await {
                Ok(inner) => inner,
                Err(_) => anyhow::bail!("command timed out after {timeout_secs} seconds"),
            }
        }
    }

    async fn authorize_fs(&self, path: &Path, write: bool) -> Result<()> {
        let Some(space) = &self.space else {
            return Ok(());
        };
        if !space
            .check_fs(path, write)
            .await
            .context("check Keel filesystem policy")?
        {
            anyhow::bail!(
                "Keel denied {} access: {}",
                if write { "write" } else { "read" },
                path.display()
            );
        }
        Ok(())
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

    async fn exec_keel(
        &self,
        program: &str,
        args: Vec<String>,
        cwd: &Path,
        cancel: Option<&CancellationToken>,
        timeout: Duration,
        audit_args: bool,
    ) -> Result<ExecResult> {
        let space = self
            .space
            .as_ref()
            .context("Keel execution space is not configured")?;
        let request = SpawnRequest::new(program)
            .args(args)
            .cwd(cwd)
            .audit_args(audit_args);
        let process = space.spawn(request).await.context("spawn with Keel")?;
        let (exit, output) = if let Some(token) = cancel {
            process
                .wait_with_output_cancel(token, timeout)
                .await
                .context("wait for Keel process")?
        } else {
            process
                .wait_with_output_timeout(timeout)
                .await
                .context("wait for Keel process")?
        };

        match exit.termination_reason {
            TerminationReason::TimedOut => {
                anyhow::bail!(
                    "command timed out after {} seconds",
                    timeout.as_secs().max(1)
                )
            }
            TerminationReason::Cancelled => anyhow::bail!("aborted"),
            _ => {}
        }

        Ok(ExecResult {
            stdout: output_text_limited(&output.stdout, BASH_OUTPUT_LIMIT),
            stderr: output_text_limited(&output.stderr, BASH_OUTPUT_LIMIT),
            exit_code: exit.exit_code.unwrap_or(-1),
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
        for entry in WalkDir::new(&self.workdir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
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

        if self.space.is_some() {
            let mut args = vec![
                "--line-number".to_string(),
                "--no-heading".to_string(),
                "--color=never".to_string(),
                "--max-count".to_string(),
                GREP_MAX_MATCHES.to_string(),
            ];
            if case_insensitive {
                args.push("-i".to_string());
            }
            args.push(pattern.to_string());
            if let Some(p) = path {
                let resolved = self.resolve(p)?;
                args.push(
                    resolved
                        .strip_prefix(&workdir)
                        .unwrap_or(&resolved)
                        .to_string_lossy()
                        .into_owned(),
                );
            } else {
                args.push(".".to_string());
            }

            let output = match self
                .exec_keel(
                    "rg",
                    args,
                    &workdir,
                    None,
                    Duration::from_secs(BASH_TIMEOUT_SECS),
                    true,
                )
                .await
            {
                Ok(output) => output,
                Err(err) if err.to_string().contains("No such file") => return Ok(None),
                Err(err) => return Err(err).context("spawn rg with Keel"),
            };
            if output.exit_code != 0 && output.exit_code != 1 {
                return Ok(None);
            }
            return Ok(Some(parse_rg_output(&output.stdout)));
        }

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

        Ok(Some(parse_rg_output(&String::from_utf8_lossy(
            &output.stdout,
        ))))
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

fn parse_rg_output(stdout: &str) -> Vec<GrepMatch> {
    stdout
        .lines()
        .filter_map(parse_rg_line)
        .take(GREP_MAX_MATCHES)
        .collect()
}

fn output_text_limited(bytes: &[u8], limit: usize) -> String {
    if bytes.len() <= limit {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut output = String::from_utf8_lossy(&bytes[..limit]).into_owned();
    output.push_str("\n...[truncated]");
    output
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn keel_exec_records_redacted_command_and_completion() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = LocalSandbox::with_keel(dir.path()).await.unwrap();
        let events = sandbox.events_path().unwrap().to_path_buf();

        let result = sandbox.exec("printf keel-ok", None, None).await.unwrap();
        assert_eq!(result.stdout, "keel-ok");
        assert_eq!(result.exit_code, 0);

        sandbox.shutdown().await.unwrap();
        let audit = fs::read_to_string(events).unwrap();
        assert!(audit.contains("\"kind\":\"exec\""));
        assert!(audit.contains("\"args_redacted\":true"));
        assert!(!audit.contains("printf keel-ok"));
        assert!(audit.contains("\"kind\":\"exec_finished\""));
    }

    #[tokio::test]
    async fn keel_exec_honors_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = LocalSandbox::with_keel(dir.path()).await.unwrap();
        let token = CancellationToken::new();
        let cancel = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel.cancel();
        });

        let err = sandbox
            .exec_with_timeout("sleep 30", None, Some(&token), Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("aborted"), "{err:#}");
        sandbox.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn keel_sandbox_blocks_child_write_outside_workspace() {
        let root = tempfile::Builder::new()
            .prefix(".zene-keel-test-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        let workspace = root.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let outside = root.path().join("outside.txt");
        let sandbox = LocalSandbox::with_keel(&workspace).await.unwrap();

        // Soft process-guard fallback (no usable bwrap) cannot kernel-block shell
        // redirects; host SpaceFs/policy still denies outside writes via check_fs.
        if cfg!(target_os = "linux") && !options::bubblewrap_usable_for_keel() {
            let denied = sandbox
                .authorize_fs(&outside, true)
                .await
                .unwrap_err()
                .to_string();
            assert!(
                denied.contains("denied") || denied.contains("Keel"),
                "{denied}"
            );
            sandbox.shutdown().await.unwrap();
            return;
        }

        let command = format!("printf escaped > '{}'", outside.display());
        let result = sandbox.exec(&command, None, None).await.unwrap();
        assert_ne!(result.exit_code, 0);
        assert!(!outside.exists());
        sandbox.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn keel_policy_and_path_checks_allow_nested_write() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = LocalSandbox::with_keel(dir.path()).await.unwrap();
        sandbox.write_text("foo/bar.txt", "nested").await.unwrap();
        assert_eq!(sandbox.read_text("foo/bar.txt").await.unwrap(), "nested");
        sandbox.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn keel_interactive_stdio_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = LocalSandbox::with_keel(dir.path()).await.unwrap();
        let mut process = sandbox
            .spawn_stdio("sh", &["-c".into(), "cat".into()], &[])
            .await
            .unwrap();
        let mut stdin = process.take_stdin().unwrap();
        let mut stdout = process.take_stdout().unwrap();
        stdin.write_all(b"mcp-ping\n").await.unwrap();
        stdin.shutdown().await.unwrap();
        drop(stdin);

        let mut response = String::new();
        stdout.read_to_string(&mut response).await.unwrap();
        assert_eq!(response, "mcp-ping\n");
        process.cancel().await.unwrap();
        sandbox.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn sandbox_off_skips_keel_space() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = LocalSandbox::with_options(
            dir.path(),
            SandboxOptions {
                profile: "off".into(),
                ..SandboxOptions::default()
            },
        )
        .await
        .unwrap();
        assert!(!sandbox.is_enforced());
        assert_eq!(sandbox.profile(), "off");
        sandbox.write_text("a.txt", "ok").await.unwrap();
        assert_eq!(sandbox.read_text("a.txt").await.unwrap(), "ok");
    }

    #[tokio::test]
    async fn read_only_profile_denies_host_egress() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = LocalSandbox::with_options(
            dir.path(),
            SandboxOptions {
                profile: "read-only".into(),
                ..SandboxOptions::default()
            },
        )
        .await
        .unwrap();
        let err = sandbox
            .authorize_egress("https://example.com/")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("denied"),
            "unexpected error: {err:#}"
        );
        sandbox.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn allow_hosts_permits_listed_egress() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = LocalSandbox::with_options(
            dir.path(),
            SandboxOptions {
                profile: "strict".into(),
                allow_hosts: vec!["example.com:443".into()],
                ..SandboxOptions::default()
            },
        )
        .await
        .unwrap();
        sandbox
            .authorize_egress("https://example.com/path")
            .await
            .unwrap();
        let err = sandbox
            .authorize_egress("https://evil.com/")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("denied"));
        sandbox.shutdown().await.unwrap();
    }
}

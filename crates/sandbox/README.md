# zene-sandbox

Deterministic, secure process isolation and filesystem sandbox for AI code agents, powered by Keel.

Part of the composable [Zene (Zen Engine)](https://github.com/ParaTensor/zene) agent stack. Usable standalone in any Rust application.

## Features

- **Workspace Confinement**: Restricts all command execution, file reads, and file writes to the configured working directory.
- **Symlink Escape Traversal Prevention**: Pre-execution canonical path resolution blocks attacks targeting symlinks pointing outside the workspace.
- **Sensitive Path Protection**: Rejects access to credentials (`.env*`, `.git/`, SSH keys, cloud provider tokens).
- **Process Guarding**: Enforces strict execution timeouts (default 120s) and bounded stdout/stderr capture (default 256KB).
- **Network Egress Policies**: Configurable network allowlists/denylists for sandboxed child processes.

## Usage

```rust
use std::path::PathBuf;
use std::time::Duration;
use zene_sandbox::{LocalSandbox, Sandbox, SandboxOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let workdir = PathBuf::from("./workspace");
    let options = SandboxOptions::default();
    let sandbox = LocalSandbox::new(&workdir, options)?;

    // Execute command securely inside the sandbox
    let result = sandbox.exec_command(
        "cargo check",
        Some(Duration::from_secs(60)),
        None,
    ).await?;

    println!("Exit code: {:?}", result.exit_code);
    println!("Output: {}", result.stdout);
    Ok(())
}
```

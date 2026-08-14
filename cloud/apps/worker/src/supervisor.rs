use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};
use tracing::{info, warn};
use uuid::Uuid;
use zene_cloud_domain::QueueStats;

use crate::{sleep_interruptible, spawn_shutdown_listener, Cli};

pub async fn run_supervisor(cli: Cli) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .context("build HTTP client")?;
    let exe = std::env::current_exe().context("current_exe")?;
    let mut children: Vec<ChildSlot> = Vec::new();
    let shutdown = Arc::new(AtomicBool::new(false));
    spawn_shutdown_listener(shutdown.clone());

    info!(
        api = %cli.api_url,
        min_warm = cli.min_warm,
        max_active = cli.max_active,
        max_hold = cli.max_hold,
        scale_interval_ms = cli.scale_interval_ms,
        "zene-cloud-worker supervisor started"
    );

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        reap_finished(&mut children);

        let stats = match fetch_queue_stats(&client, &cli).await {
            Ok(s) => s,
            Err(err) => {
                warn!(error = %err, "queue stats failed");
                sleep_interruptible(
                    Duration::from_millis(cli.scale_interval_ms.max(200)),
                    &shutdown,
                )
                .await;
                continue;
            }
        };

        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        terminate_excess_holds(&mut children, &stats, cli.max_hold);

        let desired_total = desired_children(&stats, cli.min_warm, cli.max_active, cli.max_hold);
        let current = children.len() as u64;

        if current < desired_total {
            let to_spawn = desired_total - current;
            for _ in 0..to_spawn {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
                match spawn_executor(&exe, &cli) {
                    Ok(slot) => {
                        info!(worker_id = %slot.worker_id, pid = slot.pid, "spawned executor");
                        children.push(slot);
                    }
                    Err(err) => warn!(error = %err, "failed to spawn executor"),
                }
            }
        } else if current > desired_total {
            let excess = (current - desired_total) as usize;
            scale_down_idle(&mut children, &stats, excess);
        }

        sleep_interruptible(
            Duration::from_millis(cli.scale_interval_ms.max(200)),
            &shutdown,
        )
        .await;
    }

    shutdown_all_executors(&mut children).await;
    info!("zene-cloud-worker supervisor stopped");
    Ok(())
}

struct ChildSlot {
    worker_id: String,
    pid: u32,
    child: Child,
}

fn desired_children(stats: &QueueStats, min_warm: u64, max_active: u64, max_hold: u64) -> u64 {
    let capacity = max_active.saturating_sub(stats.active);
    // When at active capacity with a non-empty queue, do not keep idle claimers —
    // they would claim past max_active. Warm idles are fine when the queue is empty.
    let desired_idle = if capacity == 0 && stats.queued > 0 {
        0
    } else {
        min_warm.max(stats.queued.min(capacity))
    };
    let uncapped = stats.active + stats.holding + desired_idle;
    let ceiling = max_active + max_hold + min_warm;
    uncapped.min(ceiling)
}

fn busy_worker_ids(stats: &QueueStats) -> HashSet<String> {
    let mut ids = HashSet::new();
    for a in &stats.actives {
        ids.insert(a.worker_id.clone());
    }
    for h in &stats.holds {
        ids.insert(h.worker_id.clone());
    }
    ids
}

fn reap_finished(children: &mut Vec<ChildSlot>) {
    children.retain_mut(|slot| match slot.child.try_wait() {
        Ok(None) => true,
        Ok(Some(status)) => {
            info!(
                worker_id = %slot.worker_id,
                pid = slot.pid,
                ?status,
                "executor exited"
            );
            false
        }
        Err(err) => {
            warn!(
                worker_id = %slot.worker_id,
                pid = slot.pid,
                error = %err,
                "executor wait failed; dropping"
            );
            false
        }
    });
}

fn terminate_excess_holds(children: &mut [ChildSlot], stats: &QueueStats, max_hold: u64) {
    if stats.holding <= max_hold {
        return;
    }
    let excess = (stats.holding - max_hold) as usize;
    let mut holds = stats.holds.clone();
    holds.sort_by_key(|h| h.since);
    for hold in holds.into_iter().take(excess) {
        if let Some(slot) = children
            .iter_mut()
            .find(|c| c.worker_id == hold.worker_id)
        {
            info!(
                worker_id = %slot.worker_id,
                run_id = %hold.run_id,
                "SIGTERM oldest hold (over max_hold)"
            );
            send_sigterm(slot);
        }
    }
}

fn scale_down_idle(children: &mut Vec<ChildSlot>, stats: &QueueStats, excess: usize) {
    let busy = busy_worker_ids(stats);
    let mut killed = 0usize;
    // Prefer terminating workers that have not claimed yet (idle claimers).
    for slot in children.iter_mut() {
        if killed >= excess {
            break;
        }
        if busy.contains(&slot.worker_id) {
            continue;
        }
        info!(
            worker_id = %slot.worker_id,
            pid = slot.pid,
            "SIGTERM idle executor (scale down)"
        );
        send_sigterm(slot);
        killed += 1;
    }
}

async fn shutdown_all_executors(children: &mut Vec<ChildSlot>) {
    if children.is_empty() {
        return;
    }
    info!(count = children.len(), "supervisor shutting down executors");
    for slot in children.iter_mut() {
        send_sigterm(slot);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        reap_finished(children);
        if children.is_empty() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            warn!(
                remaining = children.len(),
                "executors did not exit in time; force killing"
            );
            for mut slot in children.drain(..) {
                let _ = slot.child.start_kill();
            }
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn send_sigterm(slot: &mut ChildSlot) {
    #[cfg(unix)]
    {
        let pid = slot.pid as i32;
        let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
        if rc != 0 {
            warn!(
                worker_id = %slot.worker_id,
                pid = slot.pid,
                "SIGTERM failed; falling back to kill"
            );
            let _ = slot.child.start_kill();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = slot.child.start_kill();
    }
}

fn spawn_executor(exe: &PathBuf, cli: &Cli) -> Result<ChildSlot> {
    let worker_id = format!("worker-{}", &Uuid::new_v4().to_string()[..8]);
    let mut cmd = Command::new(exe);
    cmd.arg("--api-url")
        .arg(&cli.api_url)
        .arg("--worker-token")
        .arg(&cli.worker_token)
        .arg("--worker-id")
        .arg(&worker_id)
        .arg("--workspace-root")
        .arg(&cli.workspace_root)
        .arg("--acp-idle-secs")
        .arg(cli.acp_idle_secs.to_string())
        .arg("--poll-seconds")
        .arg(cli.poll_seconds.to_string())
        .env("ZENE_CLOUD_WORKER_ID", &worker_id)
        .env("ZENE_CLOUD_API_URL", &cli.api_url)
        .env("ZENE_CLOUD_WORKER_TOKEN", &cli.worker_token)
        .kill_on_drop(true);

    if let Some(bin) = &cli.zene_bin {
        cmd.arg("--zene-bin").arg(bin);
    }
    if cli.acp_yolo {
        cmd.arg("--acp-yolo");
    }
    if cli.allow_mock {
        cmd.arg("--allow-mock");
    }
    // Bool clap flags don't accept `=false`; use env for the negative case.
    if cli.push_pr {
        cmd.arg("--push-pr");
    } else {
        cmd.env("ZENE_CLOUD_PUSH_PR", "false");
    }

    // Inherit stdout/stderr so executor logs appear under the supervisor session.
    let child = cmd.spawn().context("spawn executor")?;
    let pid = child.id().context("executor pid")?;
    Ok(ChildSlot {
        worker_id,
        pid,
        child,
    })
}

async fn fetch_queue_stats(client: &reqwest::Client, cli: &Cli) -> Result<QueueStats> {
    let response = client
        .get(format!("{}/internal/v1/queue/stats", cli.api_url))
        .bearer_auth(&cli.worker_token)
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_cloud_domain::QueueStats;

    fn stats(queued: u64, active: u64, holding: u64) -> QueueStats {
        QueueStats {
            queued,
            active,
            holding,
            holds: vec![],
            actives: vec![],
        }
    }

    #[test]
    fn warm_idle_when_idle() {
        assert_eq!(desired_children(&stats(0, 0, 0), 1, 4, 8), 1);
    }

    #[test]
    fn scale_for_queue() {
        assert_eq!(desired_children(&stats(3, 0, 0), 1, 4, 8), 3);
    }

    #[test]
    fn hold_does_not_block_active_capacity() {
        // 1 hold + 1 queued → need hold process + 1 claimer
        assert_eq!(desired_children(&stats(1, 0, 1), 1, 4, 8), 2);
    }

    #[test]
    fn no_idle_claimer_when_active_full_and_queued() {
        assert_eq!(desired_children(&stats(2, 4, 0), 1, 4, 8), 4);
    }

    #[test]
    fn keep_warm_when_active_full_queue_empty() {
        assert_eq!(desired_children(&stats(0, 4, 0), 1, 4, 8), 5);
    }
}

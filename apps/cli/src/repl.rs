use std::sync::Arc;

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use tokio_util::sync::CancellationToken;
use zene_core::{Agent, AgentEvent, EventHandler, PromptOptions};

use crate::{cancel_active_turn, set_active_cancel, Cli};

pub async fn run_repl(agent: &mut Agent, cli: &Cli) -> Result<()> {
    println!("Zene coding agent");
    println!("Workdir: {}", agent.session().meta.workdir);
    println!("Session: {}", agent.session().meta.id);
    println!("Type a prompt, /steer <msg> during a turn, /plan for plan mode, /cancel to abort a turn, or /exit to quit.");
    println!("Ctrl+C cancels the current turn while a prompt is running.\n");

    let mut rl = DefaultEditor::new().map_err(|err| anyhow::anyhow!(err))?;
    loop {
        let readline = rl.readline("> ");
        match readline {
            Ok(line) => {
                let input = line.trim();
                if input.is_empty() {
                    continue;
                }
                if input == "/exit" || input == "/quit" {
                    break;
                }
                if input == "/cancel" {
                    if cancel_active_turn() {
                        eprintln!("Cancelling current turn…");
                    } else {
                        eprintln!("No turn in progress.");
                    }
                    continue;
                }
                if input == "/plan" {
                    agent.enter_plan_mode();
                    eprintln!("Entered plan mode (read-only tools + ExitPlanMode).");
                    continue;
                }
                if let Some(msg) = input.strip_prefix("/steer ") {
                    let msg = msg.trim();
                    if msg.is_empty() {
                        eprintln!("Usage: /steer <message>");
                        continue;
                    }
                    match agent.steer(msg) {
                        Ok(()) => eprintln!("Steer queued ({} chars).", msg.len()),
                        Err(err) => eprintln!("Steer failed: {err:#}"),
                    }
                    continue;
                }

                rl.add_history_entry(input).ok();

                let cancel = CancellationToken::new();
                set_active_cancel(Some(cancel.clone()));

                let prompt_result = agent
                    .prompt(
                        input,
                        PromptOptions {
                            stream: !cli.no_stream,
                            cancel: Some(cancel),
                            event_handler: verbose_event_handler(cli.verbose_events),
                            quiet: false,
                        },
                    )
                    .await;

                set_active_cancel(None);

                match prompt_result {
                    Ok(_) => {}
                    Err(err) => eprintln!("Error: {err:#}"),
                }
                if !cli.quiet_usage {
                    let usage = agent.turn_usage();
                    println!(
                        "tokens: input {} / output {} ({})",
                        usage.prompt_tokens,
                        usage.completion_tokens,
                        usage.total_tokens,
                    );
                }
                println!();
            }
            Err(ReadlineError::Interrupted) => {
                if cancel_active_turn() {
                    eprintln!("\n[cancelled]");
                    continue;
                }
                break;
            }
            Err(ReadlineError::Eof) => break,
            Err(err) => return Err(err.into()),
        }
    }

    Ok(())
}

fn verbose_event_handler(enabled: bool) -> Option<EventHandler> {
    if !enabled {
        return None;
    }
    Some(Arc::new(|event| match event {
        AgentEvent::ToolCall { name, arguments } => {
            eprintln!(
                "[event] tool_call {}({})",
                name,
                truncate_for_log(&arguments, 120)
            );
        }
        AgentEvent::ToolResult {
            name,
            content,
            is_error,
            ..
        } => {
            let status = if is_error { "error" } else { "ok" };
            eprintln!(
                "[event] tool_result {} [{status}] {}",
                name,
                truncate_for_log(&content, 200)
            );
        }
        _ => {}
    }))
}

fn truncate_for_log(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        input.to_string()
    } else {
        format!("{}...", input.chars().take(max).collect::<String>())
    }
}

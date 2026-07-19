use std::sync::Arc;

use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use tokio_util::sync::CancellationToken;
use zene_core::{Agent, AgentEvent, EventHandler, PromptOptions};

use crate::{cancel_active_turn, model_config, set_active_cancel, Cli};

pub async fn run_repl(agent: &mut Agent, cli: &Cli) -> Result<()> {
    println!("Zene coding agent");
    println!("Workdir: {}", agent.session().meta.workdir);
    println!("Session: {}", agent.session().meta.id);
    println!("Type a prompt, /steer <msg> during a turn, /plan for plan mode, /cancel to abort a turn, or /exit to quit.");
    println!("Ctrl+C cancels the current turn while a prompt is running.\n");

    if let Ok(key) = agent.config().api_key() {
        if key.is_empty() {
            println!("⚠️  Warning: No API key is set for the current provider.");
            println!("   You can configure it in ~/.zene/config.toml, set environment variables,");
            println!("   or use `/model <id>`, `/models`, or `/provider` to configure.");
            println!("   If you are running a local model (e.g. Ollama), this warning can be ignored.\n");
        }
    }

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
                if input == "/compact" || input.starts_with("/compact ") {
                    let hint = input.strip_prefix("/compact").unwrap_or("").trim();
                    let hint = if hint.is_empty() { None } else { Some(hint) };
                    match agent.compact_now(hint).await {
                        Ok(Some(r)) => println!(
                            "Compacted {} messages ({} → {} tokens, reason={}). ctx={}%.",
                            r.compacted_count,
                            r.stats.tokens_before,
                            r.stats.tokens_after,
                            r.reason,
                            agent.context_water().usage_percent()
                        ),
                        Ok(None) => println!("Nothing to compact."),
                        Err(err) => eprintln!("Compact failed: {err:#}"),
                    }
                    continue;
                }
                if input == "/rewind" || input.starts_with("/rewind ") {
                    let id = input.strip_prefix("/rewind").unwrap_or("").trim();
                    let id = if id.is_empty() { None } else { Some(id) };
                    match agent.rewind_to_checkpoint(id) {
                        Ok(cp) => println!("Rewound to checkpoint {cp}."),
                        Err(err) => eprintln!("Rewind failed: {err:#}"),
                    }
                    continue;
                }
                if input == "/fork" {
                    match agent.fork_session() {
                        Ok(id) => println!("Forked session; now on {id}."),
                        Err(err) => eprintln!("Fork failed: {err:#}"),
                    }
                    continue;
                }
                if input == "/context" || input == "/tokens" {
                    println!("{}", agent.context_report());
                    continue;
                }
                if input == "/session-info" {
                    let water = agent.context_water();
                    println!(
                        "session={}\nmodel={}\ncontext={}% of {}\nmessages={}",
                        agent.session().meta.id,
                        agent.config().model,
                        water.usage_percent(),
                        agent.config().compaction.context_window_tokens,
                        agent.session().messages.len()
                    );
                    continue;
                }
                if input == "/models" {
                    println!("{}", model_config::models_help_message(agent));
                    continue;
                }
                if input == "/provider" || input == "/providers" {
                    println!("{}", model_config::providers_help_message());
                    continue;
                }
                if input == "/model" {
                    println!("{}", model_config::models_help_message(agent));
                    continue;
                }
                if let Some(args_str) = input.strip_prefix("/model ") {
                    let args_str = args_str.trim();
                    if args_str.is_empty() {
                        println!("{}", model_config::models_help_message(agent));
                        continue;
                    }
                    match handle_model_switch(agent, args_str).await {
                        Ok(_) => println!(
                            "Successfully switched to model (saved): {}",
                            agent.config().model
                        ),
                        Err(err) => eprintln!("Error switching model: {err:#}"),
                    }
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

pub async fn handle_model_switch(agent: &mut Agent, args_str: &str) -> Result<()> {
    let parts: Vec<&str> = args_str.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(());
    }

    let resolved = crate::model_config::resolve_model_args(parts[0], &parts);
    agent
        .switch_model(
            &resolved.model,
            resolved.provider,
            resolved.base_url,
            resolved.api_key,
        )
        .await?;
    Ok(())
}

fn verbose_event_handler(enabled: bool) -> Option<EventHandler> {
    if !enabled {
        return None;
    }
    Some(Arc::new(|event| match event {
        AgentEvent::ToolCall { name, arguments, .. } => {
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

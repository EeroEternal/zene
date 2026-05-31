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

    if let Ok(key) = agent.config().api_key() {
        if key.is_empty() {
            println!("⚠️  Warning: No API key is set for the current provider.");
            println!("   You can configure it in ~/.zene/config.toml, set environment variables,");
            println!("   or use the `/model` command to switch to another model or local provider.");
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
                if input == "/models" || input == "/model" {
                    println!("{}", get_models_help_message(agent));
                    continue;
                }
                if let Some(args_str) = input.strip_prefix("/model ") {
                    let args_str = args_str.trim();
                    if args_str.is_empty() {
                        println!("{}", get_models_help_message(agent));
                        continue;
                    }
                    match handle_model_switch(agent, args_str).await {
                        Ok(_) => println!("Successfully switched to model: {}", agent.config().model),
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

pub fn get_models_help_message(agent: &Agent) -> String {
    let config = agent.config();
    let mut msg = String::new();
    msg.push_str("Current Model Configuration:\n");
    msg.push_str(&format!("  Model:    {}\n", config.model));
    msg.push_str(&format!("  Provider: {}\n", config.provider));
    msg.push_str(&format!("  Base URL: {}\n", if config.provider.trim().to_lowercase() == "anthropic" {
        config.anthropic_base_url.clone().unwrap_or_else(|| "https://api.anthropic.com".to_string())
    } else {
        config.base_url.clone()
    }));

    let key_set = if config.provider.trim().to_lowercase() == "anthropic" {
        config.anthropic_api_key.as_deref().map_or(false, |k| !k.is_empty()) || std::env::var("ANTHROPIC_API_KEY").is_ok()
    } else {
        config.api_key.as_deref().map_or(false, |k| !k.is_empty()) ||
            ["DEEPSEEK_API_KEY", "ZENE_API_KEY", "OPENAI_API_KEY"].iter().any(|var| std::env::var(var).is_ok())
    };
    msg.push_str(&format!("  API Key:  {}\n", if key_set { "Configured" } else { "Not set" }));
    msg.push_str("\nTo switch model, use:\n");
    msg.push_str("  /model <preset>\n");
    msg.push_str("  /model <model_name> [provider] [base_url] [api_key]\n\n");
    msg.push_str("Presets available:\n");
    msg.push_str("  /model deepseek-chat            Switch to DeepSeek Chat\n");
    msg.push_str("  /model gpt-4o                   Switch to OpenAI GPT-4o\n");
    msg.push_str("  /model claude-3-5-sonnet        Switch to Anthropic Claude 3.5 Sonnet\n");
    msg.push_str("  /model ollama/<model_name>      Switch to local Ollama (e.g. /model ollama/qwen2.5-coder)");
    msg
}

pub async fn handle_model_switch(agent: &mut Agent, args_str: &str) -> Result<()> {
    let parts: Vec<&str> = args_str.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(());
    }

    let raw_model = parts[0];
    let mut model = raw_model.to_string();
    let mut provider = parts.get(1).map(|s| s.to_string());
    let mut base_url = parts.get(2).map(|s| s.to_string());
    let mut api_key = parts.get(3).map(|s| s.to_string());

    // Handle presets
    if raw_model == "deepseek-chat" {
        provider = Some("openai".to_string());
        base_url = Some("https://api.deepseek.com".to_string());
    } else if raw_model == "gpt-4o" {
        provider = Some("openai".to_string());
        base_url = Some("https://api.openai.com/v1".to_string());
    } else if raw_model == "claude-3-5-sonnet" || raw_model == "claude-3-5-sonnet-20241022" {
        model = "claude-3-5-sonnet-20241022".to_string();
        provider = Some("anthropic".to_string());
        base_url = Some("https://api.anthropic.com".to_string());
    } else if let Some(ollama_model) = raw_model.strip_prefix("ollama/") {
        model = ollama_model.to_string();
        provider = Some("openai".to_string());
        base_url = Some("http://localhost:11434/v1".to_string());
        if api_key.is_none() {
            api_key = Some("ollama".to_string()); // dummy key
        }
    }

    agent.switch_model(&model, provider, base_url, api_key).await?;
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

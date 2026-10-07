use serde_json::{json, Value};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker, UserMarker},
    Id,
};

use crate::{
    access::Grant,
    brand::{self, Tone},
    concurrency::Metrics,
    context::Context,
    discord::reply::{self, Responder},
    error::BotError,
    gemini::{prompt, types, GenerateParams},
    storage::AuditEntry,
    tools::{self, ToolGuard},
};

pub struct TurnRequest {
    pub guild_id: Id<GuildMarker>,
    pub channel_id: Id<ChannelMarker>,
    pub user_id: Id<UserMarker>,
    pub user_name: String,
    pub prompt: String,
    pub request_id: String,
    /// Whether the requester owns this guild (authoritative from snapshot).
    /// Lets the model distinguish "owner explicitly authorizes a redesign"
    /// from a vague destructive request by a non-owner.
    pub is_guild_owner: bool,
    /// Whether the requester is the bot operator (unrestricted key).
    pub is_bot_owner: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TurnOutcome {
    pub steps: usize,
    pub tool_calls: usize,
    pub tool_errors: usize,
}

/// Keeps the typing indicator alive for the whole turn; stops on drop.
struct TypingGuard(tokio::task::JoinHandle<()>);

impl TypingGuard {
    fn start(ctx: &Context, channel: Id<ChannelMarker>) -> Self {
        let ctx = ctx.clone();
        Self(tokio::spawn(async move {
            loop {
                let _ = ctx.http.create_typing_trigger(channel).await;
                tokio::time::sleep(Duration::from_secs(8)).await;
            }
        }))
    }
}

impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Core ReAct loop: model → tools → model … until a plain-text answer,
/// the step budget, loop detection, or the whole-turn timeout.
pub async fn run_agent_turn(
    ctx: &Context,
    req: &TurnRequest,
    grant: &Grant,
    guard: &ToolGuard,
    responder: &Responder,
) -> Result<TurnOutcome, BotError> {
    let secs = ctx.config.turn_timeout_secs.max(30);
    match tokio::time::timeout(
        Duration::from_secs(secs),
        run_inner(ctx, req, grant, guard, responder),
    )
    .await
    {
        Ok(res) => res,
        Err(_) => Err(BotError::Timeout(secs)),
    }
}

async fn run_inner(
    ctx: &Context,
    req: &TurnRequest,
    grant: &Grant,
    guard: &ToolGuard,
    responder: &Responder,
) -> Result<TurnOutcome, BotError> {
    let started = Instant::now();
    let mem_key = (req.channel_id.get(), req.user_id.get());
    let max_iterations = ctx.config.max_iterations.max(1);
    let max_output = ctx.config.max_tool_output_chars;
    let system = prompt::system_instruction(max_iterations);
    let log_channel = ctx
        .store
        .guild_config(req.guild_id.get())
        .await
        .ok()
        .and_then(|c| c.log_channel_id);

    let user_turn = prompt::user_turn(
        req.guild_id.get(),
        req.channel_id.get(),
        req.user_id.get(),
        &req.user_name,
        &req.prompt,
        req.is_guild_owner,
        req.is_bot_owner,
    );

    let mut contents = ctx.memory.history(mem_key).await;
    contents.push(user_turn.clone());
    let turn_start = contents.len();
    let mut new_entries: Vec<Value> = vec![user_turn];

    let _typing = TypingGuard::start(ctx, req.channel_id);

    let mut last_sig: Option<String> = None;
    let mut repeat_count: u32 = 0;
    let mut tool_counts: HashMap<String, usize> = HashMap::new();
    let mut outcome = TurnOutcome::default();

    for iteration in 0..max_iterations {
        outcome.steps = iteration + 1;
        let res = ctx
            .gemini
            .generate(GenerateParams {
                api_key: &grant.api_key,
                model: &grant.model,
                system: &system,
                contents: &contents,
                tools: &ctx.tools_decl,
                temperature: ctx.config.temperature,
            })
            .await?;

        let Some(candidate) = types::candidate_content(&res).cloned() else {
            return Err(
                crate::gemini::GeminiError::Malformed("no candidate content".into()).into(),
            );
        };
        let calls = types::extract_function_calls(&candidate);

        if calls.is_empty() {
            let raw = types::extract_text(&candidate)
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "Done — the model returned no further text.".to_string());
            // The model could echo a key it saw in grounded channel history.
            // Scrub before storing in memory history and posting to Discord.
            let (text, had_key) = crate::utils::scrub_google_keys(&raw);
            if had_key {
                tracing::warn!("Model output contained an API key; redacted before delivery");
            }
            new_entries.push(candidate);
            ctx.memory.record(mem_key, new_entries).await;
            let footer = format!(
                "{} · {} tool call{} · {:.1}s",
                grant.source.label(),
                outcome.tool_calls,
                if outcome.tool_calls == 1 { "" } else { "s" },
                started.elapsed().as_secs_f32()
            );
            send_answer(ctx, responder, &text, &footer).await?;
            return Ok(outcome);
        }

        if let Some(text) = types::extract_text(&candidate) {
            if !text.trim().is_empty() {
                let scrubbed = crate::utils::scrub_google_keys(&text).0;
                tracing::debug!(iteration, text = %crate::utils::truncate_chars(&scrubbed, 300), "Model reasoning alongside tool calls");
            }
        }
        tracing::info!(iteration, count = calls.len(), "Executing tool calls");

        let sig = calls
            .iter()
            .map(|(n, a)| format!("{n}:{a}"))
            .collect::<Vec<_>>()
            .join("|");
        if Some(&sig) == last_sig.as_ref() {
            repeat_count += 1;
        } else {
            last_sig = Some(sig);
            repeat_count = 1;
        }
        if repeat_count >= 3 {
            tracing::warn!(iteration, "Tool loop detected (same calls 3x), stopping");
            ctx.memory.record(mem_key, new_entries).await;
            responder
                .embed(
                    ctx,
                    brand::embed(
                        Tone::Warn,
                        "Stopped a repeating loop",
                        "I kept repeating the same action, so I stopped to save your quota. \
                         Please rephrase with exact names/IDs and try again.",
                    ),
                    false,
                )
                .await?;
            return Ok(outcome);
        }

        let parallel = calls.len() > 1 && calls.iter().all(|(n, _)| tools::is_read_only(n));
        let results: Vec<(String, bool)> = if parallel {
            futures::future::join_all(
                calls
                    .iter()
                    .map(|(n, a)| run_tool(ctx, req, grant, guard, log_channel, n, a, max_output)),
            )
            .await
        } else {
            let mut out = Vec::with_capacity(calls.len());
            for (n, a) in &calls {
                out.push(run_tool(ctx, req, grant, guard, log_channel, n, a, max_output).await);
            }
            out
        };

        let mut response_parts = Vec::with_capacity(calls.len());
        for ((name, _), (result, ok)) in calls.iter().zip(results) {
            *tool_counts.entry(name.clone()).or_insert(0) += 1;
            outcome.tool_calls += 1;
            if !ok {
                outcome.tool_errors += 1;
            }
            response_parts.push(types::function_response_part(name, &result));
        }

        contents.push(candidate.clone());
        new_entries.push(candidate);
        let tool_msg = json!({ "role": "user", "parts": response_parts });
        contents.push(tool_msg.clone());
        new_entries.push(tool_msg);

        // Bound the in-turn context. Drop whole (call, response) pairs right
        // after this turn's user prompt so pairs never get split.
        if contents.len() > 90 {
            let excess = (contents.len() - 80) & !1;
            let end = (turn_start + excess).min(contents.len().saturating_sub(2));
            if end > turn_start {
                contents.drain(turn_start..end);
                tracing::warn!(len = contents.len(), "Trimmed in-turn context window");
            }
        }
    }

    tracing::warn!(
        max = max_iterations,
        tool_calls = outcome.tool_calls,
        "Step budget exhausted"
    );
    ctx.memory.record(mem_key, new_entries).await;
    let mut parts: Vec<String> = tool_counts
        .iter()
        .map(|(k, v)| format!("{k}×{v}"))
        .collect();
    parts.sort();
    let did = if parts.is_empty() {
        String::new()
    } else {
        format!("\n\n**Done so far:** {}", parts.join(", "))
    };
    responder
        .embed(
            ctx,
            brand::embed(
                Tone::Warn,
                "Step budget used up",
                &format!(
                    "I used all {max_iterations} steps without finishing. Partial actions were applied — \
                     say `continue` to pick up where I left off.{did}"
                ),
            ),
            false,
        )
        .await?;
    Ok(outcome)
}

#[allow(clippy::too_many_arguments)]
async fn run_tool(
    ctx: &Context,
    req: &TurnRequest,
    grant: &Grant,
    guard: &ToolGuard,
    log_channel: Option<u64>,
    name: &str,
    args: &Value,
    max_output: usize,
) -> (String, bool) {
    Metrics::inc(&ctx.metrics.tool_calls);
    if let Err(denied) = guard.check(ctx, name, args).await {
        Metrics::inc(&ctx.metrics.tool_denied);
        tracing::warn!(tool = name, reason = %denied, "Tool call blocked by permission guard");
        return (denied, false);
    }
    let started = Instant::now();
    let (raw_text, ok) = match tools::execute_tool(name, args, req.guild_id, &ctx.http).await {
        Ok(out) => (out, true),
        Err(e) => (
            crate::error::friendly_tool_error(name, &e.to_string()),
            false,
        ),
    };
    // Tool outputs can contain a leaked key (e.g. get_messages reading a
    // channel where someone pasted one). Scrub BEFORE feeding back to the
    // model, writing the audit row, or posting the guild log — otherwise the
    // key propagates into memory history, the DB, and Discord.
    let (scrubbed_text, had_key) = crate::utils::scrub_google_keys(&raw_text);
    if had_key {
        tracing::warn!(
            tool = name,
            "Tool output contained an API key; redacted before reuse"
        );
    }
    let text = truncate_chars(
        &scrubbed_text,
        if ok { max_output } else { max_output.min(2000) },
    );
    let ms = started.elapsed().as_millis() as u64;
    if tools::is_destructive(name) {
        // NEVER log raw args: send_message/topic/nickname content could carry
        // a key. Log the scrubbed serialization instead.
        let args_log = crate::utils::scrub_google_keys(&args.to_string()).0;
        tracing::warn!(tool = name, ok, ms, args = %crate::utils::truncate_chars(&args_log, 500), "Destructive tool executed");
    } else {
        tracing::info!(tool = name, ok, ms, "Tool executed");
    }

    if !tools::is_read_only(name) {
        let args_scrubbed = crate::utils::scrub_google_keys(&args.to_string()).0;
        let entry = AuditEntry {
            guild_id: Some(req.guild_id.get()),
            user_id: req.user_id.get(),
            key_source: Some(grant.source.tag().to_string()),
            action: name.to_string(),
            detail: format!(
                "args={} result={}",
                crate::utils::truncate_chars(&args_scrubbed, 1000),
                crate::utils::truncate_chars(&text, 500)
            ),
            success: ok,
        };
        let ctx2 = ctx.clone();
        let log_line = format!(
            "<@{}> via {} → `{name}` {}\n{}",
            req.user_id,
            brand::SHORT_NAME,
            if ok { "succeeded" } else { "failed" },
            crate::utils::truncate_chars(&text, 500)
        );
        tokio::spawn(async move {
            if let Err(e) = ctx2.store.audit(entry).await {
                tracing::warn!(error = %e, "Failed to write audit log");
            }
            if let Some(ch) = log_channel {
                let tone = if ok { Tone::Info } else { Tone::Warn };
                reply::guild_log(&ctx2, ch, brand::embed(tone, "Agent action", &log_line)).await;
            }
        });
    }
    (text, ok)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let kept: String = s.chars().take(max_chars).collect();
    format!("{kept}\n…(truncated to {max_chars} chars)")
}

async fn send_answer(
    ctx: &Context,
    responder: &Responder,
    text: &str,
    footer: &str,
) -> Result<(), BotError> {
    let chunks = split_message(text, brand::DESCRIPTION_MAX - 50);
    let total = chunks.len();
    for (i, chunk) in chunks.into_iter().enumerate() {
        let title = if total > 1 {
            format!("{} ({}/{total})", brand::NAME, i + 1)
        } else {
            brand::NAME.to_string()
        };
        let embed = brand::embed_with_footer(Tone::Primary, &title, &chunk, footer);
        responder.embed(ctx, embed, false).await?;
    }
    Ok(())
}

pub fn split_message(text: &str, limit: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= limit {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + limit).min(chars.len());
        if end < chars.len() {
            let window = &chars[start..end];
            if let Some(rel) = window.iter().rposition(|&c| c == '\n') {
                let cut = start + rel + 1;
                if cut > start + limit / 2 {
                    end = cut;
                }
            }
        }
        out.push(chars[start..end].iter().collect());
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::split_message;

    #[test]
    fn short_message_is_single_chunk() {
        assert_eq!(split_message("hi", 1900).len(), 1);
    }

    #[test]
    fn long_message_splits() {
        let text = "a".repeat(4000);
        let chunks = split_message(&text, 1900);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.chars().count() <= 1900));
    }
}

use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use twilight_model::channel::Message;

use crate::{
    context::Context,
    gemini::{prompt, types},
    tools,
};

/// Core ReAct reasoning loop.
///
/// model -> tools -> model -> tools ... until the model answers with plain
/// text or `max_iterations` is reached (up to 100 for real-world bulk jobs).
/// Multiple `functionCall` parts per turn are all executed (sequentially)
/// before feeding results back.
///
/// Real-world rails (instead of a low hard cap):
/// - whole-turn timeout (`TURN_TIMEOUT_SECS`, default 300s)
/// - loop detection (same tool+args 3x in a row -> stop, don't burn budget)
/// - typing indicator so users see progress on long runs
/// - system prompt is prepended every turn but NEVER stored in memory
///   (the old code stored it, evicted it after ~10 turns, and the model
///   then lost its operating rules)
/// - tool output truncated to `MAX_TOOL_OUTPUT_CHARS`
pub async fn run_agent_turn(
    msg: &Message,
    prompt_text: String,
    ctx: &Context,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let timeout = Duration::from_secs(ctx.config.turn_timeout_secs.max(30));
    match tokio::time::timeout(timeout, run_agent_turn_inner(msg, prompt_text, ctx)).await {
        Ok(res) => res,
        Err(_) => {
            tracing::warn!("Agent turn timed out");
            ctx.http
                .create_message(msg.channel_id)
                .content(&format!(
                    "That took longer than {}s so I stopped to avoid hanging. Partial actions (if any) were applied — please re-ask to continue.",
                    timeout.as_secs()
                ))
                .await?;
            Ok(())
        }
    }
}

async fn run_agent_turn_inner(
    msg: &Message,
    prompt_text: String,
    ctx: &Context,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let guild_id = match msg.guild_id {
        Some(id) => id,
        None => {
            ctx.http
                .create_message(msg.channel_id)
                .content("Administrative commands can only be used inside a server.")
                .await?;
            return Ok(());
        }
    };

    let channel_id = msg.channel_id.get();
    let tools_decl = tools::build_tools_declaration();
    let max_iterations = ctx.config.max_iterations.max(1);
    let temperature = ctx.config.temperature;
    let max_output = ctx.config.max_tool_output_chars;

    let user_turn = prompt::user_turn(
        guild_id.get(),
        channel_id,
        msg.author.id.get(),
        &msg.author.name,
        &prompt_text,
    );

    // History WITHOUT system (system is prepended per-call, never stored).
    let history = ctx
        .memory
        .build_contents(channel_id, user_turn.clone())
        .await;
    let mut contents: Vec<Value> = Vec::with_capacity(history.len() + 2);
    contents.push(prompt::system_instruction(max_iterations));
    contents.extend(history);

    // Only the NEW entries of this turn get recorded (user turn + model +
    // tool responses). History is already in memory; system is excluded.
    let mut new_entries: Vec<Value> = vec![user_turn];

    // Show typing immediately so long multi-step runs don't look dead.
    let _ = ctx.http.create_typing_trigger(msg.channel_id).await;
    let mut last_tick = std::time::Instant::now();

    // Loop detection: (tool_name + canonical args) -> consecutive repeats.
    let mut last_sig: Option<String> = None;
    let mut repeat_count: u32 = 0;
    // Counters for the exhaustion summary.
    let mut tool_counts: HashMap<String, usize> = HashMap::new();
    let mut total_calls: usize = 0;

    for iteration in 0..max_iterations {
        // Refresh typing every ~12s on long runs.
        if last_tick.elapsed() > Duration::from_secs(12) {
            let _ = ctx.http.create_typing_trigger(msg.channel_id).await;
            last_tick = std::time::Instant::now();
        }

        let res = ctx
            .gemini
            .generate(&contents, &tools_decl, temperature)
            .await
            .map_err(|e| {
                tracing::error!(?e, iteration, "Gemini call failed");
                e
            })?;

        let candidate = match types::candidate_content(&res) {
            Some(c) => c.clone(),
            None => {
                tracing::warn!(?res, "Gemini returned no candidates");
                send_chunked(
                    ctx,
                    msg,
                    "I couldn't get a response from the model. Please try again.",
                )
                .await?;
                return Ok(());
            }
        };

        let calls = types::extract_function_calls(&candidate);

        // No tool calls -> final conversational reply.
        if calls.is_empty() {
            if let Some(text) = types::extract_text(&candidate) {
                send_chunked(ctx, msg, &text).await?;
                new_entries.push(candidate);
                ctx.memory.record(channel_id, new_entries).await;
            } else {
                tracing::warn!(?candidate, "Model reply had neither text nor tool calls");
                send_chunked(ctx, msg, "The model returned an empty response.").await?;
            }
            return Ok(());
        }

        // If the model returned text ALONGSIDE tool calls, keep it visible
        // in logs (previously it was silently dropped).
        if let Some(text) = types::extract_text(&candidate) {
            if !text.trim().is_empty() {
                tracing::info!(iteration, text = %text.chars().take(300).collect::<String>(), "Model thinking alongside tool calls");
            }
        }

        tracing::info!(
            iteration,
            count = calls.len(),
            "Executing tool calls requested by Gemini"
        );

        // Loop detection across the whole block signature.
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
            new_entries.push(candidate.clone());
            contents.push(candidate);
            ctx.memory.record(channel_id, new_entries).await;
            send_chunked(
                ctx,
                msg,
                "I got stuck repeating the same action, so I stopped. The last error (if any) is above — please rephrase with exact names/IDs and try again.",
            )
            .await?;
            return Ok(());
        }

        // Execute every requested call, then feed all results back at once.
        let mut response_parts = Vec::with_capacity(calls.len());
        for (name, args) in &calls {
            if tools::is_destructive(name) {
                tracing::warn!(tool = name.as_str(), "Executing destructive tool");
            } else {
                tracing::info!(tool = name.as_str(), ?args, "Executing tool call");
            }
            let result = match tools::execute_tool(name, args, guild_id, &ctx.http).await {
                Ok(out) => truncate_chars(&out, max_output),
                Err(e) => {
                    let msg = format!("Failed to execute {name}: {e}");
                    truncate_chars(&msg, max_output.min(2000))
                }
            };
            *tool_counts.entry(name.clone()).or_insert(0) += 1;
            total_calls += 1;
            response_parts.push(types::function_response_part(name, &result));
        }

        contents.push(candidate.clone());
        new_entries.push(candidate);
        let tool_msg = json!({ "role": "user", "parts": response_parts });
        contents.push(tool_msg.clone());
        new_entries.push(tool_msg);

        // Context guard: a 100-step trajectory is ~200 content blocks.
        // Keep system + recent 80 blocks so we never send an unbounded
        // payload to Gemini (which would 400). History in memory is
        // separately bounded by HISTORY_LIMIT.
        if contents.len() > 90 {
            let drain_end = contents.len() - 80;
            // Keep index 0 (system), drain oldest middle.
            contents.drain(1..drain_end);
            tracing::warn!(len = contents.len(), "Trimmed in-turn context window");
        }
    }

    // Loop exhausted without a final text answer.
    tracing::warn!(
        max = max_iterations,
        total_calls,
        "ReAct loop hit iteration cap"
    );
    ctx.memory.record(channel_id, new_entries).await;
    let summary = if tool_counts.is_empty() {
        String::new()
    } else {
        let mut parts: Vec<String> = tool_counts
            .iter()
            .map(|(k, v)| format!("{k}×{v}"))
            .collect();
        parts.sort();
        format!(" (did: {})", parts.join(", "))
    };
    send_chunked(
        ctx,
        msg,
        &format!(
            "I used all {max_iterations} steps without finishing{summary}. Partial actions (if any) were applied — please check and re-ask to continue (e.g. 'continue where you left off')."
        ),
    )
    .await?;
    Ok(())
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let kept: String = s.chars().take(max_chars).collect();
    format!("{kept}\n…(truncated to {max_chars} chars)")
}

/// Send long model output as sequential <=2000-char Discord messages.
async fn send_chunked(
    ctx: &Context,
    msg: &Message,
    text: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    const LIMIT: usize = 1900; // margin under Discord's 2000-char cap
    if text.trim().is_empty() {
        ctx.http
            .create_message(msg.channel_id)
            .content("(empty response)")
            .await?;
        return Ok(());
    }
    for chunk in split_message(text, LIMIT) {
        ctx.http
            .create_message(msg.channel_id)
            .content(&chunk)
            .await?;
    }
    Ok(())
}

fn split_message(text: &str, limit: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= limit {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = (start + limit).min(chars.len());
        // Prefer a newline boundary so code blocks don't split mid-line.
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

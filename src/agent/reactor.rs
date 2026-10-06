use serde_json::{json, Value};
use twilight_model::channel::Message;

use crate::{
    context::Context,
    gemini::{prompt, types},
    tools,
};

/// Core ReAct reasoning loop.
///
/// Replaces the original 2-turn flow with a bounded multi-step loop:
/// model -> tools -> model -> tools ... until the model answers with plain
/// text or `max_iterations` is reached. Multiple `functionCall` parts per
/// turn are all executed (sequentially) before feeding results back.
pub async fn run_agent_turn(
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

    let user_turn = prompt::user_turn(
        guild_id.get(),
        channel_id,
        msg.author.id.get(),
        &msg.author.name,
        &prompt_text,
    );

    // Seed with a system instruction on a cold channel so the model always
    // has its operating rules even with empty history.
    let mut contents = ctx
        .memory
        .build_contents(channel_id, user_turn.clone())
        .await;
    let mut new_entries: Vec<Value> = vec![user_turn];
    if contents.len() == 1 {
        let system = prompt::system_instruction();
        contents.insert(0, system.clone());
        new_entries.insert(0, system);
    }

    for iteration in 0..ctx.config.max_iterations {
        let res = if iteration == 0 {
            ctx.gemini
                .generate(&contents, &tools_decl, ctx.config.temperature)
                .await?
        } else {
            ctx.gemini
                .generate_follow_up(&contents, &tools_decl)
                .await?
        };

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

        tracing::info!(
            iteration,
            count = calls.len(),
            "Executing tool calls requested by Gemini"
        );

        // Execute every requested call, then feed all results back at once.
        let mut response_parts = Vec::with_capacity(calls.len());
        for (name, args) in &calls {
            if tools::is_destructive(name) {
                tracing::warn!(tool = name.as_str(), "Executing destructive tool");
            } else {
                tracing::info!(tool = name.as_str(), ?args, "Executing tool call");
            }
            let result = match tools::execute_tool(name, args, guild_id, &ctx.http).await {
                Ok(out) => out,
                Err(e) => format!("Failed to execute {name}: {e}"),
            };
            response_parts.push(types::function_response_part(name, &result));
        }

        contents.push(candidate.clone());
        new_entries.push(candidate);
        let tool_msg = json!({ "role": "user", "parts": response_parts });
        contents.push(tool_msg.clone());
        new_entries.push(tool_msg);
    }

    // Loop exhausted without a final text answer.
    tracing::warn!(
        max = ctx.config.max_iterations,
        "ReAct loop hit iteration cap"
    );
    ctx.memory.record(channel_id, new_entries).await;
    send_chunked(
        ctx,
        msg,
        "I ran out of steps while working on that. Partial actions (if any) were applied — please check and re-ask to continue.",
    )
    .await?;
    Ok(())
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

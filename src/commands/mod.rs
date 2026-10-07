pub mod ai;
pub mod byok;
pub mod server;
pub mod util;

use twilight_model::id::{
    marker::{ChannelMarker, GuildMarker, RoleMarker, UserMarker},
    Id,
};

use crate::{
    access::GuildSnapshot, brand::Tone, concurrency::Metrics, context::Context,
    discord::reply::Responder,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Everyone,
    GuildOwner,
    BotOwner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Ai,
    Byok,
    Server,
    Utility,
    Owner,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Ai,
        Category::Byok,
        Category::Server,
        Category::Utility,
        Category::Owner,
    ];

    pub const fn title(self) -> &'static str {
        match self {
            Category::Ai => "AI agent",
            Category::Byok => "Bring your own key",
            Category::Server => "Server settings",
            Category::Utility => "Utility",
            Category::Owner => "Bot owner",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SlashArg {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
    pub choices: &'static [&'static str],
}

/// Single source of truth for every command: prefix parsing, slash
/// registration, access control and the help menu are all derived from it.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub usage: &'static str,
    pub summary: &'static str,
    pub details: &'static str,
    pub category: Category,
    pub access: Access,
    pub guild_only: bool,
    pub slash_args: &'static [SlashArg],
}

pub const CONFIG_SETTINGS: &[&str] = &[
    "show",
    "enabled",
    "user-byok",
    "model",
    "cooldown",
    "role",
    "log-channel",
    "reset",
];

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "ai",
        aliases: &["ask", "umbraix"],
        usage: "<request>",
        summary: "Ask the agent to do something in this server",
        details: "Runs the AI agent with your Discord permissions. Examples:\n• `list channels`\n• `timeout @user 10m spamming`\n• `create a #announcements channel and post a welcome`\nDestructive actions only run when you clearly ask.",
        category: Category::Ai,
        access: Access::Everyone,
        guild_only: true,
        slash_args: &[SlashArg { name: "request", description: "What should the agent do?", required: true, choices: &[] }],
    },
    CommandSpec {
        name: "clear",
        aliases: &["reset", "forget"],
        usage: "",
        summary: "Forget your conversation with the agent in this channel",
        details: "Clears the conversation history the agent keeps for you in this channel.",
        category: Category::Ai,
        access: Access::Everyone,
        guild_only: false,
        slash_args: &[],
    },
    CommandSpec {
        name: "byok-user",
        aliases: &["byok", "setkey"],
        usage: "",
        summary: "Add your personal Gemini API key (works in every server)",
        details: "I'll DM you a secure form. Your key is verified with Google, encrypted (AES-256-GCM) and only used for your own requests. Get a free key at https://aistudio.google.com/apikey",
        category: Category::Byok,
        access: Access::Everyone,
        guild_only: false,
        slash_args: &[],
    },
    CommandSpec {
        name: "byok-server",
        aliases: &["setserverkey"],
        usage: "",
        summary: "Add a shared Gemini key for this server's staff",
        details: "Server owner only. Stores a server key and creates the access role; the owner and members with that role can then use the agent on the server's key.",
        category: Category::Byok,
        access: Access::GuildOwner,
        guild_only: true,
        slash_args: &[],
    },
    CommandSpec {
        name: "byok-status",
        aliases: &["keys", "status"],
        usage: "",
        summary: "Show which keys are set and which one you'd use here",
        details: "Shows your personal key, this server's key, the access role and which key would power your requests here. Keys are never shown — only a short hint.",
        category: Category::Byok,
        access: Access::Everyone,
        guild_only: false,
        slash_args: &[],
    },
    CommandSpec {
        name: "byok-remove",
        aliases: &["removekey"],
        usage: "<user|server>",
        summary: "Delete your personal key or this server's key",
        details: "`user` deletes your personal key everywhere. `server` (server owner only) deletes this server's shared key.",
        category: Category::Byok,
        access: Access::Everyone,
        guild_only: false,
        slash_args: &[SlashArg { name: "scope", description: "Which key to delete", required: true, choices: &["user", "server"] }],
    },
    CommandSpec {
        name: "config",
        aliases: &["settings"],
        usage: "[setting] [value]",
        summary: "View or change this server's agent settings",
        details: "Server owner only.\n• `enabled on|off` — turn the agent on/off here\n• `user-byok on|off` — allow personal keys here\n• `model <name|default>` — Gemini model\n• `cooldown <secs|default>` — per-user cooldown\n• `role <@role|name>` — access role for the server key\n• `log-channel <#channel|off>` — post agent actions there\n• `reset` — restore defaults",
        category: Category::Server,
        access: Access::GuildOwner,
        guild_only: true,
        slash_args: &[
            SlashArg { name: "setting", description: "Setting to change (omit to view)", required: false, choices: CONFIG_SETTINGS },
            SlashArg { name: "value", description: "New value", required: false, choices: &[] },
        ],
    },
    CommandSpec {
        name: "help",
        aliases: &["h", "commands"],
        usage: "[command]",
        summary: "Show this menu or details about a command",
        details: "Use `help <command>` for details.",
        category: Category::Utility,
        access: Access::Everyone,
        guild_only: false,
        slash_args: &[SlashArg { name: "command", description: "Command to explain", required: false, choices: &[] }],
    },
    CommandSpec {
        name: "about",
        aliases: &["info", "version"],
        usage: "",
        summary: "About Umbraix Agent",
        details: "Version, uptime and how access works.",
        category: Category::Utility,
        access: Access::Everyone,
        guild_only: false,
        slash_args: &[],
    },
    CommandSpec {
        name: "ping",
        aliases: &["latency"],
        usage: "",
        summary: "Check bot latency and health",
        details: "Shows Discord gateway/API latency and database health.",
        category: Category::Utility,
        access: Access::Everyone,
        guild_only: false,
        slash_args: &[],
    },
    CommandSpec {
        name: "usage",
        aliases: &["quota"],
        usage: "",
        summary: "Your (and this server's) agent usage",
        details: "Requests and tool calls today and over the last 30 days.",
        category: Category::Utility,
        access: Access::Everyone,
        guild_only: false,
        slash_args: &[],
    },
    CommandSpec {
        name: "stats",
        aliases: &["botstats"],
        usage: "",
        summary: "Runtime statistics",
        details: "Bot owner only: guilds, load, queue, Gemini retries, stored keys.",
        category: Category::Owner,
        access: Access::BotOwner,
        guild_only: false,
        slash_args: &[],
    },
];

pub fn find(name: &str) -> Option<&'static CommandSpec> {
    let name = name.to_ascii_lowercase();
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name.as_str()))
}

/// Split `!name args…` into `(name, args)`.
pub fn parse_prefixed<'a>(content: &'a str, prefix: &str) -> Option<(String, &'a str)> {
    let rest = content.trim_start().strip_prefix(prefix)?;
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut it = rest.splitn(2, char::is_whitespace);
    let name = it.next()?.to_ascii_lowercase();
    Some((name, it.next().unwrap_or("").trim()))
}

/// Everything a command handler needs, regardless of prefix vs slash.
pub struct Invocation {
    pub user_id: Id<UserMarker>,
    pub user_name: String,
    pub guild_id: Option<Id<GuildMarker>>,
    pub channel_id: Option<Id<ChannelMarker>>,
    pub member_roles: Vec<Id<RoleMarker>>,
    pub request_id: String,
    pub responder: Responder,
    pub is_slash: bool,
}

pub async fn dispatch(ctx: &Context, inv: Invocation, spec: &'static CommandSpec, args: String) {
    Metrics::inc(&ctx.metrics.commands);
    let span = tracing::info_span!(
        "command",
        rid = %inv.request_id,
        cmd = spec.name,
        user = %inv.user_id,
        guild = ?inv.guild_id.map(|g| g.get()),
        slash = inv.is_slash,
    );
    tracing::info!(parent: &span, "Command received");

    if spec.guild_only && inv.guild_id.is_none() {
        inv.responder
            .info(
                ctx,
                Tone::Warn,
                "Server only",
                &format!(
                    "`{}{}` only works inside a server.",
                    ctx.prefix(),
                    spec.name
                ),
                true,
            )
            .await;
        return;
    }
    if !check_access(ctx, &inv, spec).await {
        return;
    }

    use tracing::Instrument;
    let fut = async {
        match spec.name {
            "ai" => ai::run(ctx, inv, args).await,
            "clear" => util::clear(ctx, inv).await,
            "help" => util::help(ctx, inv, &args).await,
            "about" => util::about(ctx, inv).await,
            "ping" => util::ping(ctx, inv).await,
            "usage" => util::usage(ctx, inv).await,
            "stats" => util::stats(ctx, inv).await,
            "byok-user" => byok::start_user(ctx, inv).await,
            "byok-server" => byok::start_server(ctx, inv).await,
            "byok-status" => byok::status(ctx, inv).await,
            "byok-remove" => byok::remove(ctx, inv, &args).await,
            "config" => server::config(ctx, inv, &args).await,
            other => tracing::error!(cmd = other, "Command in registry has no handler"),
        }
    };
    fut.instrument(span).await;
}

async fn check_access(ctx: &Context, inv: &Invocation, spec: &CommandSpec) -> bool {
    let allowed = match spec.access {
        Access::Everyone => true,
        Access::BotOwner => ctx.is_owner(inv.user_id.get()),
        Access::GuildOwner => {
            if ctx.is_owner(inv.user_id.get()) {
                true
            } else if let Some(gid) = inv.guild_id {
                match GuildSnapshot::load(ctx, gid).await {
                    Ok(s) => s.owner_id == inv.user_id,
                    Err(e) => {
                        inv.responder.error(ctx, &e, None, &inv.request_id).await;
                        return false;
                    }
                }
            } else {
                false
            }
        }
    };
    if !allowed {
        let who = match spec.access {
            Access::BotOwner => "the bot owner",
            _ => "the server owner",
        };
        tracing::info!(cmd = spec.name, user = %inv.user_id, "Command denied (access)");
        inv.responder
            .info(
                ctx,
                Tone::Warn,
                "Not allowed",
                &format!("`{}{}` can only be used by {who}.", ctx.prefix(), spec.name),
                true,
            )
            .await;
    }
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_and_aliases_are_unique() {
        let mut seen = HashSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.name), "duplicate {}", c.name);
            for a in c.aliases {
                assert!(seen.insert(*a), "duplicate alias {a}");
            }
        }
    }

    #[test]
    fn slash_constraints_hold() {
        for c in COMMANDS {
            assert!(c.name.len() <= 32 && c.name == c.name.to_lowercase());
            assert!(c.summary.len() <= 100, "{} summary too long", c.name);
            for a in c.slash_args {
                assert!(a.description.len() <= 100);
                assert!(a.choices.len() <= 25);
            }
        }
    }

    #[test]
    fn parses_prefixed_commands() {
        assert_eq!(
            parse_prefixed("!ai hello there", "!"),
            Some(("ai".into(), "hello there"))
        );
        assert_eq!(
            parse_prefixed("!BYOK-USER", "!"),
            Some(("byok-user".into(), ""))
        );
        assert_eq!(parse_prefixed("! ai", "!"), None);
        assert_eq!(parse_prefixed("hello", "!"), None);
        assert!(find("ask").is_some());
        assert!(find("nope").is_none());
    }
}

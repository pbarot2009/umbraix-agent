use twilight_model::{
    application::{
        command::{Command, CommandType},
        interaction::{
            application_command::{CommandData, CommandOptionValue},
            modal::{ModalInteractionComponent, ModalInteractionData},
            Interaction, InteractionContextType, InteractionData,
        },
    },
    oauth::ApplicationIntegrationType,
};
use twilight_util::builder::command::{CommandBuilder, StringBuilder};

use crate::{
    brand::Tone,
    commands::{self, byok, Invocation, COMMANDS},
    context::Context,
    discord::reply::Responder,
    utils,
};

/// Slash command definitions derived from the command registry.
pub fn slash_commands() -> Vec<Command> {
    COMMANDS
        .iter()
        .map(|spec| {
            let contexts = if spec.guild_only {
                vec![InteractionContextType::Guild]
            } else {
                vec![InteractionContextType::Guild, InteractionContextType::BotDm]
            };
            let mut b = CommandBuilder::new(spec.name, spec.summary, CommandType::ChatInput)
                .contexts(contexts)
                .integration_types([ApplicationIntegrationType::GuildInstall]);
            for arg in spec.slash_args {
                let mut opt = StringBuilder::new(arg.name, arg.description).required(arg.required);
                if !arg.choices.is_empty() {
                    opt = opt.choices(arg.choices.iter().map(|c| (*c, *c)));
                }
                b = b.option(opt);
            }
            b.build()
        })
        .collect()
}

pub async fn register_commands(ctx: &Context) {
    let Some(app_id) = ctx.app_id.get().copied() else {
        tracing::warn!("Cannot register slash commands: application id unknown");
        return;
    };
    let cmds = slash_commands();
    let client = ctx.http.interaction(app_id);
    let res = match ctx.config.dev_guild_id {
        Some(g) => client
            .set_guild_commands(twilight_model::id::Id::new(g), &cmds)
            .await
            .map(|_| format!("guild {g}")),
        None => client
            .set_global_commands(&cmds)
            .await
            .map(|_| "global".to_string()),
    };
    match res {
        Ok(scope) => tracing::info!(count = cmds.len(), scope, "Slash commands registered"),
        Err(e) => {
            ctx.commands_registered
                .store(false, std::sync::atomic::Ordering::Relaxed);
            tracing::error!(error = %e, "Failed to register slash commands");
        }
    }
}

fn joined_args(data: &CommandData, spec: &commands::CommandSpec) -> String {
    spec.slash_args
        .iter()
        .filter_map(|arg| {
            data.options
                .iter()
                .find(|o| o.name == arg.name)
                .and_then(|o| match &o.value {
                    CommandOptionValue::String(s) => Some(s.clone()),
                    CommandOptionValue::Integer(i) => Some(i.to_string()),
                    CommandOptionValue::Boolean(b) => Some(b.to_string()),
                    _ => None,
                })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn find_text_input(components: &[ModalInteractionComponent], custom_id: &str) -> Option<String> {
    for c in components {
        let found = match c {
            ModalInteractionComponent::TextInput(t) if t.custom_id == custom_id => {
                Some(t.value.clone())
            }
            ModalInteractionComponent::Label(l) => {
                find_text_input(std::slice::from_ref(&l.component), custom_id)
            }
            ModalInteractionComponent::ActionRow(r) => find_text_input(&r.components, custom_id),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

fn modal_value(data: &ModalInteractionData, custom_id: &str) -> Option<String> {
    find_text_input(&data.components, custom_id)
}

pub async fn handle(ctx: &Context, interaction: Interaction) {
    let Some(author) = interaction.author().cloned() else {
        return;
    };
    let responder = Responder::interaction(
        interaction.application_id,
        interaction.id,
        interaction.token.clone(),
        interaction.channel.as_ref().map(|c| c.id),
    );
    let request_id = utils::short_id(8);

    match interaction.data {
        Some(InteractionData::ApplicationCommand(data)) => {
            let Some(spec) = commands::find(&data.name) else {
                responder
                    .info(
                        ctx,
                        Tone::Warn,
                        "Unknown command",
                        "This command is outdated — it will disappear shortly.",
                        true,
                    )
                    .await;
                return;
            };
            let args = joined_args(&data, spec);
            let inv = Invocation {
                user_id: author.id,
                user_name: author.name.clone(),
                guild_id: interaction.guild_id,
                channel_id: interaction.channel.as_ref().map(|c| c.id),
                member_roles: interaction
                    .member
                    .as_ref()
                    .map(|m| m.roles.clone())
                    .unwrap_or_default(),
                request_id,
                responder,
                is_slash: true,
            };
            commands::dispatch(ctx, inv, spec, args).await;
        }
        Some(InteractionData::MessageComponent(data)) => {
            if let Some(scope) = data
                .custom_id
                .strip_prefix(byok::BUTTON_PREFIX)
                .and_then(byok::parse_scope)
            {
                tracing::info!(rid = %request_id, user = %author.id, ?scope, "BYOK button pressed");
                if let Err(e) = byok::open_modal(ctx, &responder, scope).await {
                    responder.error(ctx, &e, None, &request_id).await;
                }
            }
        }
        Some(InteractionData::ModalSubmit(data)) => {
            let Some(scope) = data
                .custom_id
                .strip_prefix(byok::MODAL_PREFIX)
                .and_then(byok::parse_scope)
            else {
                return;
            };
            tracing::info!(rid = %request_id, user = %author.id, ?scope, "BYOK modal submitted");
            match modal_value(&data, byok::KEY_INPUT_ID) {
                Some(key) => byok::submit_from_modal(ctx, &responder, author.id, scope, &key).await,
                None => {
                    responder
                        .info(
                            ctx,
                            Tone::Error,
                            "Empty form",
                            "I didn't receive a key. Please try again.",
                            true,
                        )
                        .await
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_commands_cover_registry() {
        let cmds = slash_commands();
        assert_eq!(cmds.len(), COMMANDS.len());
        assert!(cmds.iter().any(|c| c.name == "byok-user"));
        assert!(cmds.iter().any(|c| c.name == "byok-server"));
    }
}

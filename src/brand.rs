use twilight_model::{channel::message::Embed, util::Timestamp};
use twilight_util::builder::embed::{EmbedBuilder, EmbedFooterBuilder};

pub const NAME: &str = "Umbraix Agent";
pub const SHORT_NAME: &str = "Umbraix";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const TAGLINE: &str = "AI-powered Discord server administration";

pub const COLOR_PRIMARY: u32 = 0x7C3AED;
pub const COLOR_SUCCESS: u32 = 0x22C55E;
pub const COLOR_WARN: u32 = 0xF59E0B;
pub const COLOR_ERROR: u32 = 0xEF4444;
pub const COLOR_INFO: u32 = 0x3B82F6;

pub const TITLE_MAX: usize = 256;
pub const DESCRIPTION_MAX: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Success,
    Warn,
    Error,
    Info,
}

impl Tone {
    pub const fn color(self) -> u32 {
        match self {
            Tone::Primary => COLOR_PRIMARY,
            Tone::Success => COLOR_SUCCESS,
            Tone::Warn => COLOR_WARN,
            Tone::Error => COLOR_ERROR,
            Tone::Info => COLOR_INFO,
        }
    }

    const fn icon(self) -> &'static str {
        match self {
            Tone::Primary => "◆",
            Tone::Success => "✔",
            Tone::Warn => "⚠",
            Tone::Error => "✖",
            Tone::Info => "ℹ",
        }
    }
}

pub fn footer_text() -> String {
    format!("{NAME} v{VERSION}")
}

fn clip(s: &str, max: usize) -> String {
    // Single pass: collect up to `max` chars, tracking whether truncated.
    let mut end = 0;
    let mut truncated = false;
    for (count, (i, c)) in s.char_indices().enumerate() {
        if count >= max {
            truncated = true;
            break;
        }
        end = i + c.len_utf8();
    }
    if !truncated && end == s.len() {
        return s.to_string();
    }
    // We collected `max` chars but there is more — replace last char with ….
    // Re-slice to max-1 chars then push ellipsis to stay within `max`.
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    let mut out = kept;
    out.push('…');
    out
}

pub fn builder(tone: Tone, title: &str, description: &str) -> EmbedBuilder {
    let clipped_title = clip(title, TITLE_MAX.saturating_sub(2));
    let full_title = if clipped_title.trim().is_empty() {
        tone.icon().to_string()
    } else {
        format!("{} {clipped_title}", tone.icon())
    };
    let mut b = EmbedBuilder::new()
        .color(tone.color())
        .title(clip(&full_title, TITLE_MAX))
        .footer(EmbedFooterBuilder::new(footer_text()));
    if !description.trim().is_empty() {
        b = b.description(clip(description, DESCRIPTION_MAX));
    }
    if let Ok(ts) = Timestamp::from_secs(crate::utils::now_secs()) {
        b = b.timestamp(ts);
    }
    b
}

pub fn embed(tone: Tone, title: &str, description: &str) -> Embed {
    builder(tone, title, description).build()
}

pub fn embed_with_footer(tone: Tone, title: &str, description: &str, footer: &str) -> Embed {
    builder(tone, title, description)
        .footer(EmbedFooterBuilder::new(clip(
            &format!("{} · {footer}", footer_text()),
            2048,
        )))
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_is_clipped() {
        let long = "x".repeat(10_000);
        let e = embed(Tone::Info, &long, &long);
        assert!(e.title.unwrap().chars().count() <= TITLE_MAX);
        assert!(e.description.unwrap().chars().count() <= DESCRIPTION_MAX);
    }

    #[test]
    fn footer_has_version() {
        assert!(footer_text().contains(VERSION));
        assert!(footer_text().contains("Umbraix"));
    }
}

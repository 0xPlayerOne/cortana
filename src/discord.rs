//! Bounded, read-only Discord guild and channel discovery.
//!
//! Discord sources are authenticated with a bot token read only from the
//! configured `token_env` name. There is intentionally no browser OAuth for
//! Discord: the operator creates the bot in the Discord developer portal and
//! stores its token in the private environment source. This module talks to
//! Discord REST v10 solely to enumerate guilds and their channels for Desktop
//! selection. It never reads message content, never starts a sync, and never
//! prints or stores the bot token. Every response is byte-bounded and every
//! emitted id is a validated snowflake; guild and channel names are sanitized
//! to printable, bounded text before they cross the process boundary.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::{Client, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::config::{Config, SourceConfig};

const API_BASE: &str = "https://discord.com/api/v10";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_GUILDS: usize = 100;
const MAX_CHANNELS_PER_GUILD: usize = 100;
const MAX_NAME_CHARS: usize = 100;
const MAX_SNOWFLAKE_CHARS: usize = 20;
const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;

#[derive(Debug, Serialize)]
pub struct ChannelList {
    pub guilds: Vec<GuildChannels>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct GuildChannels {
    pub id: String,
    pub name: String,
    pub channels: Vec<ChannelSummary>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ChannelSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Deserialize)]
struct ApiGuild {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiChannel {
    id: String,
    name: String,
    #[serde(rename = "type")]
    channel_type: u64,
}

/// List guilds and channels visible to a configured Discord source without
/// reading any message content. The result is bounded to 100 guilds with at
/// most 100 channels each and is safe to serialize into the renderer.
pub async fn list_channels(config: &Config, selected: &str) -> Result<ChannelList> {
    validate_source_name(selected)?;
    let source = configured_discord_source(config, selected)?;
    let token = bot_token(config, source)?;
    let client = discord_client()?;

    let guilds_url = format!("{API_BASE}/users/@me/guilds?limit={MAX_GUILDS}");
    let guilds: Vec<ApiGuild> = get_json(&client, &guilds_url, &token).await?;
    let truncated = guilds.len() >= MAX_GUILDS;

    let mut discovered = Vec::new();
    for guild in guilds.into_iter().take(MAX_GUILDS) {
        let id = validate_snowflake(&guild.id, "guild")?;
        let name = sanitize_name(&guild.name, "guild")?;
        let channels_url = format!("{API_BASE}/guilds/{id}/channels");
        let channels: Vec<ApiChannel> = get_json(&client, &channels_url, &token).await?;
        let channels_truncated = channels.len() >= MAX_CHANNELS_PER_GUILD;
        let mut summaries = Vec::new();
        for channel in channels.into_iter().take(MAX_CHANNELS_PER_GUILD) {
            summaries.push(ChannelSummary {
                id: validate_snowflake(&channel.id, "channel")?,
                name: sanitize_name(&channel.name, "channel")?,
                kind: channel_kind(channel.channel_type).to_string(),
            });
        }
        discovered.push(GuildChannels {
            id,
            name,
            channels: summaries,
            truncated: channels_truncated,
        });
    }

    Ok(ChannelList {
        guilds: discovered,
        truncated,
    })
}

async fn get_json<T: DeserializeOwned>(client: &Client, url: &str, token: &str) -> Result<T> {
    let response = client
        .get(url)
        .header("Authorization", format!("Bot {token}"))
        .send()
        .await
        .context("request Discord channel discovery")?;
    let status = response.status();
    anyhow::ensure!(
        status.is_success(),
        "Discord channel discovery request failed with status {}",
        status.as_u16()
    );
    bounded_json(response).await
}

async fn bounded_json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        bail!("Discord response exceeded the safety limit")
    }
    let bytes = response.bytes().await.context("read Discord response")?;
    anyhow::ensure!(
        bytes.len() <= MAX_RESPONSE_BYTES,
        "Discord response exceeded the safety limit"
    );
    serde_json::from_slice(&bytes).context("Discord returned invalid JSON")
}

fn discord_client() -> Result<Client> {
    Client::builder()
        .redirect(Policy::none())
        .timeout(HTTP_TIMEOUT)
        .user_agent(format!("cortana/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build Discord API client")
}

/// Discord discovery requires the bot token environment variable. The token
/// is never stored on this source and never appears in errors or output; only
/// the configured environment-variable name is ever surfaced.
fn bot_token(config: &Config, source: &SourceConfig) -> Result<String> {
    let name = source
        .token_env
        .as_deref()
        .context("Discord source requires a bot token environment variable")?;
    let token = config.environment_value(name).with_context(|| {
        format!("Discord bot token environment variable {name} is not configured")
    })?;
    let token = token.trim().to_string();
    anyhow::ensure!(
        !token.is_empty() && token.len() <= MAX_CREDENTIAL_BYTES,
        "Discord bot token environment variable {name} is invalid"
    );
    anyhow::ensure!(
        !token
            .chars()
            .any(|character| character.is_control() || character.is_whitespace()),
        "Discord bot token environment variable {name} is invalid"
    );
    Ok(token)
}

/// Discord snowflakes are decimal 64-bit unsigned ids. Keep them as strings:
/// renderer numbers cannot represent ids above 2^53 exactly.
pub(crate) fn validate_snowflake(value: &str, label: &str) -> Result<String> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= MAX_SNOWFLAKE_CHARS
            && value.bytes().all(|byte| byte.is_ascii_digit()),
        "Discord {label} returned an invalid id"
    );
    let parsed = value
        .parse::<u64>()
        .context("Discord returned an invalid id")?;
    anyhow::ensure!(parsed > 0, "Discord {label} returned an invalid id");
    Ok(value.to_string())
}

pub(crate) fn sanitize_name(value: &str, label: &str) -> Result<String> {
    let sanitized = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    anyhow::ensure!(
        !sanitized.is_empty(),
        "Discord {label} returned an empty name"
    );
    anyhow::ensure!(
        sanitized.chars().count() <= MAX_NAME_CHARS,
        "Discord {label} returned an oversized name"
    );
    Ok(sanitized)
}

fn channel_kind(channel_type: u64) -> &'static str {
    match channel_type {
        0 => "text",
        2 => "voice",
        4 => "category",
        5 => "announcement",
        10 => "announcement-thread",
        11 => "public-thread",
        12 => "private-thread",
        13 => "stage",
        14 => "directory",
        15 => "forum",
        16 => "media",
        _ => "other",
    }
}

fn validate_source_name(value: &str) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty() && value.len() <= 64,
        "source name is invalid"
    );
    anyhow::ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "source name is invalid"
    );
    Ok(())
}

pub(crate) fn configured_discord_source<'a>(
    config: &'a Config,
    selected: &str,
) -> Result<&'a SourceConfig> {
    validate_source_name(selected)?;
    let source = config
        .sources
        .iter()
        .find(|source| source.name == selected)
        .with_context(|| format!("configured source {selected} was not found"))?;
    anyhow::ensure!(
        source.kind == "discord",
        "source {} is not a Discord connector",
        source.name
    );
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SourceConfig;

    fn source(kind: &str, token_env: Option<&str>) -> SourceConfig {
        SourceConfig {
            name: "community".into(),
            kind: kind.into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: token_env.map(str::to_string),
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        }
    }

    fn config_with(source: SourceConfig, environment: &[(&str, &str)]) -> Config {
        let mut config = Config::default();
        config.sources.push(source);
        config.environment = environment
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();
        config
    }

    #[test]
    fn discovery_requires_an_exact_discord_connector() {
        let config = config_with(source("slack", Some("SLACK_BOT_TOKEN")), &[]);
        let error = configured_discord_source(&config, "community")
            .expect_err("slack sources must not be discoverable as Discord");
        assert!(error.to_string().contains("not a Discord connector"));

        let config = config_with(source("discord", Some("DISCORD_BOT_TOKEN")), &[]);
        configured_discord_source(&config, "community").expect("discord source matches");
    }

    #[test]
    fn missing_sources_and_names_fail_closed() {
        let config = Config::default();
        assert!(configured_discord_source(&config, "missing").is_err());
        assert!(validate_source_name("").is_err());
        assert!(validate_source_name("../community").is_err());
        assert!(validate_source_name("community-2").is_ok());
    }

    #[test]
    fn bot_token_requires_the_configured_environment_variable() {
        let config = config_with(source("discord", Some("DISCORD_BOT_TOKEN")), &[]);
        let error = bot_token(&config, &config.sources[0])
            .expect_err("a missing environment value must fail closed");
        let message = error.to_string();
        assert!(
            message.contains("DISCORD_BOT_TOKEN"),
            "errors must name the environment variable: {message}"
        );
        assert!(
            !message.contains("supersecret"),
            "errors must never include token values"
        );

        let config = config_with(
            source("discord", Some("DISCORD_BOT_TOKEN")),
            &[("DISCORD_BOT_TOKEN", "supersecret-bot-token")],
        );
        assert_eq!(
            bot_token(&config, &config.sources[0]).expect("configured token"),
            "supersecret-bot-token"
        );

        let config = config_with(
            source("discord", Some("DISCORD_BOT_TOKEN")),
            &[("DISCORD_BOT_TOKEN", " bad token "), ("OTHER", "x")],
        );
        let error = bot_token(&config, &config.sources[0])
            .expect_err("whitespace in a token must fail closed");
        assert!(!error.to_string().contains("bad token"));
    }

    #[test]
    fn discord_source_without_token_env_fails_closed() {
        let config = config_with(source("discord", None), &[]);
        let error = bot_token(&config, &config.sources[0]).expect_err("token env is required");
        assert!(error.to_string().contains("token environment variable"));
    }

    #[test]
    fn snowflake_ids_are_strictly_validated() {
        assert_eq!(
            validate_snowflake("175928847299117063", "guild").expect("valid snowflake"),
            "175928847299117063"
        );
        assert_eq!(
            validate_snowflake("18446744073709551615", "channel").expect("max u64 snowflake"),
            "18446744073709551615"
        );
        for invalid in [
            "",
            "0",
            "123abc",
            "abc123",
            "12.5",
            "-123",
            " 123",
            "123\n456",
            "18446744073709551616",
            "123456789012345678901",
        ] {
            assert!(
                validate_snowflake(invalid, "guild").is_err(),
                "snowflake {invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn names_are_sanitized_and_bounded() {
        assert_eq!(
            sanitize_name("  Engineering  ", "guild").expect("trimmed name"),
            "Engineering"
        );
        assert_eq!(
            sanitize_name("an\u{0}noun\u{7f}cements", "channel")
                .expect("control characters are stripped"),
            "announcements"
        );
        let maximum = "x".repeat(MAX_NAME_CHARS);
        assert_eq!(
            sanitize_name(&maximum, "guild").expect("maximum length name"),
            maximum
        );
        for invalid in ["   ", "\u{0}", "\n\t", &"x".repeat(MAX_NAME_CHARS + 1)] {
            assert!(
                sanitize_name(invalid, "guild").is_err(),
                "name {invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn channel_kinds_are_mapped_to_fixed_labels() {
        assert_eq!(channel_kind(0), "text");
        assert_eq!(channel_kind(2), "voice");
        assert_eq!(channel_kind(4), "category");
        assert_eq!(channel_kind(5), "announcement");
        assert_eq!(channel_kind(10), "announcement-thread");
        assert_eq!(channel_kind(11), "public-thread");
        assert_eq!(channel_kind(12), "private-thread");
        assert_eq!(channel_kind(13), "stage");
        assert_eq!(channel_kind(14), "directory");
        assert_eq!(channel_kind(15), "forum");
        assert_eq!(channel_kind(16), "media");
        assert_eq!(channel_kind(999), "other");
    }

    #[test]
    fn serialized_discovery_never_contains_credentials() {
        let discovery = ChannelList {
            guilds: vec![GuildChannels {
                id: "175928847299117063".into(),
                name: "Engineering".into(),
                channels: vec![ChannelSummary {
                    id: "175928847299117064".into(),
                    name: "release".into(),
                    kind: "text".into(),
                }],
                truncated: false,
            }],
            truncated: false,
        };
        let serialized = serde_json::to_string(&discovery).expect("serialize discovery");
        assert!(serialized.contains("Engineering"));
        assert!(!serialized.contains("Bot "));
        assert!(!serialized.contains("token"));
    }
}

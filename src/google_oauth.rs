use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration as ChronoDuration, Utc};
use reqwest::{Client, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use uuid::Uuid;

use crate::config::{Config, SourceConfig};

const AUTHORIZATION_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_CLIENT_FILE_BYTES: u64 = 64 * 1024;
const MAX_CALLBACK_BYTES: usize = 8 * 1024;
const MAX_CALLBACK_CONNECTIONS: usize = 20;
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";
const GMAIL_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";

#[derive(Debug, Serialize)]
pub struct AuthorizationOutcome {
    pub source: String,
    pub project: String,
    pub scopes: Vec<String>,
    pub token_path: String,
    pub authorized: bool,
}

#[derive(Debug, Deserialize)]
struct ClientFile {
    installed: InstalledClient,
}

#[derive(Debug, Deserialize)]
struct InstalledClient {
    client_id: String,
    client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    scope: Option<String>,
    token_type: String,
}

#[derive(Debug, Deserialize)]
struct ExistingToken {
    refresh_token: Option<String>,
}

#[derive(Debug, Serialize)]
struct StoredToken<'a> {
    token: &'a str,
    access_token: &'a str,
    refresh_token: &'a str,
    token_uri: &'static str,
    client_id: &'a str,
    client_secret: &'a str,
    scopes: &'a [String],
    token_type: &'a str,
    expiry: String,
}

pub async fn authorize(config: &Config, selected: &str) -> Result<AuthorizationOutcome> {
    validate_source_name(selected)?;
    let source = config
        .sources
        .iter()
        .find(|source| source.name == selected)
        .with_context(|| format!("configured source {selected} was not found"))?;
    anyhow::ensure!(
        is_google_source(&source.kind),
        "source {} is not a Google connector",
        source.name
    );
    let token_path = configured_token_path(config, source)?;
    let client_path = required_secure_path(source, source.oauth_client.as_ref(), "OAuth client")?;
    ensure_outside_filesystem_roots(config, &token_path, "token")?;
    ensure_outside_filesystem_roots(config, client_path, "OAuth client")?;
    anyhow::ensure!(
        token_path.as_path() != client_path,
        "Google token and OAuth client paths must be different"
    );
    let client = read_client_file(client_path)?;
    let scopes = scopes_for_token(config, &token_path)?;
    let existing_refresh_token = read_existing_refresh_token(&token_path)?;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("bind Google OAuth loopback callback")?;
    let port = listener
        .local_addr()
        .context("read Google OAuth callback address")?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state = random_secret();
    let verifier = random_secret();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let authorization_url = authorization_url(
        &client.client_id,
        &redirect_uri,
        &scopes,
        &state,
        &challenge,
    )?;

    open::that_detached(authorization_url.as_str())
        .context("open Google authorization in the system browser")?;
    let code = wait_for_callback(listener, &state).await?;
    let token = exchange_code(&client, &redirect_uri, &verifier, &code).await?;
    verify_granted_scopes(&scopes, token.scope.as_deref())?;
    let refresh_token = token
        .refresh_token
        .as_deref()
        .or(existing_refresh_token.as_deref())
        .context(
            "Google did not return a refresh token; revoke the prior grant and authorize again",
        )?;
    anyhow::ensure!(
        token.token_type.eq_ignore_ascii_case("bearer"),
        "Google returned an unsupported token type"
    );
    validate_credential("Google access token", &token.access_token, 16 * 1024)?;
    validate_credential("Google refresh token", refresh_token, 16 * 1024)?;

    let stored = StoredToken {
        token: &token.access_token,
        access_token: &token.access_token,
        refresh_token,
        token_uri: TOKEN_ENDPOINT,
        client_id: &client.client_id,
        client_secret: client.client_secret.as_deref().unwrap_or(""),
        scopes: &scopes,
        token_type: "Bearer",
        expiry: (Utc::now()
            + ChronoDuration::seconds(i64::try_from(token.expires_in).unwrap_or(3600)))
        .to_rfc3339(),
    };
    write_token(&token_path, &stored)?;

    Ok(AuthorizationOutcome {
        source: source.name.clone(),
        project: source.project.clone(),
        scopes,
        token_path: token_path.display().to_string(),
        authorized: true,
    })
}

fn read_client_file(path: &Path) -> Result<InstalledClient> {
    reject_symlink(path)?;
    reject_symlink_components(path)?;
    let metadata =
        fs::metadata(path).with_context(|| format!("inspect OAuth client {}", path.display()))?;
    anyhow::ensure!(metadata.is_file(), "OAuth client must be a regular file");
    ensure_owner_only(&metadata, "OAuth client")?;
    anyhow::ensure!(
        metadata.len() <= MAX_CLIENT_FILE_BYTES,
        "OAuth client file exceeds 64 KiB"
    );
    let body = fs::read(path).with_context(|| format!("read OAuth client {}", path.display()))?;
    let client: ClientFile = serde_json::from_slice(&body).context(
        "OAuth client JSON must contain credentials for a Google Desktop app under `installed`",
    )?;
    validate_credential("Google client ID", &client.installed.client_id, 1024)?;
    if let Some(secret) = client.installed.client_secret.as_deref() {
        validate_credential("Google client secret", secret, 4096)?;
    }
    Ok(client.installed)
}

fn read_existing_refresh_token(path: &Path) -> Result<Option<String>> {
    reject_symlink(path)?;
    reject_symlink_components(path)?;
    if !path.exists() {
        return Ok(None);
    }
    let metadata =
        fs::metadata(path).with_context(|| format!("inspect Google token {}", path.display()))?;
    anyhow::ensure!(metadata.is_file(), "Google token must be a regular file");
    ensure_owner_only(&metadata, "Google token")?;
    anyhow::ensure!(
        metadata.len() <= MAX_CLIENT_FILE_BYTES,
        "Google token file exceeds 64 KiB"
    );
    let token: ExistingToken = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read Google token {}", path.display()))?,
    )
    .context("existing Google token JSON is invalid")?;
    if let Some(refresh_token) = token.refresh_token.as_deref() {
        validate_credential("Google refresh token", refresh_token, 16 * 1024)?;
    }
    Ok(token.refresh_token)
}

fn required_secure_path<'a>(
    source: &SourceConfig,
    value: Option<&'a PathBuf>,
    label: &str,
) -> Result<&'a Path> {
    let path = value
        .map(PathBuf::as_path)
        .with_context(|| format!("Google source {} requires {label} path", source.name))?;
    anyhow::ensure!(
        path.is_absolute()
            && path.parent().is_some()
            && path
                .parent()
                .is_some_and(|parent| parent.parent().is_some())
            && !path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            }),
        "Google source {} {label} path must be absolute and outside the filesystem root",
        source.name
    );
    Ok(path)
}

fn configured_token_path(config: &Config, source: &SourceConfig) -> Result<PathBuf> {
    if let Some(path) = source.token.as_ref() {
        return Ok(required_secure_path(source, Some(path), "token")?.to_path_buf());
    }
    let token_env = source
        .token_env
        .as_deref()
        .filter(|name| !name.is_empty())
        .with_context(|| {
            format!(
                "Google source {} requires a token file or token path environment variable",
                source.name
            )
        })?;
    let value = config.environment_value(token_env).with_context(|| {
        format!("Google token path environment variable {token_env} is not set")
    })?;
    let path = PathBuf::from(value);
    Ok(required_secure_path(source, Some(&path), "token")?.to_path_buf())
}

fn ensure_outside_filesystem_roots(config: &Config, path: &Path, label: &str) -> Result<()> {
    for source in config.sources.iter().filter(|source| {
        source.kind == "filesystem" && source.root.as_deref().is_some_and(Path::is_absolute)
    }) {
        let root = source.root.as_deref().expect("filtered filesystem root");
        anyhow::ensure!(
            !path.starts_with(root),
            "Google {label} path must be outside filesystem source {}",
            source.name
        );
    }
    Ok(())
}

fn scopes_for_token(config: &Config, token_path: &Path) -> Result<Vec<String>> {
    let mut scopes = Vec::new();
    for source in config.sources.iter().filter(|source| {
        source.token.as_deref() == Some(token_path)
            || source
                .token_env
                .as_deref()
                .and_then(|name| config.environment_value(name))
                .is_some_and(|value| Path::new(&value) == token_path)
    }) {
        let scope = match source.kind.as_str() {
            "google-drive" => DRIVE_SCOPE,
            "gmail" => GMAIL_SCOPE,
            "google-calendar" => CALENDAR_SCOPE,
            _ => continue,
        };
        if !scopes.iter().any(|existing| existing == scope) {
            scopes.push(scope.to_string());
        }
    }
    scopes.sort();
    anyhow::ensure!(
        !scopes.is_empty(),
        "no Google scopes were configured for this token"
    );
    Ok(scopes)
}

fn authorization_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &[String],
    state: &str,
    challenge: &str,
) -> Result<Url> {
    let mut url = Url::parse(AUTHORIZATION_ENDPOINT)?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &scopes.join(" "))
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("include_granted_scopes", "true");
    Ok(url)
}

async fn wait_for_callback(listener: TcpListener, expected_state: &str) -> Result<String> {
    timeout(CALLBACK_TIMEOUT, async {
        for _ in 0..MAX_CALLBACK_CONNECTIONS {
            let (mut stream, peer) = listener
                .accept()
                .await
                .context("accept Google OAuth callback")?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let target = read_request_target(&mut stream).await?;
            match parse_callback_target(&target, expected_state) {
                Ok(Some(code)) => {
                    respond(
                        &mut stream,
                        "200 OK",
                        "Authorization received. Return to Cortana while setup completes.",
                    )
                    .await?;
                    return Ok(code);
                }
                Ok(None) => {
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "Invalid authorization callback.",
                    )
                    .await?;
                }
                Err(error) => {
                    respond(
                        &mut stream,
                        "400 Bad Request",
                        "Authorization was not completed.",
                    )
                    .await?;
                    return Err(error);
                }
            }
        }
        anyhow::bail!("too many invalid Google OAuth callbacks")
    })
    .await
    .map_err(|_| anyhow::anyhow!("Google authorization timed out after 5 minutes"))?
}

async fn read_request_target(stream: &mut TcpStream) -> Result<String> {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .context("read Google OAuth callback")?;
        anyhow::ensure!(read > 0, "Google OAuth callback closed early");
        request.extend_from_slice(&buffer[..read]);
        anyhow::ensure!(
            request.len() <= MAX_CALLBACK_BYTES,
            "Google OAuth callback exceeded 8 KiB"
        );
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&request).context("Google OAuth callback was not UTF-8")?;
    let line = request
        .lines()
        .next()
        .context("Google OAuth callback was empty")?;
    let mut parts = line.split_ascii_whitespace();
    anyhow::ensure!(
        parts.next() == Some("GET"),
        "Google OAuth callback must use GET"
    );
    let target = parts
        .next()
        .context("Google OAuth callback target is missing")?;
    anyhow::ensure!(
        parts
            .next()
            .is_some_and(|version| version.starts_with("HTTP/1.")),
        "Google OAuth callback has an invalid HTTP version"
    );
    anyhow::ensure!(
        target.len() <= 4096,
        "Google OAuth callback target is too long"
    );
    Ok(target.to_string())
}

fn parse_callback_target(target: &str, expected_state: &str) -> Result<Option<String>> {
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .context("Google OAuth callback URL is invalid")?;
    if url.path() != "/callback" {
        return Ok(None);
    }
    let mut state = None;
    let mut code = None;
    let mut error = None;
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "state" => {
                anyhow::ensure!(state.is_none(), "Google OAuth callback repeated state");
                state = Some(value.into_owned());
            }
            "code" => {
                anyhow::ensure!(code.is_none(), "Google OAuth callback repeated code");
                code = Some(value.into_owned());
            }
            "error" => {
                anyhow::ensure!(error.is_none(), "Google OAuth callback repeated error");
                error = Some(value.into_owned());
            }
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Ok(None);
    }
    if let Some(error) = error {
        validate_credential("Google OAuth error", &error, 256)?;
        anyhow::bail!("Google authorization was not granted ({error})");
    }
    let code = code.context("Google OAuth callback did not contain a code")?;
    validate_credential("Google authorization code", &code, 4096)?;
    Ok(Some(code))
}

async fn respond(stream: &mut TcpStream, status: &str, message: &str) -> Result<()> {
    let body = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>Cortana</title><style>body{{font:16px system-ui;background:#1b1c1a;color:#efeee8;display:grid;min-height:90vh;place-items:center}}main{{max-width:34rem;padding:2rem;border:1px solid #454640;background:#242521}}</style><main><h1>Cortana</h1><p>{message}</p></main>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .context("respond to Google OAuth callback")
}

async fn exchange_code(
    client: &InstalledClient,
    redirect_uri: &str,
    verifier: &str,
    code: &str,
) -> Result<TokenResponse> {
    let http = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(Policy::none())
        .build()?;
    let mut form = vec![
        ("client_id", client.client_id.as_str()),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(secret) = client.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    let response = http
        .post(TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .context("exchange Google authorization code")?;
    anyhow::ensure!(
        response.status().is_success(),
        "Google token exchange failed with status {}",
        response.status().as_u16()
    );
    response
        .json()
        .await
        .context("Google token response was invalid")
}

fn verify_granted_scopes(requested: &[String], granted: Option<&str>) -> Result<()> {
    let Some(granted) = granted else {
        return Ok(());
    };
    let granted = granted.split_ascii_whitespace().collect::<Vec<_>>();
    for scope in requested {
        anyhow::ensure!(
            granted.contains(&scope.as_str()),
            "Google did not grant required scope {scope}"
        );
    }
    Ok(())
}

fn write_token(path: &Path, token: &StoredToken<'_>) -> Result<()> {
    reject_symlink(path)?;
    reject_symlink_components(path)?;
    let parent = path.parent().context("Google token path has no parent")?;
    let created_parent = !parent.exists();
    fs::create_dir_all(parent)
        .with_context(|| format!("create Google token directory {}", parent.display()))?;
    if created_parent {
        set_directory_owner_only(parent)?;
    }
    if path.exists() {
        let backup = path.with_extension("json.backup");
        reject_symlink(&backup)?;
        fs::copy(path, &backup)
            .with_context(|| format!("back up Google token {}", path.display()))?;
        set_owner_only(&backup)?;
    }
    let body = serde_json::to_vec_pretty(token)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".cortana-google-token-{}-{nonce}.tmp",
        std::process::id()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("create Google token {}", temporary.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
            .with_context(|| format!("replace Google token {}", path.display()))?;
        set_owner_only(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn validate_source_name(value: &str) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= 64
            && value.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || matches!(character, '-' | '_')
            })
            && value.chars().next().is_some_and(
                |character| character.is_ascii_lowercase() || character.is_ascii_digit()
            ),
        "source name is invalid"
    );
    Ok(())
}

fn validate_credential(label: &str, value: &str, maximum_bytes: usize) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= maximum_bytes
            && !value.contains(['\0', '\n', '\r'])
            && value.trim() == value,
        "{label} is invalid"
    );
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "refusing to use symlinked file {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut current = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::ensure!(
                    is_allowed_system_alias(&current),
                    "refusing to use symlinked path component {}",
                    current.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    Ok(())
}

fn is_allowed_system_alias(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        // macOS exposes these standard locations as symlinks into /private.
        path == Path::new("/tmp") || path == Path::new("/var") || path == Path::new("/etc")
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

fn random_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn is_google_source(kind: &str) -> bool {
    matches!(kind, "google-drive" | "gmail" | "google-calendar")
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_directory_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_directory_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

fn ensure_owner_only(metadata: &fs::Metadata, label: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        anyhow::ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "{label} file must not be accessible by group or others"
        );
    }
    let _ = (metadata, label);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_url_uses_pkce_loopback_and_minimal_scopes() {
        let scopes = vec![GMAIL_SCOPE.to_string()];
        let url = authorization_url(
            "client.apps.googleusercontent.com",
            "http://127.0.0.1:43210/callback",
            &scopes,
            "expected-state",
            "challenge",
        )
        .unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("accounts.google.com"));
        let query = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(query.get("code_challenge_method").unwrap(), "S256");
        assert_eq!(query.get("access_type").unwrap(), "offline");
        assert_eq!(query.get("scope").unwrap(), GMAIL_SCOPE);
        assert_eq!(
            query.get("redirect_uri").unwrap(),
            "http://127.0.0.1:43210/callback"
        );
    }

    #[test]
    fn callback_requires_exact_state_and_path() {
        assert_eq!(
            parse_callback_target("/callback?state=state&code=code", "state").unwrap(),
            Some("code".into())
        );
        assert!(
            parse_callback_target("/callback?state=wrong&code=code", "state")
                .unwrap()
                .is_none()
        );
        assert!(
            parse_callback_target("/favicon.ico", "state")
                .unwrap()
                .is_none()
        );
        assert!(
            parse_callback_target("/callback?state=state&error=access_denied", "state").is_err()
        );
        assert!(
            parse_callback_target("/callback?state=state&state=state&code=code", "state").is_err()
        );
    }

    #[test]
    fn existing_refresh_token_is_loaded_without_exposing_other_fields() {
        let directory = tempfile::tempdir().unwrap();
        let token = directory.path().join("token.json");
        fs::write(
            &token,
            r#"{"access_token":"not-returned","refresh_token":"retained-refresh-token"}"#,
        )
        .unwrap();
        #[cfg(unix)]
        set_owner_only(&token).unwrap();
        assert_eq!(
            read_existing_refresh_token(&token).unwrap().as_deref(),
            Some("retained-refresh-token")
        );
    }

    #[cfg(unix)]
    #[test]
    fn oauth_inputs_reject_group_or_world_readable_files() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let token = directory.path().join("token.json");
        fs::write(&token, r#"{"refresh_token":"retained-refresh-token"}"#).unwrap();
        fs::set_permissions(&token, fs::Permissions::from_mode(0o644)).unwrap();
        let error = read_existing_refresh_token(&token)
            .expect_err("group-readable Google token must be rejected");
        assert!(error.to_string().contains("must not be accessible"));

        let client = directory.path().join("client.json");
        fs::write(
            &client,
            r#"{"installed":{"client_id":"client.apps.googleusercontent.com"}}"#,
        )
        .unwrap();
        fs::set_permissions(&client, fs::Permissions::from_mode(0o644)).unwrap();
        let error =
            read_client_file(&client).expect_err("group-readable OAuth client must be rejected");
        assert!(error.to_string().contains("must not be accessible"));
    }

    #[cfg(unix)]
    #[test]
    fn existing_refresh_token_rejects_dangling_symlink() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let token = directory.path().join("token.json");
        symlink(directory.path().join("missing-token.json"), &token).unwrap();

        let error = read_existing_refresh_token(&token)
            .expect_err("dangling token symlink must be rejected");
        assert!(error.to_string().contains("symlinked file"));
    }

    #[cfg(unix)]
    #[test]
    fn token_paths_reject_symlinked_parent_components() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::write(
            real.join("token.json"),
            r#"{"refresh_token":"retained-refresh-token"}"#,
        )
        .unwrap();
        let linked = directory.path().join("linked");
        symlink(&real, &linked).unwrap();

        let error = read_existing_refresh_token(&linked.join("token.json"))
            .expect_err("symlinked token parent must be rejected");
        assert!(error.to_string().contains("symlinked path component"));
    }

    #[test]
    fn shared_token_gets_union_of_configured_google_scopes() {
        let token = PathBuf::from("/tmp/cortana/token.json");
        let source = |name: &str, kind: &str| SourceConfig {
            name: name.into(),
            kind: kind.into(),
            enabled: false,
            project: "personal".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            token_env: None,
            token: Some(token.clone()),
            oauth_client: Some(PathBuf::from("/tmp/cortana/client.json")),
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        };
        let config = Config {
            sources: vec![
                source("drive", "google-drive"),
                source("mail", "gmail"),
                source("calendar", "google-calendar"),
            ],
            ..Config::default()
        };
        assert_eq!(
            scopes_for_token(&config, &token).unwrap(),
            [CALENDAR_SCOPE, DRIVE_SCOPE, GMAIL_SCOPE]
        );
    }

    #[test]
    fn token_path_environment_value_can_authorize_and_share_scopes() {
        let token = PathBuf::from("/tmp/cortana/env-token.json");
        let source = |name: &str, kind: &str| SourceConfig {
            name: name.into(),
            kind: kind.into(),
            enabled: false,
            project: "personal".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            token_env: Some("GOOGLE_TOKEN_PATH".into()),
            token: None,
            oauth_client: Some(PathBuf::from("/tmp/cortana/client.json")),
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        };
        let mut config = Config {
            sources: vec![source("drive", "google-drive"), source("mail", "gmail")],
            ..Config::default()
        };
        config
            .environment
            .insert("GOOGLE_TOKEN_PATH".into(), token.display().to_string());

        assert_eq!(
            configured_token_path(&config, &config.sources[0]).unwrap(),
            token
        );
        assert_eq!(
            scopes_for_token(&config, &token).unwrap(),
            [DRIVE_SCOPE, GMAIL_SCOPE]
        );
    }

    #[test]
    fn token_path_environment_value_is_required_when_no_explicit_file_exists() {
        let source = SourceConfig {
            name: "drive".into(),
            kind: "google-drive".into(),
            enabled: false,
            project: "personal".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            token_env: Some("MISSING_GOOGLE_TOKEN_PATH".into()),
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
        };
        let error = configured_token_path(&Config::default(), &source)
            .expect_err("missing token path environment value must fail closed");
        assert!(error.to_string().contains("MISSING_GOOGLE_TOKEN_PATH"));
    }

    #[test]
    fn credentials_cannot_be_stored_inside_an_indexed_filesystem_root() {
        let config = Config {
            sources: vec![SourceConfig {
                name: "documents".into(),
                kind: "filesystem".into(),
                enabled: false,
                project: "personal".into(),
                root: Some(PathBuf::from("/tmp/cortana/documents")),
                source: None,
                channels: Vec::new(),
                token_env: None,
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
            }],
            ..Config::default()
        };
        assert!(
            ensure_outside_filesystem_roots(
                &config,
                Path::new("/tmp/cortana/documents/token.json"),
                "token"
            )
            .is_err()
        );
    }
}

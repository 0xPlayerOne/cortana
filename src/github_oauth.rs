//! GitHub device-flow authorization and bounded repository discovery.
//!
//! GitHub OAuth apps do not provide a client secret that can safely be
//! shipped inside a desktop bundle. The operator supplies a client-id JSON
//! file, completes the short-lived device flow in the system browser, and
//! Cortana stores only the resulting access token in an owner-only file.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use reqwest::{Client, Url, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::config::{Config, SourceConfig};

const DEVICE_CODE_ENDPOINT: &str = "https://github.com/login/device/code";
const ACCESS_TOKEN_ENDPOINT: &str = "https://github.com/login/oauth/access_token";
const API_ENDPOINT: &str = "https://api.github.com";
const DEVICE_FLOW_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CLIENT_FILE_BYTES: u64 = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_REPOSITORIES: usize = 1_000;
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = MAX_REPOSITORIES / PAGE_SIZE;
const GITHUB_SCOPE: &str = "repo read:org";

#[derive(Debug, Serialize)]
pub struct AuthorizationOutcome {
    pub source: String,
    pub project: String,
    pub token_path: String,
    pub authorized: bool,
}

#[derive(Debug, Serialize)]
pub struct RepositoryList {
    pub repositories: Vec<RepositorySummary>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct RepositorySummary {
    pub id: u64,
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
struct ClientFile {
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredToken {
    access_token: String,
    token_type: String,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRepository {
    id: u64,
    full_name: String,
    private: bool,
    default_branch: String,
    html_url: String,
}

/// Run the GitHub OAuth device flow for one configured source.
pub async fn authorize(config: &Config, selected: &str) -> Result<AuthorizationOutcome> {
    validate_source_name(selected)?;
    let source = configured_github_source(config, selected)?;
    let token_path = required_token_destination(source)?;
    let client_path = required_secure_path(source.oauth_client.as_ref(), "GitHub OAuth client")?;
    ensure_outside_filesystem_roots(config, &token_path, "token")?;
    ensure_outside_filesystem_roots(config, client_path, "OAuth client")?;
    anyhow::ensure!(
        token_path != client_path,
        "GitHub token and OAuth client paths must be different"
    );

    let client_id = read_client_id(client_path)?;
    let client = github_client()?;
    let device = request_device_code(&client, &client_id).await?;
    let verification_uri = validate_browser_url(&device.verification_uri)?;
    let browser_url = device
        .verification_uri_complete
        .as_deref()
        .map(validate_browser_url)
        .transpose()?
        .unwrap_or_else(|| verification_uri.clone());

    // The complete URL is preferred because it avoids placing the short-lived
    // user code in the Desktop log. The CLI still prints the code when GitHub
    // does not provide a complete URL.
    open::that_detached(browser_url.as_str())
        .context("open GitHub authorization in the system browser")?;
    if device.verification_uri_complete.is_none() {
        println!(
            "GitHub authorization opened at {verification_uri}; enter device code {}",
            device.user_code
        );
    } else {
        println!("GitHub authorization opened in the system browser");
    }

    let token = poll_device_token(&client, &client_id, &device).await?;
    let access_token = token
        .access_token
        .context("GitHub OAuth response did not contain an access token")?;
    let token_type = token.token_type.unwrap_or_else(|| "bearer".to_string());
    anyhow::ensure!(
        token_type.eq_ignore_ascii_case("bearer"),
        "GitHub returned an unsupported token type"
    );
    validate_credential(&access_token, 16 * 1024, "GitHub access token")?;
    write_token(
        &token_path,
        &StoredToken {
            access_token,
            token_type: "bearer".into(),
            scope: token.scope,
        },
    )?;

    Ok(AuthorizationOutcome {
        source: source.name.clone(),
        project: source.project.clone(),
        token_path: token_path.display().to_string(),
        authorized: true,
    })
}

/// List repositories visible to a configured GitHub source without reading
/// repository content. The result is bounded to 1,000 entries for Desktop
/// selection and is safe to serialize into the renderer.
pub async fn list_repositories(config: &Config, selected: &str) -> Result<RepositoryList> {
    validate_source_name(selected)?;
    let source = configured_github_source(config, selected)?;
    let token = access_token(config, source)?;
    let client = github_client()?;
    let mut repositories = Vec::new();
    let mut page = 1usize;
    let mut truncated = false;

    loop {
        let url = format!(
            "{API_ENDPOINT}/user/repos?per_page={PAGE_SIZE}&page={page}&sort=updated&direction=desc&affiliation=owner,collaborator,organization_member"
        );
        let response: Vec<GithubRepository> = get_json(&client, &url, &token).await?;
        if response.is_empty() {
            break;
        }
        let remaining = MAX_REPOSITORIES.saturating_sub(repositories.len());
        repositories.extend(response.into_iter().take(remaining).map(|repository| {
            RepositorySummary {
                id: repository.id,
                full_name: repository.full_name,
                private: repository.private,
                default_branch: repository.default_branch,
                html_url: repository.html_url,
            }
        }));
        if repositories.len() >= MAX_REPOSITORIES {
            truncated = true;
            break;
        }
        if page >= MAX_PAGES || repositories.len() < page * PAGE_SIZE {
            break;
        }
        page += 1;
    }

    Ok(RepositoryList {
        repositories,
        truncated,
    })
}

async fn request_device_code(client: &Client, client_id: &str) -> Result<DeviceCodeResponse> {
    let response = client
        .post(DEVICE_CODE_ENDPOINT)
        .header("Accept", "application/json")
        .form(&[("client_id", client_id), ("scope", GITHUB_SCOPE)])
        .send()
        .await
        .context("request GitHub device code")?;
    let status = response.status();
    let payload: DeviceCodeResponse = bounded_json(response).await?;
    anyhow::ensure!(status.is_success(), "GitHub device-code request failed");
    anyhow::ensure!(
        !payload.device_code.is_empty() && !payload.user_code.is_empty() && payload.expires_in > 0,
        "GitHub returned an invalid device-code response"
    );
    Ok(payload)
}

async fn poll_device_token(
    client: &Client,
    client_id: &str,
    device: &DeviceCodeResponse,
) -> Result<AccessTokenResponse> {
    let deadline = Instant::now() + DEVICE_FLOW_TIMEOUT.min(Duration::from_secs(device.expires_in));
    let mut interval = Duration::from_secs(device.interval.unwrap_or(5).clamp(1, 30));
    loop {
        if Instant::now() >= deadline {
            bail!("GitHub device authorization timed out; start it again")
        }
        tokio::time::sleep(interval).await;
        let response = client
            .post(ACCESS_TOKEN_ENDPOINT)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", client_id),
                ("device_code", device.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .context("poll GitHub device authorization")?;
        let status = response.status();
        let token: AccessTokenResponse = bounded_json(response).await?;
        anyhow::ensure!(
            status.is_success(),
            "GitHub device authorization returned an invalid response"
        );
        match token.error.as_deref() {
            None => return Ok(token),
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = (interval + Duration::from_secs(5)).min(Duration::from_secs(30));
            }
            Some("expired_token") => bail!("GitHub device code expired; start authorization again"),
            Some("access_denied") => bail!("GitHub authorization was denied"),
            Some(error) => bail!("GitHub authorization failed: {error}"),
        }
    }
}

async fn get_json<T: DeserializeOwned>(client: &Client, url: &str, token: &str) -> Result<T> {
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .bearer_auth(token)
        .send()
        .await
        .context("request GitHub repository list")?;
    anyhow::ensure!(
        response.status().is_success(),
        "GitHub repository request failed"
    );
    bounded_json(response).await
}

async fn bounded_json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        bail!("GitHub response exceeded the safety limit")
    }
    let bytes = response.bytes().await.context("read GitHub response")?;
    anyhow::ensure!(
        bytes.len() <= MAX_RESPONSE_BYTES,
        "GitHub response exceeded the safety limit"
    );
    serde_json::from_slice(&bytes).context("GitHub returned invalid JSON")
}

fn github_client() -> Result<Client> {
    Client::builder()
        .redirect(Policy::none())
        .timeout(HTTP_TIMEOUT)
        .user_agent(format!("cortana/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build GitHub API client")
}

fn configured_github_source<'a>(config: &'a Config, selected: &str) -> Result<&'a SourceConfig> {
    let source = config
        .sources
        .iter()
        .find(|source| source.name == selected)
        .with_context(|| format!("configured source {selected} was not found"))?;
    anyhow::ensure!(
        source.kind == "github",
        "source {} is not a GitHub connector",
        source.name
    );
    Ok(source)
}

fn access_token(config: &Config, source: &SourceConfig) -> Result<String> {
    if let Some(path) = source.token.as_deref() {
        return read_token_file(path);
    }
    let name = source
        .token_env
        .as_deref()
        .context("GitHub source requires a token file or token environment variable")?;
    let token = config
        .environment_value(name)
        .with_context(|| format!("GitHub token environment variable {name} is not configured"))?;
    validate_credential(&token, 16 * 1024, "GitHub access token")?;
    Ok(token)
}

fn read_token_file(path: &Path) -> Result<String> {
    validate_secure_file(path, "GitHub token")?;
    let body = fs::read(path).with_context(|| format!("read GitHub token {}", path.display()))?;
    let token: StoredToken =
        serde_json::from_slice(&body).context("GitHub token file is invalid")?;
    anyhow::ensure!(
        token.token_type.eq_ignore_ascii_case("bearer"),
        "GitHub token file has an unsupported token type"
    );
    validate_credential(&token.access_token, 16 * 1024, "GitHub access token")?;
    Ok(token.access_token)
}

fn read_client_id(path: &Path) -> Result<String> {
    validate_secure_file(path, "GitHub OAuth client")?;
    let body =
        fs::read(path).with_context(|| format!("read GitHub OAuth client {}", path.display()))?;
    let client: ClientFile =
        serde_json::from_slice(&body).context("GitHub OAuth client must contain client_id")?;
    validate_credential(&client.client_id, 1024, "GitHub OAuth client id")?;
    Ok(client.client_id)
}

fn required_token_destination(source: &SourceConfig) -> Result<PathBuf> {
    let path = source
        .token
        .clone()
        .context("GitHub OAuth requires a token file destination")?;
    anyhow::ensure!(
        path.is_absolute(),
        "GitHub token destination must be absolute"
    );
    reject_symlink_components(&path)?;
    Ok(path)
}

fn required_secure_path<'a>(value: Option<&'a PathBuf>, label: &str) -> Result<&'a Path> {
    let path = value.context(format!("GitHub source requires a {label} path"))?;
    validate_secure_file(path, label)?;
    Ok(path)
}

fn validate_secure_file(path: &Path, label: &str) -> Result<()> {
    anyhow::ensure!(path.is_absolute(), "{label} path must be absolute");
    reject_symlink_components(path)?;
    let metadata = fs::symlink_metadata(path).with_context(|| format!("inspect {label}"))?;
    anyhow::ensure!(metadata.is_file(), "{label} must be a regular file");
    ensure_owner_only(&metadata, label)?;
    anyhow::ensure!(
        metadata.len() <= MAX_CLIENT_FILE_BYTES,
        "{label} exceeds 64 KiB"
    );
    Ok(())
}

fn write_token(path: &Path, token: &StoredToken) -> Result<()> {
    reject_symlink_components(path)?;
    let parent = path
        .parent()
        .context("GitHub token destination has no parent")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create GitHub token directory {}", parent.display()))?;
    let body = serde_json::to_vec_pretty(token)?;
    anyhow::ensure!(
        body.len() <= MAX_CLIENT_FILE_BYTES as usize,
        "GitHub token payload is too large"
    );
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .context("create GitHub token file")?;
    if let Err(error) = file.write_all(&body).and_then(|_| file.write_all(b"\n")) {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("write GitHub token file");
    }
    file.sync_all().context("flush GitHub token file")?;
    drop(file);
    fs::rename(&temporary, path)
        .with_context(|| format!("replace GitHub token {}", path.display()))?;
    Ok(())
}

fn validate_browser_url(value: &str) -> Result<String> {
    let url = Url::parse(value).context("GitHub returned an invalid verification URL")?;
    anyhow::ensure!(
        url.scheme() == "https",
        "GitHub verification URL must use HTTPS"
    );
    anyhow::ensure!(
        url.host_str() == Some("github.com"),
        "GitHub verification URL has an unexpected host"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "GitHub verification URL contains credentials"
    );
    Ok(url.to_string())
}

fn validate_credential(value: &str, maximum: usize, label: &str) -> Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "{label} is empty");
    anyhow::ensure!(value.len() <= maximum, "{label} exceeds safety limit");
    anyhow::ensure!(
        !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace()),
        "{label} contains whitespace or control characters"
    );
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut current = path.to_path_buf();
    loop {
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            anyhow::ensure!(
                !metadata.file_type().is_symlink() || is_allowed_system_alias(&current),
                "path component must not be a symlink"
            );
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
        path == Path::new("/tmp") || path == Path::new("/var") || path == Path::new("/etc")
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

fn ensure_owner_only(metadata: &fs::Metadata, label: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "{label} must be owner-only"
        );
    }
    Ok(())
}

fn ensure_outside_filesystem_roots(config: &Config, candidate: &Path, label: &str) -> Result<()> {
    let roots = config
        .sources
        .iter()
        .filter(|source| source.kind == "filesystem")
        .filter_map(|source| source.root.as_deref());
    anyhow::ensure!(
        !roots.clone().any(|root| candidate.starts_with(root)),
        "GitHub {label} path must be outside every filesystem source root"
    );
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn browser_urls_are_restricted_to_github_https() {
        assert!(validate_browser_url("https://github.com/login/device").is_ok());
        assert!(validate_browser_url("http://github.com/login/device").is_err());
        assert!(validate_browser_url("https://evil.example/login").is_err());
        assert!(validate_browser_url("https://user:pass@github.com/login/device").is_err());
    }

    #[test]
    fn client_and_token_files_require_private_regular_files() {
        let directory = TempDir::new().expect("temp directory");
        let client = directory.path().join("client.json");
        fs::write(&client, r#"{"client_id":"Iv1.abc"}"#).expect("client");
        let token = directory.path().join("token.json");
        fs::write(
            &token,
            r#"{"access_token":"gho_test","token_type":"bearer","scope":"repo"}"#,
        )
        .expect("token");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&client, fs::Permissions::from_mode(0o600)).expect("client mode");
            fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).expect("token mode");
        }
        assert_eq!(read_client_id(&client).expect("client id"), "Iv1.abc");
        assert_eq!(read_token_file(&token).expect("access token"), "gho_test");
    }

    #[test]
    fn token_destination_must_be_absolute_and_outside_source_roots() {
        let source = SourceConfig {
            name: "github".into(),
            kind: "github".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
            repositories: vec!["owner/repo".into()],
            token_env: None,
            token: Some(PathBuf::from("relative.json")),
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
        assert!(required_token_destination(&source).is_err());
    }
}

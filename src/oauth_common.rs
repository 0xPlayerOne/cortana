//! Shared primitives for browser OAuth authorization flows.
//!
//! Google and Discord authorization share the same security-critical
//! building blocks: a random loopback HTTP listener that accepts only the
//! exact state-verified callback, byte-bounded response reading, and
//! owner-only token file writes that reject symlinks and group-readable
//! inputs. Keeping them in one module means the hardened implementation is
//! reviewed and tested once instead of being duplicated per provider.
//!
//! Every function takes a human-readable `provider` or `label` argument so
//! errors keep naming the provider that failed without embedding any
//! credential value.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use reqwest::{Response, Url};
use serde::de::DeserializeOwned;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use uuid::Uuid;

pub const MAX_CLIENT_FILE_BYTES: u64 = 64 * 1024;
pub const MAX_TOKEN_RESPONSE_BYTES: usize = 64 * 1024;
pub const MAX_CALLBACK_BYTES: usize = 8 * 1024;
pub const MAX_CALLBACK_CONNECTIONS: usize = 20;
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// Return a new 64-character random secret for OAuth state or PKCE verifier.
pub fn random_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn validate_credential(label: &str, value: &str, maximum_bytes: usize) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= maximum_bytes
            && !value.contains(['\0', '\n', '\r'])
            && value.trim() == value,
        "{label} is invalid"
    );
    Ok(())
}

/// Wait for the OAuth loopback callback and return the authorization code.
/// Only the exact expected state is accepted; every other connection is
/// answered with a 400 and skipped.
pub async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    provider: &str,
) -> Result<String> {
    timeout(CALLBACK_TIMEOUT, async {
        for _ in 0..MAX_CALLBACK_CONNECTIONS {
            let (mut stream, peer) = listener
                .accept()
                .await
                .with_context(|| format!("accept {provider} OAuth callback"))?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let target = read_request_target(&mut stream, provider).await?;
            match parse_callback_target(&target, expected_state, provider) {
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
        bail!("too many invalid {provider} OAuth callbacks")
    })
    .await
    .map_err(|_| anyhow::anyhow!("{provider} authorization timed out after 5 minutes"))?
}

pub async fn read_request_target(stream: &mut TcpStream, provider: &str) -> Result<String> {
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .with_context(|| format!("read {provider} OAuth callback"))?;
        anyhow::ensure!(read > 0, "{provider} OAuth callback closed early");
        request.extend_from_slice(&buffer[..read]);
        anyhow::ensure!(
            request.len() <= MAX_CALLBACK_BYTES,
            "{provider} OAuth callback exceeded 8 KiB"
        );
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&request).context("OAuth callback was not UTF-8")?;
    let line = request.lines().next().context("OAuth callback was empty")?;
    let mut parts = line.split_ascii_whitespace();
    anyhow::ensure!(
        parts.next() == Some("GET"),
        "{provider} OAuth callback must use GET"
    );
    let target = parts.next().context("OAuth callback target is missing")?;
    anyhow::ensure!(
        parts
            .next()
            .is_some_and(|version| version.starts_with("HTTP/1.")),
        "OAuth callback has an invalid HTTP version"
    );
    anyhow::ensure!(target.len() <= 4096, "OAuth callback target is too long");
    Ok(target.to_string())
}

/// Parse the loopback callback target. Returns `Ok(None)` for any request
/// that is not the provider callback (e.g. favicon probes) so the listener
/// can keep waiting, and an error for a matching callback with a rejected
/// state, repeated parameter, or missing code.
pub fn parse_callback_target(
    target: &str,
    expected_state: &str,
    provider: &str,
) -> Result<Option<String>> {
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .with_context(|| format!("{provider} OAuth callback URL is invalid"))?;
    if url.path() != "/callback" {
        return Ok(None);
    }
    let mut state = None;
    let mut code = None;
    let mut error = None;
    for (name, value) in url.query_pairs() {
        match name.as_ref() {
            "state" => {
                anyhow::ensure!(state.is_none(), "{provider} OAuth callback repeated state");
                state = Some(value.into_owned());
            }
            "code" => {
                anyhow::ensure!(code.is_none(), "{provider} OAuth callback repeated code");
                code = Some(value.into_owned());
            }
            "error" => {
                anyhow::ensure!(error.is_none(), "{provider} OAuth callback repeated error");
                error = Some(value.into_owned());
            }
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Ok(None);
    }
    if let Some(error) = error {
        validate_credential(&format!("{provider} OAuth error"), &error, 256)?;
        bail!("{provider} authorization was not granted ({error})");
    }
    let code = code.context("OAuth callback did not contain a code")?;
    validate_credential(&format!("{provider} authorization code"), &code, 4096)?;
    Ok(Some(code))
}

pub async fn respond(stream: &mut TcpStream, status: &str, message: &str) -> Result<()> {
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
        .context("respond to OAuth callback")
}

/// Read a JSON body with a hard byte ceiling, checking the declared content
/// length first and then the streamed size so a lying server cannot bypass
/// the bound.
pub async fn bounded_json<T: DeserializeOwned>(
    response: Response,
    max_bytes: usize,
    provider: &str,
) -> Result<T> {
    anyhow::ensure!(max_bytes > 0, "JSON response safety limit must be positive");
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        bail!("{provider} response exceeded {max_bytes} bytes");
    }
    let mut body = Vec::new();
    let mut response = response;
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            body.len().saturating_add(chunk.len()) <= max_bytes,
            "{provider} response exceeded {max_bytes} bytes"
        );
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).context("JSON response body was invalid")
}

/// Atomically write an owner-only credential file: reject symlinks, create
/// the parent directory privately, back up an existing file, write through a
/// unique temporary file, fsync, rename, and force mode `0600` on Unix.
/// `label` names the credential in error messages (for example "Google
/// token") and `temp_prefix` names the temporary file so stale files from
/// crashed runs remain identifiable.
pub fn write_owner_only_file(
    path: &Path,
    body: &[u8],
    label: &str,
    temp_prefix: &str,
) -> Result<()> {
    reject_symlink(path)?;
    reject_symlink_components(path)?;
    let parent = path
        .parent()
        .with_context(|| format!("{label} path has no parent"))?;
    let created_parent = !parent.exists();
    fs::create_dir_all(parent)
        .with_context(|| format!("create {label} directory {}", parent.display()))?;
    if created_parent {
        set_directory_owner_only(parent)?;
    }
    if path.exists() {
        let backup = path.with_extension("json.backup");
        reject_symlink(&backup)?;
        fs::copy(path, &backup).with_context(|| format!("back up {label} {}", path.display()))?;
        set_owner_only(&backup)?;
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{temp_prefix}-{}-{nonce}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("create {label} {}", temporary.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
            .with_context(|| format!("replace {label} {}", path.display()))?;
        set_owner_only(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn reject_symlink(path: &Path) -> Result<()> {
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

pub fn reject_symlink_components(path: &Path) -> Result<()> {
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

pub fn is_allowed_system_alias(path: &Path) -> bool {
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

#[cfg(unix)]
pub fn set_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn set_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
pub fn set_directory_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn set_directory_owner_only(_path: &Path) -> Result<()> {
    Ok(())
}

pub fn ensure_owner_only(metadata: &fs::Metadata, label: &str) -> Result<()> {
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
    fn callback_requires_exact_state_and_path_for_every_provider() {
        for provider in ["Google", "Discord"] {
            assert_eq!(
                parse_callback_target("/callback?state=state&code=code", "state", provider)
                    .unwrap(),
                Some("code".into())
            );
            assert!(
                parse_callback_target("/callback?state=wrong&code=code", "state", provider)
                    .unwrap()
                    .is_none()
            );
            assert!(
                parse_callback_target("/favicon.ico", "state", provider)
                    .unwrap()
                    .is_none()
            );
            assert!(
                parse_callback_target(
                    "/callback?state=state&error=access_denied",
                    "state",
                    provider
                )
                .is_err()
            );
            assert!(
                parse_callback_target(
                    "/callback?state=state&state=state&code=code",
                    "state",
                    provider
                )
                .is_err()
            );
        }
    }

    #[test]
    fn credentials_reject_empty_line_breaks_and_surrounding_whitespace() {
        assert!(validate_credential("token", "abc-123", 1024).is_ok());
        for invalid in ["", " a", "a ", "a\nb", "a\rb", "a\0b", &"x".repeat(1025)] {
            assert!(
                validate_credential("token", invalid, 1024).is_err(),
                "credential {invalid:?} must be rejected"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_writes_force_mode_0600_and_replace_atomically() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let token = directory.path().join("nested/token.json");

        write_owner_only_file(
            &token,
            b"{\"access_token\":\"one\"}",
            "token",
            "cortana-test-token",
        )
        .expect("write token");
        let metadata = fs::metadata(&token).expect("token metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        let parent = fs::metadata(token.parent().expect("token parent")).expect("parent metadata");
        assert_eq!(parent.permissions().mode() & 0o777, 0o700);
        assert_eq!(fs::read(&token).unwrap(), b"{\"access_token\":\"one\"}\n");

        write_owner_only_file(
            &token,
            b"{\"access_token\":\"two\"}",
            "token",
            "cortana-test-token",
        )
        .expect("rewrite token");
        assert_eq!(fs::read(&token).unwrap(), b"{\"access_token\":\"two\"}\n");
        let backup = token.with_extension("json.backup");
        assert_eq!(
            fs::read(&backup).unwrap(),
            b"{\"access_token\":\"one\"}\n",
            "the previous token must be backed up before replacement"
        );
        assert_eq!(
            fs::metadata(&backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            !fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains(".tmp")),
            "no temporary files may remain after a successful write"
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_writes_reject_symlinked_targets_and_parents() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real");
        fs::create_dir(&real).unwrap();
        let linked = directory.path().join("linked");
        symlink(&real, &linked).unwrap();

        let error = write_owner_only_file(
            &linked.join("token.json"),
            b"{}",
            "token",
            "cortana-test-token",
        )
        .expect_err("symlinked token parent must be rejected");
        assert!(error.to_string().contains("symlinked path component"));

        let target = real.join("token.json");
        fs::write(&target, "{}").unwrap();
        let alias = directory.path().join("alias.json");
        symlink(&target, &alias).unwrap();
        let error = write_owner_only_file(&alias, b"{}", "token", "cortana-test-token")
            .expect_err("symlinked token target must be rejected");
        assert!(error.to_string().contains("symlinked file"));
    }
}

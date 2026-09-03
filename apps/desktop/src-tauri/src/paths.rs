use std::path::Path;

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, FilePath};
use tokio::sync::oneshot;

pub async fn pick(app: AppHandle, kind: &str) -> Result<Option<String>, String> {
    let (sender, receiver) = oneshot::channel();
    let dialog = app.dialog().file();
    match kind {
        "directory" => dialog
            .set_title("Choose a Cortana source directory")
            .pick_folder(move |path| {
                let _ = sender.send(path);
            }),
        "source-file" => dialog
            .set_title("Choose a file to index")
            .pick_file(move |path| {
                let _ = sender.send(path);
            }),
        "oauth-client" => dialog
            .set_title("Choose a Desktop OAuth client")
            .add_filter("OAuth client", &["json"])
            .pick_file(move |path| {
                let _ = sender.send(path);
            }),
        "google-token" => dialog
            .set_title("Choose where Cortana should store the Google token")
            .set_file_name("cortana-google-token.json")
            .add_filter("Google token", &["json"])
            .save_file(move |path| {
                let _ = sender.send(path);
            }),
        "github-token" => dialog
            .set_title("Choose where Cortana should store the GitHub token")
            .set_file_name("cortana-github-token.json")
            .add_filter("GitHub token", &["json"])
            .save_file(move |path| {
                let _ = sender.send(path);
            }),
        "discord-token" => dialog
            .set_title("Choose where Cortana should store the Discord RPC token")
            .set_file_name("cortana-discord-rpc-token.json")
            .add_filter("Discord RPC token", &["json"])
            .save_file(move |path| {
                let _ = sender.send(path);
            }),
        "slack-token" => dialog
            .set_title("Choose where Cortana should store the Slack user token")
            .set_file_name("cortana-slack-token.json")
            .add_filter("Slack token", &["json"])
            .save_file(move |path| {
                let _ = sender.send(path);
            }),
        "settings-export" => dialog
            .set_title("Export redacted Cortana settings")
            .set_file_name("cortana-settings.json")
            .add_filter("Cortana settings", &["json"])
            .save_file(move |path| {
                let _ = sender.send(path);
            }),
        "settings-import" => dialog
            .set_title("Import redacted Cortana settings")
            .add_filter("Cortana settings", &["json"])
            .pick_file(move |path| {
                let _ = sender.send(path);
            }),
        "backup-export" => dialog
            .set_title("Export a verified Cortana database backup")
            .set_file_name("cortana-backup.sqlite3")
            .add_filter("Cortana database backup", &["sqlite3"])
            .save_file(move |path| {
                let _ = sender.send(path);
            }),
        "backup-import" => dialog
            .set_title("Restore a Cortana database backup")
            .add_filter("Cortana database backup", &["sqlite3"])
            .pick_file(move |path| {
                let _ = sender.send(path);
            }),
        "vault-export" => dialog
            .set_title("Choose an Obsidian vault directory")
            .pick_folder(move |path| {
                let _ = sender.send(path);
            }),
        _ => return Err("unsupported native path picker".into()),
    }
    let selected = receiver
        .await
        .map_err(|_| "native path picker closed unexpectedly".to_string())?;
    selected.map(file_path).transpose()
}

fn file_path(path: FilePath) -> Result<String, String> {
    let path = path
        .into_path()
        .map_err(|error| format!("selected path is invalid: {error}"))?;
    validate_path(&path)?;
    Ok(path.display().to_string())
}

fn validate_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.parent().is_none_or(|parent| parent.parent().is_none())
    {
        return Err("select an absolute path outside the filesystem root".into());
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    }) {
        return Err("selected path must not contain relative components".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_paths_reject_roots_and_relative_components() {
        assert!(validate_path(Path::new("/Users/example/Documents")).is_ok());
        assert!(validate_path(Path::new("/Users")).is_err());
        assert!(validate_path(Path::new("../Documents")).is_err());
    }
}

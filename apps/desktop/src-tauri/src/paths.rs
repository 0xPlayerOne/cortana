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
        "oauth-client" => dialog
            .set_title("Choose a Google Desktop OAuth client")
            .add_filter("Google OAuth client", &["json"])
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

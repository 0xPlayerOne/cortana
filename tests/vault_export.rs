use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use cortana::model::Document;
use cortana::store::Store;
use cortana::vault_export::{VaultExportOptions, export_vault};

fn insert_document(store: &Store, project: &str, source_id: &str, content: &str, acl: Vec<String>) {
    insert_document_with_metadata(
        store,
        project,
        source_id,
        content,
        acl,
        serde_json::json!({"access_token": "must-not-export"}),
    );
}

fn insert_document_with_metadata(
    store: &Store,
    project: &str,
    source_id: &str,
    content: &str,
    acl: Vec<String>,
    metadata: serde_json::Value,
) {
    store
        .upsert(
            &Document {
                source: "notes".into(),
                source_id: source_id.into(),
                title: format!("Note {source_id}"),
                content: content.into(),
                uri: Some(format!("https://example.test/{source_id}")),
                updated_at: Utc::now(),
                project: project.into(),
                acl,
                metadata,
            },
            &[(content.into(), vec![1.0; 16])],
        )
        .expect("insert document");
}

#[test]
fn vault_export_is_scoped_incremental_atomic_and_reversible() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
    insert_document_with_metadata(
        &store,
        "work",
        "one",
        "first body",
        vec!["work".into()],
        serde_json::json!({"folder": "Research", "access_token": "must-not-export"}),
    );
    insert_document(
        &store,
        "personal",
        "private",
        "private body",
        vec!["personal".into()],
    );
    let output = directory.path().join("Cortana Vault");
    let cancel = Arc::new(AtomicBool::new(false));
    let options = VaultExportOptions {
        output: output.clone(),
        workspaces: BTreeSet::from(["work".into()]),
        principal_acl: vec!["work".into()],
        dry_run: false,
        cancel: cancel.clone(),
        progress: None,
    };

    let first = export_vault(&store, &options).expect("first export");
    assert_eq!(first.documents, 1);
    assert_eq!(first.content_rewrites, 1);
    assert_eq!(first.deleted_documents, 0);
    let markdown = std::fs::read_to_string(&first.files[0]).expect("markdown");
    assert!(markdown.contains("cortana_document_id:"));
    assert!(markdown.contains("workspace: \"work\""));
    assert!(markdown.contains("source_id: \"one\""));
    assert!(markdown.contains("first body"));
    assert!(!markdown.contains("must-not-export"));
    assert!(!markdown.contains("private body"));
    assert!(first.files[0].ends_with("work/notes/Research/one.md"));
    assert!(output.join(".cortana-vault.json").is_file());

    let unchanged = export_vault(&store, &options).expect("unchanged export");
    assert_eq!(unchanged.documents, 1);
    assert_eq!(unchanged.content_rewrites, 0);
    assert_eq!(unchanged.unchanged_documents, 1);

    insert_document_with_metadata(
        &store,
        "work",
        "one",
        "updated first body",
        vec!["work".into()],
        serde_json::json!({"folder": "Research", "access_token": "must-not-export"}),
    );
    let updated = export_vault(&store, &options).expect("updated export");
    assert_eq!(updated.content_rewrites, 1);
    assert!(
        updated
            .previous_vault
            .as_ref()
            .is_some_and(|path| path.is_dir())
    );
    assert!(
        std::fs::read_to_string(&updated.files[0])
            .expect("updated markdown")
            .contains("updated first body")
    );

    let personal_options = VaultExportOptions {
        workspaces: BTreeSet::from(["personal".into()]),
        principal_acl: vec!["personal".into()],
        ..options.clone()
    };
    let replaced = export_vault(&store, &personal_options).expect("replace selected scope");
    assert_eq!(replaced.documents, 1);
    assert_eq!(replaced.deleted_documents, 1);
    assert!(!replaced.files[0].to_string_lossy().contains("work"));
    let previous = replaced.previous_vault.expect("previous complete vault");
    assert!(
        std::fs::read_to_string(previous.join("work/notes/Research/one.md"))
            .expect("reversible previous document")
            .contains("updated first body")
    );
}

#[test]
fn vault_export_dry_run_and_cancellation_do_not_publish_partial_output() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
    insert_document(&store, "work", "one", "first body", Vec::new());
    let output = directory.path().join("dry-run-parent/vault");
    let cancel = Arc::new(AtomicBool::new(false));
    let mut options = VaultExportOptions {
        output: output.clone(),
        workspaces: BTreeSet::from(["work".into()]),
        principal_acl: vec!["*".into()],
        dry_run: true,
        cancel: cancel.clone(),
        progress: None,
    };

    let dry_run = export_vault(&store, &options).expect("dry run");
    assert_eq!(dry_run.content_rewrites, 1);
    assert!(!output.exists());
    assert!(!output.parent().expect("output parent").exists());

    options.dry_run = false;
    cancel.store(true, Ordering::SeqCst);
    let error = export_vault(&store, &options).expect_err("cancelled export");
    assert!(error.to_string().contains("cancelled"));
    assert!(!output.exists());
}

#[test]
fn vault_export_reports_progress_and_mid_export_cancellation_keeps_previous_vault() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
    insert_document(&store, "work", "one", "first body", Vec::new());
    let output = directory.path().join("vault");
    let cancel = Arc::new(AtomicBool::new(false));
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut options = VaultExportOptions {
        output: output.clone(),
        workspaces: BTreeSet::from(["work".into()]),
        principal_acl: vec!["*".into()],
        dry_run: false,
        cancel: cancel.clone(),
        progress: Some({
            let events = events.clone();
            Arc::new(move |progress| events.lock().expect("events").push(progress))
        }),
    };
    export_vault(&store, &options).expect("initial export");
    assert_eq!(
        events
            .lock()
            .expect("events")
            .last()
            .expect("completion event")
            .phase,
        "complete"
    );
    let previous_markdown =
        std::fs::read_to_string(output.join("work/notes/one.md")).expect("previous complete vault");

    insert_document(&store, "work", "one", "changed body", Vec::new());
    options.progress = Some({
        let cancel = cancel.clone();
        Arc::new(move |progress| {
            if progress.phase == "writing" {
                cancel.store(true, Ordering::SeqCst);
            }
        })
    });
    let error = export_vault(&store, &options).expect_err("cancel during staged write");
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(
        std::fs::read_to_string(output.join("work/notes/one.md")).expect("usable old vault"),
        previous_markdown
    );
    assert!(
        std::fs::read_dir(directory.path())
            .expect("temporary root")
            .flatten()
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .contains("cortana-staging"))
    );
}

#[test]
fn vault_export_refuses_unmanaged_or_symlink_destinations_and_keeps_manifest_private() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
    insert_document(&store, "work", "one", "first body", Vec::new());
    let unmanaged = directory.path().join("unmanaged");
    std::fs::create_dir(&unmanaged).expect("unmanaged directory");
    std::fs::write(unmanaged.join("keep.txt"), "keep").expect("unmanaged file");
    let options = VaultExportOptions {
        output: unmanaged.clone(),
        workspaces: BTreeSet::from(["work".into()]),
        principal_acl: vec!["*".into()],
        dry_run: false,
        cancel: Arc::new(AtomicBool::new(false)),
        progress: None,
    };
    let error = export_vault(&store, &options).expect_err("unmanaged destination");
    assert!(error.to_string().contains("unmanaged directory"));
    assert_eq!(
        std::fs::read_to_string(unmanaged.join("keep.txt")).expect("preserved unmanaged file"),
        "keep"
    );

    let output = directory.path().join("managed");
    let managed_options = VaultExportOptions {
        output: output.clone(),
        ..options.clone()
    };
    export_vault(&store, &managed_options).expect("managed export");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let mode = std::fs::metadata(output.join(".cortana-vault.json"))
            .expect("manifest metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let linked = directory.path().join("linked");
        symlink(&output, &linked).expect("vault symlink");
        let linked_options = VaultExportOptions {
            output: linked,
            ..managed_options
        };
        let error = export_vault(&store, &linked_options).expect_err("symlink destination");
        assert!(error.to_string().contains("symbolic link"));
    }
}

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::tempdir;

fn executable(path: &Path, body: &str) {
    fs::write(path, body).expect("write executable");
    let mut permissions = fs::metadata(path)
        .expect("executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("set executable permissions");
}

fn release_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let directory = tempdir().expect("temporary directory");
    let root = directory.path();
    let archive = root.join("archive");
    let fake_bin = root.join("fake-bin");
    let log = root.join("cortana.log");
    fs::create_dir_all(archive.join("bin")).expect("archive bin");
    fs::create_dir_all(archive.join("dist")).expect("archive dist");
    fs::create_dir_all(archive.join("share/cortana/web")).expect("archive web");
    fs::create_dir_all(&fake_bin).expect("fake bin");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/install-release.sh"),
        archive.join("install.sh"),
    )
    .expect("copy installer");
    fs::write(archive.join("share/cortana/web/index.html"), "cortana").expect("web fixture");
    fs::write(
        archive.join("dist/cortana_brain-0.0.0-py3-none-any.whl"),
        "",
    )
    .expect("wheel fixture");
    executable(
        &archive.join("bin/cortana"),
        r#"#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> "$CORTANA_TEST_LOG"
if [[ "$*" == *" init "* ]]; then
  while [[ "$#" -gt 0 ]]; do
    if [[ "$1" == "--config" ]]; then
      mkdir -p "$(dirname "$2")"
      : > "$2"
      break
    fi
    shift
  done
fi
"#,
    );
    executable(
        &fake_bin.join("uv"),
        r#"#!/usr/bin/env bash
set -eu
if [[ "${1:-}" == "venv" ]]; then
  for argument in "$@"; do destination="$argument"; done
  mkdir -p "$destination/bin"
fi
"#,
    );
    executable(
        &fake_bin.join("uname"),
        "#!/usr/bin/env bash\nprintf 'Darwin\\n'\n",
    );
    (directory, archive, log)
}

fn run_installer(enable_sync: bool) -> String {
    let (directory, archive, log) = release_fixture();
    let root = directory.path();
    let path = format!(
        "{}:{}",
        root.join("fake-bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let status = Command::new("/bin/bash")
        .arg("-u")
        .arg(archive.join("install.sh"))
        .env("PATH", path)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("CORTANA_INSTALL_PREFIX", root.join("prefix"))
        .env("CORTANA_CONFIG", root.join("config/cortana/config.toml"))
        .env("CORTANA_TEST_LOG", &log)
        .env(
            "CORTANA_ENABLE_SYNC_SERVICE",
            if enable_sync { "1" } else { "0" },
        )
        .status()
        .expect("run release installer");
    assert!(status.success(), "release installer should succeed");
    fs::read_to_string(log).expect("read Cortana invocation log")
}

#[test]
fn query_only_release_install_works_with_nounset_and_omits_sync_service() {
    let log = run_installer(false);
    let service = log
        .lines()
        .find(|line| line.contains(" service install "))
        .expect("service install invocation");
    assert!(!service.contains("--enable-sync-service"));
}

#[test]
fn release_install_requires_explicit_sync_opt_in() {
    let log = run_installer(true);
    let service = log
        .lines()
        .find(|line| line.contains(" service install "))
        .expect("service install invocation");
    assert!(service.contains("--enable-sync-service"));
}

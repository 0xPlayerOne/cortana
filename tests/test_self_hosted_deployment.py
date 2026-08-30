from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEPLOY = ROOT / "deploy" / "self-hosted"


def test_container_profile_is_non_root_and_uses_durable_runtime_paths() -> None:
    dockerfile = (ROOT / "Dockerfile").read_text()
    compose = (DEPLOY / "compose.yaml").read_text()

    assert "USER 10001:10001" in dockerfile
    assert "--allow-remote" in dockerfile
    assert "read_only: true" in compose
    assert "cap_drop:" in compose and "- ALL" in compose
    for mount in ("cortana_data", "cortana_backups", "cortana_models"):
        assert mount in compose
    assert "/etc/cortana/config.toml:ro" in compose
    assert "restart: unless-stopped" in compose
    assert "stop_grace_period:" in compose


def test_default_compose_exposure_is_loopback_only_and_bearer_scoped() -> None:
    compose = (DEPLOY / "compose.yaml").read_text()
    config = (DEPLOY / "config.toml").read_text()

    assert "127.0.0.1:7331:7331" in compose
    assert "0.0.0.0:7331:7331" not in compose
    assert "CORTANA_OWNER_TOKEN: ${CORTANA_OWNER_TOKEN:?" in compose
    assert 'token_env = "CORTANA_OWNER_TOKEN"' in config
    assert 'scopes = ["query", "status", "memory", "admin"]' in config
    assert "acl = []" in config


def test_tls_overlay_is_explicit_and_reverse_proxies_only_cortana() -> None:
    overlay = (DEPLOY / "compose.tls.yaml").read_text()
    caddy = (DEPLOY / "Caddyfile").read_text()

    assert "CORTANA_DOMAIN:?" in overlay
    assert "80:80" in overlay and "443:443" in overlay
    assert "reverse_proxy cortana:7331" in caddy
    assert "127.0.0.1" not in caddy


def test_container_release_workflow_is_tag_scoped_and_publishes_ghcr() -> None:
    workflow = (ROOT / ".github" / "workflows" / "container.yml").read_text()

    assert "tags:" in workflow and "- 'v*'" in workflow
    assert "packages: write" in workflow
    assert "ghcr.io/${{ github.repository }}" in workflow
    assert "push: ${{ startsWith(github.ref, 'refs/tags/v') }}" in workflow
    assert (
        "provenance: ${{ startsWith(github.ref, 'refs/tags/v') && 'mode=max' || false }}"
        in workflow
    )
    assert "sbom: ${{ startsWith(github.ref, 'refs/tags/v') }}" in workflow
    assert "docker/build-push-action@10e90e3645eae34f1e60eeb005ba3a3d33f178e8" in workflow
    assert "scripts/self-hosted-conformance.sh" in workflow


def test_self_hosted_conformance_drill_is_bounded_and_cleans_up() -> None:
    drill = (ROOT / "scripts" / "self-hosted-conformance.sh").read_text()

    assert "127.0.0.1::7331" in drill
    assert "--read-only" in drill
    assert "--cap-drop ALL" in drill
    assert "--security-opt no-new-privileges" in drill
    assert "synthetic-provider-conformance-token" in drill
    assert "docker restart --timeout 20" in drill
    assert "backup --keep 3" in drill
    assert "cortana.provider.v1" in drill
    assert "trap cleanup EXIT INT TERM" in drill
    assert 'docker volume rm "$data_volume" "$backup_volume"' in drill

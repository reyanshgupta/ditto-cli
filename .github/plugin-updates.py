#!/usr/bin/env python3
"""Check real Claude/Codex plugin updates using local fixtures and temporary homes."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main():
    if os.name == "nt":
        raise SystemExit("This fixture requires Unix HOME discovery.")
    binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/ditto-cli").resolve())
    for tool in [binary, "claude", "codex"]:
        if not shutil.which(tool):
            raise SystemExit(f"Required executable is missing: {tool}")
    with tempfile.TemporaryDirectory(prefix="ditto-plugin-updates-") as temporary:
        root = Path(temporary).resolve()
        home = root / "home"
        home.mkdir()
        market = root / "market"
        env = {
            "PATH": os.environ["PATH"],
            "HOME": str(home),
            "USERPROFILE": str(home),
            "DITTO_HOME": str(root / "store"),
            "DITTO_NO_PROXY": "1",
            "CLAUDE_CONFIG_DIR": str(home / "custom-claude"),
            "CODEX_HOME": str(home / ".codex"),
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
            "DISABLE_AUTOUPDATER": "1",
        }

        def write(path, value):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(value if isinstance(value, str) else json.dumps(value))

        def run(args):
            result = subprocess.run(args, env=env, cwd=root, capture_output=True,
                                    text=True, timeout=30)
            assert result.returncode == 0, (args, result.stdout, result.stderr)
            return result.stdout

        def publish_fixture(version):
            write(market / "fixture/.claude-plugin/plugin.json", {
                "name": "fixture", "version": version,
                "description": "Test plugin update propagation",
            })
            write(market / "fixture/skills/fixture/SKILL.md",
                  "---\nname: fixture\ndescription: Plugin update fixture.\n---\n"
                  f"Version {version}\n")

        plugin_id = "fixture@ditto-update-fixture"
        write(market / ".claude-plugin/marketplace.json", {
            "name": "ditto-update-fixture", "owner": {"name": "Ditto Test"},
            "plugins": [{"name": "fixture", "source": "./fixture"}],
        })
        write(home / ".codex/config.toml", "[features]\nremote_plugin = false\n")
        publish_fixture("1.0.0")
        for tool in ["claude", "codex"]:
            run([tool, "plugin", "marketplace", "add", str(market)])
            run([tool, "plugin", "install" if tool == "claude" else "add", plugin_id])
        profiles = {name: json.loads(run([binary, "create", name, "--json"]))
                    for name in ["work", "personal"]}

        # Claude shares its cache in both directions. Codex keeps executable
        # code local, so profile-installed updates must remain in that profile.
        for version, updater in [("2.0.0", "default"), ("3.0.0", "work")]:
            publish_fixture(version)
            for tool in ["claude", "codex"]:
                prefix = [tool] if updater == "default" else [binary, tool, updater, "--"]
                run(prefix + ["plugin", "update" if tool == "claude" else "add", plugin_id])
                for profile in ["default", "work", "personal"]:
                    prefix = [tool] if profile == "default" else [binary, tool, profile, "--"]
                    listed = json.loads(run(prefix + ["plugin", "list", "--json"]))
                    entries = listed if tool == "claude" else listed["installed"]
                    entry = next(item for item in entries
                                 if item.get("id", item.get("pluginId")) == plugin_id)
                    expected = ("2.0.0" if tool == "codex" and updater == "work"
                                and profile != "work" else version)
                    assert entry["version"] == expected, (tool, profile, entry)
                    native = home / ("custom-claude" if tool == "claude" else ".codex")
                    config = native if profile == "default" else Path(profiles[profile][tool])
                    skill = config / f"plugins/cache/ditto-update-fixture/fixture/{expected}/skills/fixture/SKILL.md"
                    assert skill.read_text().endswith(f"Version {expected}\n"), (tool, profile)
                    if tool == "codex":
                        assert skill.resolve().is_relative_to(config.resolve()), (tool, profile)
                        if expected != version:
                            assert not (config / f"plugins/cache/ditto-update-fixture/fixture/{version}").exists()
                if tool == "codex" and updater == "work":
                    print("codex: 3.0.0 stayed local to work; default and personal retained 2.0.0.")
                else:
                    print(f"{tool}: {version} reached default, work and personal without Ditto sync.")


if __name__ == "__main__":
    main()

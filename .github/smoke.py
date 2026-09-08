#!/usr/bin/env python3
"""Exercise the compiled CLI on Unix with stand-in agents and a temporary home."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


ROOTS = {
    "claude": ("CLAUDE_CONFIG_DIR", ""),
    "codex": ("CODEX_HOME", ""),
    "fx": ("HOME", ".fx"),
    "opencode": ("XDG_DATA_HOME", "opencode"),
    "omp": ("HOME", ".omp/profiles/work/agent"),
    "prime-agent": ("PRIME_AGENT_CODING_AGENT_DIR", ""),
    "pi": ("PI_CODING_AGENT_DIR", ""),
    "gemini": ("GEMINI_CLI_HOME", ".gemini"),
    "qwen": ("QWEN_HOME", ""),
    "openclaude": ("OPENCLAUDE_CONFIG_DIR", ""),
    "copilot": ("COPILOT_HOME", ""),
    "cursor-agent": ("CURSOR_CONFIG_DIR", ""),
    "grok": ("GROK_HOME", ""),
    "devin": ("XDG_CONFIG_HOME", "devin"),
    "kimi": ("KIMI_CODE_HOME", ""),
    "cline": ("CLINE_DIR", ""),
    "codebuff": ("HOME", ".config/manicode"),
    "cn": ("CONTINUE_GLOBAL_DIR", ""),
    "command-code": ("HOME", ".commandcode"),
    "hermes": ("HERMES_HOME", ""),
    "openclaw": ("OPENCLAW_STATE_DIR", ""),
    "vibe": ("VIBE_HOME", ""),
    "acli": ("HOME", ".rovodev"),
    "amp": ("XDG_CONFIG_HOME", "amp"),
    "droid": ("HOME", ".factory"),
    "goose": ("XDG_CONFIG_HOME", "goose"),
    "aider": ("HOME", ".aider"),
    "crush": ("XDG_CONFIG_HOME", "crush"),
    "kilo": ("XDG_CONFIG_HOME", "kilo"),
    "kiro-cli": ("HOME", ".kiro"),
    "auggie": ("HOME", ".augment"),
    "agy": ("HOME", ".gemini/antigravity-cli"),
    "mimo": ("XDG_CONFIG_HOME", "mimocode"),
    "ante": ("ANTE_HOME", ""),
    "traecli": ("HOME", ".trae"),
    "autohand": ("AUTOHAND_HOME", ""),
}


def write(path, contents):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents)


def main():
    if os.name == "nt":
        raise SystemExit("This smoke fixture requires Unix HOME discovery; use cargo test on Windows.")
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/ditto-cli").resolve()
    with tempfile.TemporaryDirectory(prefix="ditto-smoke-") as directory:
        root = Path(directory).resolve()
        home = root / "home"
        home.mkdir()
        # Neither provider keys nor a parent Ditto's redirected roots belong in
        # this fixture. OMP stores profiles under HOME even with DITTO_HOME set.
        env = {key: value for key, value in os.environ.items() if key in {
            "PATH", "SystemRoot", "WINDIR", "PATHEXT", "COMSPEC", "TMP", "TEMP", "TMPDIR"
        }}
        env.update(HOME=str(home), USERPROFILE=str(home), DITTO_HOME=str(root / "store"),
                   DITTO_NO_PROXY="1", CLAUDE_CONFIG_DIR=str(home / "custom-claude"))
        stub = root / "agent.py"
        write(stub, """import json, os, subprocess, sys
if len(sys.argv) > 1 and sys.argv[1] == 'nested':
    result = subprocess.run(sys.argv[2:])
    sys.exit(result.returncode)
print(json.dumps({'args': sys.argv[1:], 'env': dict(os.environ)}))
sys.exit(23 if '--fail' in sys.argv else 0)
""")
        wrapper = root / "agent"
        write(wrapper, f'#!/bin/sh\nexec "{sys.executable}" "{stub}" "$@"\n')
        wrapper.chmod(0o700)
        for tool in ROOTS:
            env[f"DITTO_{tool.upper().replace('-', '_')}_BIN"] = str(wrapper)

        def run(*args, code=0):
            result = subprocess.run([str(binary), *args], env=env, cwd=root,
                                    capture_output=True, text=True, timeout=30)
            assert result.returncode == code, (args, result.returncode, result.stderr)
            try:
                return json.loads(result.stdout) if result.stdout.strip() else None
            except json.JSONDecodeError as error:
                raise AssertionError((args, result.stdout, result.stderr)) from error

        native = run("paths", "default", "--json")
        assert Path(native["claude"]) == home / "custom-claude"
        for tool in ("claude", "codex"):
            write(Path(native[tool]) / "skills/fixture/SKILL.md", "fixture skill")
        write(Path(native["claude"]) / "settings.json", json.dumps({
            "enabledPlugins": {"first@local": True}, "model": "original"
        }))
        write(Path(native["codex"]) / "agents/reviewer.toml", 'name = "reviewer"\n')
        write(Path(native["codex"]) / "auth.json", '{"fixture":"private"}')
        desktop_project = {"id": "pi", "name": "SharePi", "rootPaths": [str(root / "pi")],
                           "createdAt": 1, "updatedAt": 2}
        native_desktop = Path(native["codex"]) / ".codex-global-state.json"
        write(native_desktop, json.dumps({"local-projects": {"pi": desktop_project},
                                         "electron-persisted-atom-state": {"account": "private"}}))
        write(Path(native["codex"]) / "plugins/cache/fixture/1/service.mjs", "plugin code")
        created = run("create", "work", "--json")
        assert created["shared_failed"] == [], created["shared_failed"]
        assert created["desktop_projects_copied"] == ["SharePi"]
        assert created["shared_copied"] == ["codex/plugins/cache"]
        code = Path(created["codex"]) / "plugins/cache/fixture/1/service.mjs"
        assert code.read_text() == "plugin code"
        assert code.resolve().is_relative_to(Path(created["codex"]).resolve())
        profile_desktop = Path(created["codex"]) / ".codex-global-state.json"
        assert json.loads(profile_desktop.read_text()) == {
            "local-projects": {"pi": desktop_project}, "project-order": ["pi"]}
        run("create", "other", "--json")
        expected_tools = {key.replace("_", "-") for key in created
                          if key in {tool.replace("-", "_") for tool in ROOTS}}
        assert expected_tools == set(ROOTS)
        status_tools = {entry["tool"] for entry in run("status", "work", "--json")["tools"]}
        assert status_tools == set(ROOTS), "update the smoke matrix for newly supported agents"
        assert not (Path(created["codex"]) / "auth.json").exists()
        assert (Path(created["codex"]) / "agents/reviewer.toml").read_text() == 'name = "reviewer"\n'

        for tool, (variable, suffix) in ROOTS.items():
            launched = run(tool, "work", "--", "argument with spaces", "--literal=value")
            observed = Path(launched["env"][variable]) / suffix
            assert observed == Path(created[tool.replace("-", "_")]), (tool, observed)
            assert launched["env"]["DITTO_PROFILE"] == "work", tool
            assert launched["env"]["DITTO_LAUNCHED_TOOL"] == tool, tool
            prefix = ["--profile", "work"] if tool == "omp" else []
            assert launched["args"] == prefix + ["argument with spaces", "--literal=value"], tool
            run(tool, "work", "--", "--fail", code=23)

        # A first install differs from adding a skill to an already linked
        # directory; launches must discover the newly created directory too.
        write(Path(native["claude"]) / "commands/later.md", "later command")
        run("claude", "work", "--", "probe")
        assert (Path(created["claude"]) / "commands/later.md").read_text() == "later command"
        write(Path(native["claude"]) / "settings.json", json.dumps({
            "enabledPlugins": {"first@local": True, "later@local": True}, "model": "changed"
        }))
        core_project = dict(desktop_project, id="core", name="ShareOS", rootPaths=[str(root / "core")])
        write(native_desktop, json.dumps({"local-projects": {"pi": desktop_project, "core": core_project}}))
        # Launching a CLI beside a desktop app must not rewrite the state that
        # the desktop process has already loaded into memory.
        before = profile_desktop.read_bytes()
        run("codex", "work", "--", "probe")
        assert profile_desktop.read_bytes() == before
        synced = run("sync", "work", "--json")
        assert synced["shared_failed"] == [], synced["shared_failed"]
        assert synced["desktop_projects_copied"] == ["ShareOS"]
        assert json.loads(profile_desktop.read_text())["local-projects"]["core"] == core_project
        settings = json.loads((Path(created["claude"]) / "settings.json").read_text())
        assert settings["enabledPlugins"]["later@local"] is True
        assert settings["model"] == "original"
        assert not run("sync", "work", "--json")["changed"]

        nested = run("fx", "work", "--", "nested", str(binary), "paths", "default", "--json")
        assert nested == native
        for tool in ("fx", "pi", "prime-agent", "goose"):
            nested = run(tool, "work", "--", "nested", str(binary), tool, "default", "--", "probe")
            variable, suffix = ROOTS[tool]
            assert Path(nested["env"][variable]) / suffix == Path(native[tool.replace("-", "_")]), tool
            for session_variable in ("PI_CODING_AGENT_SESSION_DIR", "PRIME_AGENT_SESSION_DIR"):
                assert session_variable not in nested["env"], (tool, session_variable)
        # A failed trusted-cache migration must stop Codex, while a launch of
        # another tool must leave the unrelated Codex cache untouched.
        external = root / "external-plugin"
        write(external, "outside the trusted cache")
        invalid = Path(native["codex"]) / "plugins/cache/external"
        invalid.symlink_to(external)
        run("claude", "work", "--", "probe")
        run("codex", "work", "--", "probe", code=1)
        assert not (Path(created["codex"]) / "plugins/cache/external").exists()
        invalid.unlink()
        run("workspace", "clear", "--json")
        run("default", "work", "--json")
        run("rename", "work", "client", "--json")
        assert run("list", "--json")["default_profile"] == "client"
        renamed = run("paths", "client", "--json")
        assert (Path(renamed["claude"]) / "skills/fixture/SKILL.md").read_text() == "fixture skill"
        run("delete", "client", "--yes", "--json")
        assert (Path(native["claude"]) / "skills/fixture/SKILL.md").read_text() == "fixture skill"
        assert json.loads((Path(native["codex"]) / "auth.json").read_text()) == {"fixture": "private"}
        assert run("list", "--json")["default_profile"] is None
        print(f"CLI smoke passed: {len(ROOTS)} agents, argument and exit forwarding, sharing, sync, nested discovery, rename, delete.")


if __name__ == "__main__":
    main()

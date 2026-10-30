# ahok

A fast, stateless control plane for coding-agent harnesses. Harnesses run
`ahok hook` from their hooks. It enforces the rules in `~/.agents/rules/*.toml`,
formats edited files with [`fml`](https://github.com/arvinduh/formality), and
runs a lint gate when the agent stops.

## Install

Linux and macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/arvinduh/ahok/releases/latest/download/ahok-installer.sh | sh
```

Windows (PowerShell):

```powershell
powershell -c "irm https://github.com/arvinduh/ahok/releases/latest/download/ahok-installer.ps1 | iex"
```

Cargo:

```sh
cargo install --git https://github.com/arvinduh/ahok --locked
```

Update in place anytime with `ahok update`.

## Claude Code hooks

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|Edit|Write|MultiEdit|Skill",
        "hooks": [
          {
            "type": "command",
            "command": "ahok hook claude PreToolUse",
            "timeout": 5
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Edit|Write|MultiEdit",
        "hooks": [
          {
            "type": "command",
            "command": "ahok hook claude PostToolUse",
            "timeout": 60
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "ahok hook claude Stop",
            "timeout": 300
          }
        ]
      }
    ]
  }
}
```

The hook always exits 0 and writes JSON. Any internal failure is logged to
`errors.log` and the call is allowed.

## Codex hooks

Codex reads the same file shape from `~/.codex/hooks.json`. Use the block above
with `ahok hook codex <event>` and the matcher `Bash|apply_patch` on
`PreToolUse` and `apply_patch` on `PostToolUse`. Codex asks you to trust a new
hook the first time it runs.

Two rule kinds behave differently, because Codex can't do what they need:

- An `ask` rule denies, telling the agent to get your go-ahead in chat. Codex
  rejects `ask` replies and would run the call.
- A `require_skill` rule adds a note naming the skill instead of blocking. Codex
  has no Skill tool, so ahok can't see a skill load.

## Commands

| Command                        | Does                                                  |
| ------------------------------ | ----------------------------------------------------- |
| `ahok hook <harness> <event>`  | Answers one hook call (`claude`, `codex`, `agy`).     |
| `ahok ci wait [pr]`            | Waits for CI silently, then prints one summary.       |
| `ahok doctor`                  | Checks the rules, `fml`/`git`/`mise`, and hook setup. |
| `ahok link [repo] [--dry-run]` | Symlinks a repo's configs from its `links.toml`.      |
| `ahok update [--check]`        | Updates `ahok` in place to the latest release.        |
| `ahok stats`                   | Shows p50/p99 hook latency and rule hit counts.       |

`ahok ci wait` needs a `gh` login. It reads checks through the REST API, polls
with backoff up to once a minute, and prints each failed job's last log lines.
It exits 0 when all checks pass, 1 on a failure, and 75 when `--timeout` runs
out (540 s by default outside Claude Code, where it waits as long as it takes).

`ahok link` reads `[[link]]` entries with a `source` path in the repo and a
target in `home`, `linux` or `windows` (`~` and `%VAR%` expand). A trailing `/*`
on `source` links each child. `cloud = true` also links the entry in Claude Code
cloud sessions, which ahok detects from `CLAUDE_CODE_REMOTE`. It never
overwrites a real file: it moves it to `<name>.bak-<time>` first.

Config comes from `$AHOK_CONFIG_DIR` (default `~/.agents`). State lives in
`%LOCALAPPDATA%\ahok` on Windows and `~/.local/state/ahok` elsewhere:
`decisions.jsonl`, `errors.log`, and `sessions/`.

# ahok

A fast, stateless control plane for coding-agent harnesses. Harnesses run
`ahok hook` from their hooks. It enforces the rules in `~/.agents/rules/*.toml`,
formats edited files with [`fml`](https://github.com/arvinduh/formality), and
runs a lint gate when the agent stops.

## Install

```sh
cargo install --git https://github.com/arvinduh/ahok --locked
ahok doctor
```

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

## Commands

| Command                        | Does                                                  |
| ------------------------------ | ----------------------------------------------------- |
| `ahok hook <harness> <event>`  | Answers one hook call (`claude` today).               |
| `ahok doctor`                  | Checks the rules, `fml`/`git`/`mise`, and hook setup. |
| `ahok link [repo] [--dry-run]` | Symlinks a repo's configs from its `links.toml`.      |
| `ahok stats`                   | Shows p50/p99 hook latency and rule hit counts.       |

`ahok link` reads `[[link]]` entries with a `source` path in the repo and a
target in `home`, `linux` or `windows` (`~` and `%VAR%` expand). A trailing
`/*` on `source` links each child. `cloud = true` also links the entry in
Claude Code cloud sessions, which ahok detects from `CLAUDE_CODE_REMOTE`.
It never overwrites a real file: it moves it to `<name>.bak-<time>` first.

Config comes from `$AHOK_CONFIG_DIR` (default `~/.agents`). State lives in
`%LOCALAPPDATA%\ahok` on Windows and `~/.local/state/ahok` elsewhere:
`decisions.jsonl`, `errors.log`, and `sessions/`.

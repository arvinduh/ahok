# ahok

A fast, stateless control plane for coding-agent harnesses. One `ahok.toml`
places your instructions, skills and dotfiles, and wires Claude Code and Codex
to `ahok hook`, which enforces your rules before each tool call and runs your
checks after edits and when the agent stops.

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

## Commands

| Command                               | Does                                                   |
| ------------------------------------- | ------------------------------------------------------ |
| `ahok setup [repo...] [--check]`      | Applies each workspace's `ahok.toml`.                  |
| `ahok update [--check]`               | Updates `ahok` in place to the latest release.         |
| `ahok hook -C <ws> <harness> <event>` | Answers one hook call; `setup` registers it.           |
| `ahok git <hook> [args]`              | Runs the `GitHook` rules from `commit-msg`/`pre-push`. |

## `ahok.toml`

`ahok setup` reads `ahok.toml` at the root of each workspace (default: the
current directory) and puts it into place. `--check` changes nothing and exits 1
if anything is out of date.

```toml
[agents]
harnesses = ["claude", "codex", "agy"]
instructions = "AGENTS.md"      # linked to each harness's instructions file
skills = "skills"               # each child linked into each harness's skills
rules = "rules"                 # *.toml rules checked before each tool call
claude_settings = "claude.json" # extra keys for ~/.claude/settings.json

[[check]]
on = ["edit"]                   # edit: the edited files
run = "fml fmt {files}"         # {files} becomes the changed files

[[check]]
on = ["stop"]                   # stop: every file changed since the branch base
run = "cargo clippy --all-targets -- -D warnings"
changed = ["*.rs"]              # skip unless a changed file matches

[[link]]
from = "nvim/*"                 # a trailing /* links each child
to = "~/.config/nvim"

[[link]]
from = "format/ruff.toml"
to = { linux = "~/.config/ruff/ruff.toml", windows = "%APPDATA%/ruff/ruff.toml" }
on = ["linux", "windows"]       # default: linux, windows and cloud
```

Only one workspace may have `[agents]`. From it, `setup` writes the `hooks` key
of `~/.claude/settings.json` and `~/.codex/hooks.json`, registering only the
events the rules and checks need, and leaves every other key alone. Each hook
runs `ahok hook -C <workspace>`, so rules and checks are read live: edit them
without rerunning `setup`, unless a new kind of event is needed. Antigravity
gets links but no hooks yet.

A failed check blocks with its output: after an edit the agent sees it, and at
stop the agent keeps working. A check whose program is missing is skipped.

`~` and `%VAR%` expand in targets. Claude Code cloud sessions, which ahok
detects from `CLAUDE_CODE_REMOTE`, use the linux target. `setup` never
overwrites a real file: it moves it to `<name>.bak-<time>` first, and replaces a
symlinked JSON config with a real file holding the same keys.

## Hook behavior

The hook always exits 0 and writes JSON. Any internal failure is logged to
`errors.log` and the call is allowed. Codex asks you to trust a new hook the
first time it runs. Two rule kinds behave differently there, because Codex can't
do what they need:

- An `ask` rule denies, telling the agent to get your go-ahead in chat. Codex
  rejects `ask` replies and would run the call.
- A `require_skill` rule adds a note naming the skill instead of blocking. Codex
  has no Skill tool, so ahok can't see a skill load.

## Git hooks

`ahok git` runs from global git hooks: point `core.hooksPath` at scripts that
call `ahok git commit-msg "$@"` and `ahok git pre-push "$@"` (the agents repo
ships them in `git/hooks/`). It reads rules from `$AHOK_CONFIG_DIR/rules`
(default `~/.agents`). A broken rule prints its reason and exits 1, which stops
git; if the rules fail to load, it warns and lets the commit through. The diff
budget counts added plus deleted lines of the staged diff.

State lives in `%LOCALAPPDATA%\ahok` on Windows and `~/.local/state/ahok`
elsewhere: `errors.log` and `sessions/`.

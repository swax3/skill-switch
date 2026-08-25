# Skill Switch

A small Windows desktop app for managing [Claude Code](https://claude.com/claude-code) skills — toggle them on/off without touching the filesystem by hand.

Built with Tauri v2, React, Tailwind v4, and shadcn/ui.

## What it does

- Lists every skill folder in `%USERPROFILE%\.claude\skills` (active) and `%USERPROFILE%\.claude\skills-disabled` (inactive, created automatically if missing).
- Shows each skill's name and description (parsed from its `SKILL.md` frontmatter), with a toggle switch.
- Toggling **moves** the skill folder between the two directories — atomically, and only ever a move. It never deletes anything, and refuses (with a clear error) if a folder with the same name already exists at the destination.
- Works correctly with symlinked/junction skill folders (e.g. skills managed by a package manager) — the link itself moves, its target is untouched.
- Per-skill copy buttons for two ready-to-paste prompts, for toggling a skill in an already-running Claude Code chat (folder moves only take effect in new sessions):
  - **Activate**: `Lies <full path to SKILL.md> und wende diesen Skill ab jetzt an.`
  - **Ignore**: `Ignoriere ab sofort den Skill <name> vollständig.`
- Read-only view of installed plugins and configured MCP servers (from `~/.claude/plugins/installed_plugins.json`, `~/.claude/settings.json`, and `~/.claude.json`) — for visibility only; toggle those from Claude Code itself.
- Manual refresh button, since the list is only read on launch/toggle.
- Follows the OS light/dark theme; remembers window size and position.

All filesystem access happens in the Rust backend — the frontend has no direct file access and only talks to the three exposed commands (`list_skills`, `set_skill_enabled`, `read_env`).

## Requirements

- Windows 10/11
- [Node.js](https://nodejs.org/) 20+
- [Rust](https://www.rust-lang.org/tools/install) (`stable-x86_64-pc-windows-msvc` toolchain)
- Visual Studio Build Tools with the "Desktop development with C++" workload (required by Tauri's Windows target)

## Getting started

```bash
npm install
npm run tauri dev
```

## Building an installer

App icons aren't included in this repo (binary assets, kept out for a clean text-only diff history). Generate your own first from a 1024×1024 PNG:

```bash
npx tauri icon path/to/icon.png
```

Then build:

```bash
npm run tauri build
```

Produces an MSI and an NSIS installer under `src-tauri/target/release/bundle/`.

## Project layout

```
src/                 React frontend (no filesystem access)
  components/         SkillList, ConfigPanel, shadcn/ui primitives
  lib/api.ts          Typed wrapper around the Tauri commands
src-tauri/src/lib.rs  Rust backend: list_skills, set_skill_enabled, read_env
```

## License

[MIT](LICENSE)

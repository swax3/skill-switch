# Skill Switch

A small Windows desktop app for managing [Claude Code](https://claude.com/claude-code) skills — toggle them on/off without touching the filesystem by hand.

Built with Tauri v2, React, Tailwind v4, and shadcn/ui.

## What it does

- Lists every skill folder in `%USERPROFILE%\.claude\skills` (active) and `%USERPROFILE%\.claude\skills-disabled` (inactive, created automatically if missing).
- Shows each skill's name and description (parsed from its `SKILL.md` frontmatter), with a three-way state control:
  - **Aktiv** — normal, Claude sees and auto-invokes it.
  - **Manuell** — writes `skillOverrides[name] = "user-invocable-only"` to `~/.claude/settings.json` (patches just that one key, every other setting is left untouched). Claude no longer suggests or auto-invokes the skill, but it's still callable by typing `/name`.
  - **Aus** — **moves** the skill folder to `skills-disabled/`. Atomic, only ever a move, never deletes anything, and refuses (with a clear error) if a folder with the same name already exists at the destination.
- Switching to **Aus** also removes any leftover `skillOverrides` entry for that skill (best-effort tidy-up); switching a skill back to **Aktiv** clears the override too.
- Works correctly with symlinked/junction skill folders (e.g. skills managed by a package manager) — the link itself moves, its target is untouched.
- Both mechanisms only take effect in a **new** Claude Code session — `settings.json` is read once at launch, same as the filesystem. There's no way to hot-reload either mid-chat, which is exactly what the copy-prompt buttons below are for.
- **Groups skills by where they came from.** If `~/.agents/.skill-lock.json` exists (written by skill-registry tools that manage `~/.agents/skills` and link it into `~/.claude/skills` via junctions), each group header shows the source repo as a clickable link (opens in your default browser) and, for groups with more than one skill, its own three-way control to switch every skill in that group at once — handy since one "install" from such a registry can easily unpack into a dozen+ individual skills. A group's control shows no state selected when its members disagree (e.g. some active, some off). Skills with no lock entry — installed by hand — show under "Ohne bekannte Quelle" with no link, same as before this feature existed.
- Per-skill copy buttons for two ready-to-paste prompts, for toggling a skill in an already-running Claude Code chat (folder moves only take effect in new sessions):
  - **Activate**: `Lies <full path to SKILL.md> und wende diesen Skill ab jetzt an.`
  - **Ignore**: `Ignoriere ab sofort den Skill <name> vollständig.`
- Read-only view of installed plugins and configured MCP servers (from `~/.claude/plugins/installed_plugins.json`, `~/.claude/settings.json`, and `~/.claude.json`) — for visibility only; toggle those from Claude Code itself.
- **Profiles**: save the current set of skill states as a named preset (e.g. "Website-Projekt") and re-apply it later with one click. Applying only touches skills whose state actually differs from the profile — the rest are left alone — and skills the profile mentions but that aren't currently installed are skipped and reported, not treated as an error. Stored in its own `profiles.json` under the app's local data folder, independent of the skill folders and `settings.json` themselves.
- Manual refresh button, since the list is only read on launch/toggle.
- Follows the OS light/dark theme; remembers window size and position.

All filesystem/settings access happens in the Rust backend — the frontend has no direct file access and only talks to the four exposed skill commands (`list_skills`, `set_skill_enabled`, `set_skill_manual_only`, `read_env`). `set_skill_manual_only` never rewrites `settings.json` wholesale; it patches the `skillOverrides` key in place (with `serde_json`'s `preserve_order` feature, so the file's key order doesn't churn) and a mutex serializes concurrent writes. Opening a repo link goes through `@tauri-apps/plugin-shell`'s `open()` (a plain `<a target="_blank">` gets blocked by Tauri's navigation guard) — scoped in `capabilities/default.json` to `^https://` only.

### OmniRoute tab

Routes individual, deliberately-added "throwaway" project folders through a local [OmniRoute](https://github.com/diegosouzapw/OmniRoute) proxy instead of talking to Anthropic directly — **never globally**.

- **Terminal/CLI only.** Claude Code CLI reads `env.ANTHROPIC_BASE_URL`/`env.ANTHROPIC_AUTH_TOKEN` from a project's `.claude/settings.local.json`; Claude **Desktop does not** — Desktop has its own, separate, global-to-the-whole-app gateway mechanism with no per-project concept, so there is no way to route only some Desktop projects without it affecting every Desktop chat. This app deliberately doesn't try.
- **Two independent ways to route a session:**
  - **Persistent** — a per-project toggle patches (never replaces) `.claude/settings.local.json`'s `env` object with the two variables. Takes effect in terminal sessions started afterward — the fallback for anyone who just opens a normal terminal themselves.
  - **One-off** — an "open terminal" button per project launches Windows Terminal (or `cmd.exe` if `wt.exe` isn't installed) with the two variables set only on that one process's environment, then runs `claude`. Nothing is written to disk.
- Backups of the file being patched live centrally under the app's own local data folder (never inside the project — no git noise, no unignored plaintext-key file), capped at the 3 most recent per project.
- If a project's `.gitignore` doesn't cover `settings.local.json`, the app offers to append that line (never overwrites) rather than blocking the toggle.
- On launch, checks whether `ANTHROPIC_BASE_URL` is accidentally set globally — in `~/.claude/settings.json` or in the Windows registry (`HKCU`/`HKLM` `Environment`, not the process's own possibly-stale env snapshot) — and warns if so, since that would affect every project.
- The OmniRoute API key is stored write-only from the frontend's perspective (`apiKeySet: boolean`, never the plaintext, crosses the Tauri IPC boundary).
- **"Test connection" button** — Claude Code itself has no way to show which endpoint or model it's actually using, so this sends one tiny real request through the configured URL + key and displays the model that answered.

### Usage tab

Explains where your Claude Code usage limit goes, from the transcripts Claude Code already keeps locally (`~/.claude/projects/**/*.jsonl`).

- **Metadata only, nothing stored.** Reads token counts, model, effort, tool names and error/compaction/rate-limit markers per request. Prompt text and file contents are never evaluated, kept, or sent over IPC. No collector, no database, no hooks — the transcript files are the data source, analysed on demand when you open the tab.
- Deterministic findings with evidence: context size per request (the usual #1 driver — every request re-reads the whole context), long-lived sessions, workflow/multi-agent share, effort `xhigh` share, requests per turn, and how often the 5-hour limit was hit.
- Percentages are labelled honestly: *gemessen* (measured), *berechnet* (derived — the token weighting is a heuristic mirroring API price ratios; how the subscription limit weights cached tokens isn't observable locally), or *heuristik* (threshold rule). Thresholds live in one `RULE_*` block in `src-tauri/src/usage.rs`.
- A **trend** section compares the last 7 days against the 7 days before on the same metrics, plus a 14-day list and per-skill/per-MCP-server usage counts pulled from transcript attribution markers — a correlation ("this skill was active in N messages"), never a cost figure, and shown in the Skills and Plugins & MCP tabs too.

### Live tab

Shows which Claude Code sessions are writing right now (Desktop app and CLI both write the same transcripts), the context size their last request actually read, requests and failed tool calls in the current turn, session age, active subagents, and — if the transcript recorded a 5-hour-limit rejection — the reset countdown. Each session gets concrete advice for the next prompt (compact now / soon / fine, retry-loop suspicion, session too old, effort xhigh) with a one-click copy of the matching slash command. Polling only reads bytes appended since the previous poll, so it's cheap even on 100 MB transcripts. Nothing can be executed inside a running session from outside — the app prepares the command, you paste it. Polling now runs app-wide (not just while the tab is open), and a "Benachrichtigungen" toggle in the tab header raises one Windows notification per session per problem — compact-now, a retry loop, or the 5-hour reset — instead of one per 5-second poll. A one-click installable `UserPromptSubmit` hook mirrors the compact warning directly in the Claude Code chat itself (Desktop and CLI), so you see it even with the app closed.

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
src/                    React frontend (no filesystem access)
  components/            SkillList, ConfigPanel, OmniroutePanel, shadcn/ui primitives
  lib/api.ts             Typed wrapper around the Tauri commands
src-tauri/src/lib.rs      Rust backend: list_skills, set_skill_enabled, set_skill_manual_only, read_env
src-tauri/src/omniroute.rs  OmniRoute tab backend (11 commands, see CLAUDE.md)
```

## License

[MIT](LICENSE)

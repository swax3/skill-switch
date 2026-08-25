import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"

export type Skill = {
  name: string
  description: string
  skillMdPath: string | null
  enabled: boolean
  /** skillOverrides[name] === "user-invocable-only" — only meaningful while enabled. */
  manualOnly: boolean
  linked: boolean
  /** "owner/repo" from ~/.agents/.skill-lock.json, if this skill is tracked there. */
  source: string | null
  sourceUrl: string | null
}

export type McpServerInfo = {
  name: string
  scope: string
}

export type PluginInfo = {
  id: string
  version: string | null
  enabled: boolean
}

export type EnvInfo = {
  mcpServers: McpServerInfo[]
  plugins: PluginInfo[]
}

// Backend structs are #[serde(rename_all = "camelCase")], so field names line up as-is.
const isTauri = "__TAURI_INTERNALS__" in window

export async function listSkills(): Promise<Skill[]> {
  if (!isTauri) return mockSkills
  return invoke<Skill[]>("list_skills")
}

export async function setSkillEnabled(name: string, enabled: boolean): Promise<void> {
  if (!isTauri) {
    const skill = mockSkills.find((s) => s.name === name)
    if (skill) {
      skill.enabled = enabled
      if (!enabled) skill.manualOnly = false
    }
    return
  }
  await invoke("set_skill_enabled", { name, enabled })
}

export async function setSkillManualOnly(name: string, manualOnly: boolean): Promise<void> {
  if (!isTauri) {
    const skill = mockSkills.find((s) => s.name === name)
    if (skill) skill.manualOnly = manualOnly
    return
  }
  await invoke("set_skill_manual_only", { name, manualOnly })
}

export async function readEnv(): Promise<EnvInfo> {
  if (!isTauri) return mockEnv
  return invoke<EnvInfo>("read_env")
}

// Tauri blocks in-webview navigation to external origins by default; opening a repo
// link has to go through the shell plugin so it launches the OS's default browser.
export async function openExternal(url: string): Promise<void> {
  if (!isTauri) {
    window.open(url, "_blank", "noopener,noreferrer")
    return
  }
  await open(url)
}

// Dev-mock data so the UI can be previewed in a plain browser (no Tauri backend attached).
const mockSkills: Skill[] = [
  {
    name: "humanizer",
    description: "Rewrite AI-sounding text so it reads naturally without changing what it says.",
    skillMdPath: "C:\\Users\\you\\.claude\\skills\\humanizer\\SKILL.md",
    enabled: true,
    manualOnly: false,
    linked: true,
    source: "blader/humanizer",
    sourceUrl: "https://github.com/blader/humanizer",
  },
  {
    name: "playwright-cli",
    description: "Automate browser interactions, test web pages and work with Playwright tests.",
    skillMdPath: "C:\\Users\\you\\.claude\\skills\\playwright-cli\\SKILL.md",
    enabled: true,
    manualOnly: true,
    linked: false,
    source: null,
    sourceUrl: null,
  },
  {
    name: "gsap-core",
    description: "Official GSAP skill for the core API.",
    skillMdPath: "C:\\Users\\you\\.claude\\skills\\gsap-core\\SKILL.md",
    enabled: false,
    manualOnly: false,
    linked: true,
    source: "greensock/gsap-skills",
    sourceUrl: "https://github.com/greensock/gsap-skills",
  },
  {
    name: "gsap-utils",
    description: "gsap.utils helpers — clamp, mapRange, random, snap, toArray, wrap.",
    skillMdPath: "C:\\Users\\you\\.claude\\skills\\gsap-utils\\SKILL.md",
    enabled: true,
    manualOnly: false,
    linked: true,
    source: "greensock/gsap-skills",
    sourceUrl: "https://github.com/greensock/gsap-skills",
  },
]

const mockEnv: EnvInfo = {
  mcpServers: [{ name: "chrome-devtools", scope: "Global" }],
  plugins: [
    { id: "ponytail@ponytail", version: "4.9.0", enabled: true },
    { id: "impeccable@impeccable", version: "4.1.1", enabled: true },
  ],
}

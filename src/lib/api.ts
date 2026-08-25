import { invoke } from "@tauri-apps/api/core"

export type Skill = {
  name: string
  description: string
  skillMdPath: string | null
  enabled: boolean
  linked: boolean
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
    if (skill) skill.enabled = enabled
    return
  }
  await invoke("set_skill_enabled", { name, enabled })
}

export async function readEnv(): Promise<EnvInfo> {
  if (!isTauri) return mockEnv
  return invoke<EnvInfo>("read_env")
}

// Dev-mock data so the UI can be previewed in a plain browser (no Tauri backend attached).
const mockSkills: Skill[] = [
  {
    name: "humanizer",
    description: "Rewrite AI-sounding text so it reads naturally without changing what it says.",
    skillMdPath: "C:\\Users\\you\\.claude\\skills\\humanizer\\SKILL.md",
    enabled: true,
    linked: true,
  },
  {
    name: "playwright-cli",
    description: "Automate browser interactions, test web pages and work with Playwright tests.",
    skillMdPath: "C:\\Users\\you\\.claude\\skills\\playwright-cli\\SKILL.md",
    enabled: true,
    linked: false,
  },
  {
    name: "gsap-core",
    description: "Official GSAP skill for the core API.",
    skillMdPath: "C:\\Users\\you\\.claude\\skills\\gsap-core\\SKILL.md",
    enabled: false,
    linked: true,
  },
]

const mockEnv: EnvInfo = {
  mcpServers: [{ name: "chrome-devtools", scope: "Global" }],
  plugins: [
    { id: "ponytail@ponytail", version: "4.9.0", enabled: true },
    { id: "impeccable@impeccable", version: "4.1.1", enabled: true },
  ],
}

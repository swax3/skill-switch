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

export type ProjectRoute = {
  path: string
  exists: boolean
  /** env.ANTHROPIC_BASE_URL is present in that project's settings.local.json right now. */
  active: boolean
  baseUrl: string | null
  hasToken: boolean
  gitignoreOk: boolean
  /** env.CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY is set — only meaningful while active. */
  gatewayDiscovery: boolean
  error: string | null
}

export type OmniState = {
  baseUrl: string
  apiKeySet: boolean
  projects: ProjectRoute[]
}

export type GlobalRouteWarning = {
  settingsJson: string | null
  userEnv: string | null
  machineEnv: string | null
}

export type ConnectionTestResult = {
  model: string
  latencyMs: number
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

// --- OmniRoute -------------------------------------------------------------
// CLI/terminal only — Claude Desktop reads its own global, app-wide gateway
// config, not any settings.json, so per-project routing is out of reach there.

export async function readOmniroute(): Promise<OmniState> {
  if (!isTauri) return mockOmniState
  return invoke<OmniState>("read_omniroute")
}

export async function setOmnirouteConfig(baseUrl: string, apiKey?: string): Promise<void> {
  if (!isTauri) {
    mockOmniState.baseUrl = baseUrl
    if (apiKey) mockOmniState.apiKeySet = true
    return
  }
  await invoke("set_omniroute_config", { baseUrl, apiKey: apiKey ?? null })
}

export async function addOmnirouteProject(path: string): Promise<void> {
  if (!isTauri) {
    mockOmniState.projects.push({
      path,
      exists: true,
      active: false,
      baseUrl: null,
      hasToken: false,
      gitignoreOk: false,
      gatewayDiscovery: false,
      error: null,
    })
    return
  }
  await invoke("add_omniroute_project", { path })
}

export async function removeOmnirouteProject(path: string): Promise<void> {
  if (!isTauri) {
    mockOmniState.projects = mockOmniState.projects.filter((p) => p.path !== path)
    return
  }
  await invoke("remove_omniroute_project", { path })
}

/** Returns a warning string if the route was applied but something needs attention. */
export async function setProjectRoute(path: string, active: boolean): Promise<string | null> {
  if (!isTauri) {
    const project = mockOmniState.projects.find((p) => p.path === path)
    if (project) {
      project.active = active
      project.baseUrl = active ? mockOmniState.baseUrl : null
      project.hasToken = active
      if (!active) project.gatewayDiscovery = false
    }
    return null
  }
  return invoke<string | null>("set_project_route", { path, active })
}

/** Returns a warning string if the change was applied but something needs attention. */
export async function setGatewayDiscovery(path: string, enabled: boolean): Promise<string | null> {
  if (!isTauri) {
    const project = mockOmniState.projects.find((p) => p.path === path)
    if (project) project.gatewayDiscovery = enabled
    return null
  }
  return invoke<string | null>("set_gateway_discovery", { path, enabled })
}

export async function addGitignoreEntry(path: string): Promise<void> {
  if (!isTauri) {
    const project = mockOmniState.projects.find((p) => p.path === path)
    if (project) project.gitignoreOk = true
    return
  }
  await invoke("add_gitignore_entry", { path })
}

export async function omnirouteStatus(): Promise<boolean> {
  if (!isTauri) return mockServiceRunning
  return invoke<boolean>("omniroute_status")
}

export async function startOmniroute(): Promise<void> {
  if (!isTauri) {
    mockServiceRunning = true
    return
  }
  await invoke("start_omniroute")
}

export async function stopOmniroute(): Promise<void> {
  if (!isTauri) {
    mockServiceRunning = false
    return
  }
  await invoke("stop_omniroute")
}

export async function openOmnirouteTerminal(path: string): Promise<void> {
  if (!isTauri) {
    window.alert(`(Vorschau) Würde ein Terminal in ${path} mit OmniRoute-Routing öffnen.`)
    return
  }
  await invoke("open_omniroute_terminal", { path })
}

export async function checkGlobalRoute(): Promise<GlobalRouteWarning> {
  if (!isTauri) return mockGlobalRoute
  return invoke<GlobalRouteWarning>("check_global_route")
}

/** Sends one tiny real request through the configured route and reports which
 *  model answered — uses OmniRoute's own "auto" routing, so this confirms the
 *  proxy itself works, not necessarily what a specific Claude Code model resolves to. */
export async function testOmnirouteConnection(): Promise<ConnectionTestResult> {
  if (!isTauri) return { model: "auto/best-fast → gpt-5.5 (Vorschau)", latencyMs: 420 }
  return invoke<ConnectionTestResult>("test_omniroute_connection")
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

const mockOmniState: OmniState = {
  baseUrl: "http://localhost:20128/v1",
  apiKeySet: true,
  projects: [
    {
      path: "C:\\Users\\you\\Code\\wegwerf-projekt",
      exists: true,
      active: true,
      baseUrl: "http://localhost:20128/v1",
      hasToken: true,
      gitignoreOk: true,
      gatewayDiscovery: false,
      error: null,
    },
    {
      path: "C:\\Users\\you\\Code\\anderes-projekt",
      exists: true,
      active: false,
      baseUrl: null,
      hasToken: false,
      gitignoreOk: false,
      gatewayDiscovery: false,
      error: null,
    },
  ],
}

let mockServiceRunning = false

const mockGlobalRoute: GlobalRouteWarning = {
  settingsJson: null,
  userEnv: null,
  machineEnv: null,
}

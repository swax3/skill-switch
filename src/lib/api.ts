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

// --- Usage (metadata-only analysis of ~/.claude/projects transcripts) ---------

export type Tokens = {
  requests: number
  input: number
  cacheWrite: number
  cacheRead: number
  output: number
}

export type Named = { name: string; tokens: Tokens; sharePct: number }
export type Bucket = { label: string; requests: number; requestsPct: number; volume: number; volumePct: number }
export type ToolCount = { name: string; count: number }

export type SessionRow = {
  id: string
  project: string
  startDay: string
  durationMin: number
  turns: number
  requests: number
  requestsPerTurn: number
  maxContext: number
  subagents: number
  workflowAgents: number
  compactions: number
  errors: number
  agentSharePct: number
  sharePct: number
  models: string
}

export type Finding = {
  severity: "high" | "medium" | "low" | "info"
  title: string
  evidence: string
  recommendation: string
  basis: "gemessen" | "berechnet" | "heuristik"
}

export type DayRow = { day: string; requests: number; avgContext: number; turns: number; weighted: number }

export type PeriodStats = {
  daysActive: number
  requests: number
  avgContext: number
  requestsPerTurn: number
  xhighPct: number
  workflowPct: number
  fiveHourHits: number
  weighted: number
}

export type Trend = { current: PeriodStats; previous: PeriodStats }

/** A marker that a skill or MCP server was involved in a message — correlation, never a cost. */
export type AttributionUsage = { name: string; messages: number; sessions: number; lastDay: string | null }

export type UsageReport = {
  files: number
  unreadableLines: number
  firstDay: string | null
  lastDay: string | null
  total: Tokens
  byKind: Named[]
  byModel: Named[]
  byEffort: Named[]
  contextBuckets: Bucket[]
  avgContext: number
  cacheReadPct: number
  largeContextVolumePct: number
  apiErrors: number
  toolErrors: number
  compactions: number
  fiveHourHits: number
  lastFiveHourReset: number | null
  mainSessions: number
  totalTurns: number
  avgRequestsPerTurn: number
  sessionsOver150k: number
  sessionsCompacted: number
  longSessions: number
  longestSessionDays: number
  longestSessionTurns: number
  workflowSharePct: number
  workflowAgents: number
  sessionsWithManyWorkflowAgents: number
  maxWorkflowAgentsInSession: number
  xhighSharePct: number
  topToolsMain: ToolCount[]
  topToolsAgents: ToolCount[]
  sessions: SessionRow[]
  findings: Finding[]
  days: DayRow[]
  trend: Trend
  skillUsage: AttributionUsage[]
  mcpUsage: AttributionUsage[]
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

/** Metadata-only pass over every transcript in ~/.claude/projects. Slow-ish (reads
 *  hundreds of MB), so callers show a loading state and cache the result. */
export async function analyzeUsage(): Promise<UsageReport> {
  if (!isTauri) return mockUsage
  return invoke<UsageReport>("analyze_usage")
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

const mockTokens = (requests: number, ctx: number, output: number): Tokens => ({
  requests,
  input: Math.round(ctx * 0.02),
  cacheWrite: Math.round(ctx * 0.04),
  cacheRead: Math.round(ctx * 0.94),
  output,
})

const mockUsage: UsageReport = {
  files: 240,
  unreadableLines: 2,
  firstDay: "2026-08-01",
  lastDay: "2026-08-28",
  total: mockTokens(12_000, 2_400_000_000, 15_000_000),
  byKind: [
    { name: "Hauptsession", tokens: mockTokens(6_000, 1_500_000_000, 8_000_000), sharePct: 62 },
    { name: "Subagent (Agent-Tool)", tokens: mockTokens(1_500, 150_000_000, 1_200_000), sharePct: 6 },
    { name: "Workflow-Agent", tokens: mockTokens(4_500, 750_000_000, 5_800_000), sharePct: 32 },
  ],
  byModel: [
    { name: "claude-opus-5-5", tokens: mockTokens(7_000, 1_600_000_000, 9_000_000), sharePct: 64 },
    { name: "claude-sonnet-5-5", tokens: mockTokens(4_000, 650_000_000, 5_000_000), sharePct: 28 },
    { name: "claude-haiku-4-5", tokens: mockTokens(1_000, 150_000_000, 1_000_000), sharePct: 8 },
  ],
  byEffort: [
    { name: "high", tokens: mockTokens(6_000, 1_200_000_000, 7_500_000), sharePct: 50 },
    { name: "xhigh", tokens: mockTokens(4_000, 960_000_000, 6_000_000), sharePct: 40 },
    { name: "unbekannt", tokens: mockTokens(2_000, 240_000_000, 1_500_000), sharePct: 10 },
  ],
  contextBuckets: [
    { label: "<50k", requests: 3_000, requestsPct: 25, volume: 75_000_000, volumePct: 3 },
    { label: "50–100k", requests: 3_600, requestsPct: 30, volume: 270_000_000, volumePct: 11 },
    { label: "100–150k", requests: 2_000, requestsPct: 17, volume: 250_000_000, volumePct: 10 },
    { label: "150–200k", requests: 1_000, requestsPct: 8, volume: 175_000_000, volumePct: 7 },
    { label: ">200k", requests: 2_400, requestsPct: 20, volume: 1_630_000_000, volumePct: 68 },
  ],
  avgContext: 200_000,
  cacheReadPct: 94,
  largeContextVolumePct: 68,
  apiErrors: 40,
  toolErrors: 310,
  compactions: 12,
  fiveHourHits: 9,
  lastFiveHourReset: null,
  mainSessions: 18,
  totalTurns: 600,
  avgRequestsPerTurn: 10,
  sessionsOver150k: 11,
  sessionsCompacted: 4,
  longSessions: 5,
  longestSessionDays: 12.5,
  longestSessionTurns: 140,
  workflowSharePct: 32,
  workflowAgents: 220,
  sessionsWithManyWorkflowAgents: 2,
  maxWorkflowAgentsInSession: 90,
  xhighSharePct: 40,
  topToolsMain: [
    { name: "Bash", count: 1200 },
    { name: "Edit", count: 900 },
    { name: "Read", count: 400 },
    { name: "Grep", count: 250 },
  ],
  topToolsAgents: [
    { name: "Read", count: 2100 },
    { name: "Bash", count: 1800 },
    { name: "WebSearch", count: 900 },
    { name: "WebFetch", count: 700 },
  ],
  sessions: [
    { id: "a3f9c210", project: "my-website", startDay: "2026-08-04", durationMin: 18_000, turns: 140, requests: 2_100, requestsPerTurn: 10, maxContext: 640_000, subagents: 3, workflowAgents: 40, compactions: 3, errors: 45, agentSharePct: 20, sharePct: 22, models: "opus-5-5:1800 sonnet-5-5:300" },
    { id: "7b2e4d91", project: "data-pipeline", startDay: "2026-08-12", durationMin: 4_300, turns: 45, requests: 1_300, requestsPerTurn: 12, maxContext: 410_000, subagents: 2, workflowAgents: 90, compactions: 1, errors: 30, agentSharePct: 55, sharePct: 14, models: "opus-5-5:1100 sonnet-5-5:200" },
  ],
  findings: [
    { severity: "high", title: "Kontextgröße ist der größte Treiber", evidence: "68% des Kontextvolumens stammen aus Requests mit >200k Kontext. Ø 200k Tokens pro Request. 11 von 18 Sessions über 150k, nur 4 haben je kompaktiert.", recommendation: "Pro Aufgabe eine neue Session starten. Bei Themenwechsel /clear, bei langen Aufgaben früher /compact.", basis: "gemessen" },
    { severity: "high", title: "Workflows / Multi-Agent-Runden", evidence: "220 Workflow-Agents, 32% der gewichteten Nutzung.", recommendation: "Workflows als Ausnahme, nicht als Standard.", basis: "berechnet" },
    { severity: "medium", title: "Effort xhigh ist der Standard", evidence: "40% der gewichteten Nutzung liefen mit effort xhigh.", recommendation: "Effort high als Standard; xhigh gezielt.", basis: "berechnet" },
    { severity: "info", title: "5h-Limit erreicht", evidence: "9 Mal laut Transcript-Marker.", recommendation: "Eine Live-Prozentanzeige gibt es lokal nicht.", basis: "gemessen" },
  ],
  days: [
    "2026-08-15", "2026-08-16", "2026-08-17", "2026-08-18", "2026-08-19", "2026-08-20", "2026-08-21",
    "2026-08-22", "2026-08-23", "2026-08-24", "2026-08-25", "2026-08-26", "2026-08-27", "2026-08-28",
  ].map((day, i) => {
    const inCurrentWeek = i >= 7
    const requests = inCurrentWeek ? 180 + i * 5 : 430 - i * 10
    return {
      day,
      requests,
      avgContext: inCurrentWeek ? 340_000 : 255_000,
      turns: Math.round(requests / (inCurrentWeek ? 6 : 18)),
      weighted: requests * (inCurrentWeek ? 900_000 : 1_400_000),
    }
  }),
  trend: {
    current: { daysActive: 5, requests: 1_400, avgContext: 230_000, requestsPerTurn: 7.5, xhighPct: 35, workflowPct: 20, fiveHourHits: 3, weighted: 1.0e12 },
    previous: { daysActive: 6, requests: 2_200, avgContext: 260_000, requestsPerTurn: 11, xhighPct: 45, workflowPct: 30, fiveHourHits: 4, weighted: 1.8e12 },
  },
  skillUsage: [
    { name: "humanizer", messages: 24, sessions: 4, lastDay: "2026-08-27" },
    { name: "gsap-utils", messages: 9, sessions: 2, lastDay: "2026-08-20" },
  ],
  mcpUsage: [
    { name: "chrome-devtools", messages: 120, sessions: 4, lastDay: "2026-08-26" },
    { name: "Claude Browser", messages: 300, sessions: 7, lastDay: "2026-08-28" },
  ],
}

const mockGlobalRoute: GlobalRouteWarning = {
  settingsJson: null,
  userEnv: null,
  machineEnv: null,
}

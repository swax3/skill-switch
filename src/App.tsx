import { useEffect, useState } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { Info, Minus, Plug, Puzzle, Route, Square, X } from "lucide-react"
import { toast } from "sonner"
import {
  addOmnirouteProject,
  checkGlobalRoute,
  listSkills,
  readEnv,
  readOmniroute,
  removeOmnirouteProject,
  setGatewayDiscovery,
  setOmnirouteConfig,
  setProjectRoute,
  setSkillEnabled,
  setSkillManualOnly,
  type EnvInfo,
  type GlobalRouteWarning,
  type OmniState,
  type Skill,
} from "@/lib/api"
import { SkillList, type SkillState } from "@/components/skill-list"
import { ConfigPanel } from "@/components/config-panel"
import { OmniroutePanel } from "@/components/omniroute-panel"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

type View = "skills" | "config" | "omniroute"

const appWindow = "__TAURI_INTERNALS__" in window ? getCurrentWindow() : null

const SIDEBAR_NOTE: Partial<Record<View, string>> = {
  skills:
    'Alle drei Zustände wirken erst in der nächsten Claude-Code-Session. In laufenden Chats: die Kopier-Buttons nutzen.',
  omniroute:
    'Gilt nur für Terminal-Sessions in diesem Ordner. Die Desktop-App läuft immer über Anthropic.',
}

function App() {
  const [view, setView] = useState<View>("skills")
  const [skills, setSkills] = useState<Skill[]>([])
  const [skillsLoading, setSkillsLoading] = useState(true)
  const [pending, setPending] = useState<Set<string>>(new Set())
  const [env, setEnv] = useState<EnvInfo | null>(null)
  const [envLoading, setEnvLoading] = useState(true)
  const [omni, setOmni] = useState<OmniState | null>(null)
  const [omniLoading, setOmniLoading] = useState(true)
  const [globalWarn, setGlobalWarn] = useState<GlobalRouteWarning | null>(null)

  const refreshSkills = () =>
    listSkills()
      .then(setSkills)
      .catch((e) => toast.error(String(e)))
      .finally(() => setSkillsLoading(false))

  const refreshOmni = () =>
    readOmniroute()
      .then(setOmni)
      .catch((e) => toast.error(String(e)))
      .finally(() => setOmniLoading(false))

  useEffect(() => {
    refreshSkills()
    refreshOmni()
    readEnv()
      .then(setEnv)
      .catch((e) => toast.error(String(e)))
      .finally(() => setEnvLoading(false))
    // App-startup check, not tab-open: an accidental global override affects every
    // project, not just ones opted into OmniRoute, so it needs to surface up front.
    checkGlobalRoute()
      .then((w) => setGlobalWarn(w.settingsJson || w.userEnv || w.machineEnv ? w : null))
      .catch((e) => toast.error(String(e)))
  }, [])

  const STATE_LABEL: Record<SkillState, string> = {
    active: "aktiv",
    manual: "nur manuell (per /name aufrufbar)",
    disabled: "deaktiviert",
  }

  // Core transition, no pending-tracking/toast — shared by the single-skill and
  // group handlers below so the two don't drift out of sync.
  async function applyState(skill: Skill, next: SkillState) {
    if (next === "disabled") {
      await setSkillEnabled(skill.name, false)
      return
    }
    if (!skill.enabled) await setSkillEnabled(skill.name, true)
    await setSkillManualOnly(skill.name, next === "manual")
  }

  function withPending<T>(names: string[], fn: () => Promise<T>) {
    setPending((p) => {
      const next = new Set(p)
      names.forEach((n) => next.add(n))
      return next
    })
    return fn().finally(() => {
      setPending((p) => {
        const next = new Set(p)
        names.forEach((n) => next.delete(n))
        return next
      })
    })
  }

  async function handleStateChange(skill: Skill, next: SkillState) {
    await withPending([skill.name], async () => {
      try {
        await applyState(skill, next)
        toast.success(`„${skill.name}" ist jetzt ${STATE_LABEL[next]}. Wirkt ab der nächsten Claude-Code-Session.`)
        await refreshSkills()
      } catch (e) {
        toast.error(String(e))
      }
    })
  }

  async function handleGroupStateChange(groupSkills: Skill[], next: SkillState) {
    await withPending(
      groupSkills.map((s) => s.name),
      async () => {
        const results = await Promise.allSettled(groupSkills.map((s) => applyState(s, next)))
        const failed = results.filter((r) => r.status === "rejected").length
        if (failed === 0) {
          toast.success(
            `${groupSkills.length} Skills sind jetzt ${STATE_LABEL[next]}. Wirkt ab der nächsten Claude-Code-Session.`
          )
        } else {
          toast.error(`${failed} von ${groupSkills.length} Skills konnten nicht umgestellt werden.`)
        }
        await refreshSkills()
      }
    )
  }

  async function handleSaveOmniConfig(baseUrl: string, apiKey?: string) {
    await setOmnirouteConfig(baseUrl, apiKey)
    await refreshOmni()
  }

  async function handleAddProject(path: string) {
    await withPending([path], async () => {
      await addOmnirouteProject(path)
      await refreshOmni()
    })
  }

  async function handleRemoveProject(path: string) {
    await withPending([path], async () => {
      await removeOmnirouteProject(path)
      await refreshOmni()
    })
  }

  async function handleToggleProject(path: string, active: boolean) {
    await withPending([path], async () => {
      try {
        const warning = await setProjectRoute(path, active)
        toast.success(
          `Routing für „${path}" ist jetzt ${active ? "an" : "aus"}. Wirkt ab der nächsten Terminal-Session.`
        )
        if (warning) toast.warning(warning)
        await refreshOmni()
      } catch (e) {
        toast.error(String(e))
      }
    })
  }

  async function handleToggleGatewayDiscovery(path: string, enabled: boolean) {
    await withPending([path], async () => {
      try {
        const warning = await setGatewayDiscovery(path, enabled)
        toast.success(
          `Gateway-Modellauswahl für „${path}" ist jetzt ${enabled ? "an" : "aus"}. Wirkt ab der nächsten Terminal-Session.`
        )
        if (warning) toast.warning(warning)
        await refreshOmni()
      } catch (e) {
        toast.error(String(e))
      }
    })
  }

  const activeRouteCount = omni?.projects.filter((p) => p.active).length ?? 0

  return (
    <TooltipProvider>
      <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
        <TitleBar />
        <div className="flex min-h-0 flex-1">
          <Sidebar view={view} onChange={setView} omniBadge={activeRouteCount} />
          <main className="min-w-0 flex-1 overflow-hidden">
            {view === "skills" ? (
              <SkillList
                skills={skills}
                loading={skillsLoading}
                pending={pending}
                onStateChange={handleStateChange}
                onGroupStateChange={handleGroupStateChange}
                onRefresh={refreshSkills}
              />
            ) : view === "config" ? (
              <ConfigPanel env={env} loading={envLoading} />
            ) : (
              <OmniroutePanel
                omni={omni}
                loading={omniLoading}
                pending={pending}
                globalWarn={globalWarn}
                onRefresh={refreshOmni}
                onSaveConfig={handleSaveOmniConfig}
                onAddProject={handleAddProject}
                onRemoveProject={handleRemoveProject}
                onToggleProject={handleToggleProject}
                onToggleGatewayDiscovery={handleToggleGatewayDiscovery}
              />
            )}
          </main>
        </div>
      </div>
      <Toaster position="bottom-right" />
    </TooltipProvider>
  )
}

function TitleBar() {
  return (
    <div
      data-tauri-drag-region
      className="flex h-[38px] shrink-0 items-center justify-between border-b border-border pl-4"
    >
      <span data-tauri-drag-region className="text-xs font-medium text-muted-foreground">
        Skill Switch
      </span>
      <div className="flex h-full items-center">
        <WindowButton onClick={() => appWindow?.minimize()} label="Minimieren">
          <Minus className="size-3.5" />
        </WindowButton>
        <WindowButton onClick={() => appWindow?.toggleMaximize()} label="Maximieren">
          <Square className="size-3" />
        </WindowButton>
        <WindowButton onClick={() => appWindow?.close()} label="Schließen" danger>
          <X className="size-3.5" />
        </WindowButton>
      </div>
    </div>
  )
}

function WindowButton({
  children,
  onClick,
  label,
  danger,
}: {
  children: React.ReactNode
  onClick: () => void
  label: string
  danger?: boolean
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className={cn(
        "flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
        danger && "hover:bg-destructive hover:text-white"
      )}
    >
      {children}
    </button>
  )
}

function Sidebar({
  view,
  onChange,
  omniBadge,
}: {
  view: View
  onChange: (v: View) => void
  omniBadge: number
}) {
  const note = SIDEBAR_NOTE[view]
  return (
    <aside className="flex w-52 shrink-0 flex-col justify-between border-r border-border p-3">
      <nav className="space-y-0.5">
        <NavItem
          icon={Puzzle}
          label="Skills"
          active={view === "skills"}
          onClick={() => onChange("skills")}
        />
        <NavItem
          icon={Plug}
          label="Plugins & MCP"
          active={view === "config"}
          onClick={() => onChange("config")}
        />
        <NavItem
          icon={Route}
          label="OmniRoute"
          active={view === "omniroute"}
          onClick={() => onChange("omniroute")}
          badge={omniBadge}
        />
      </nav>

      {note && (
        <div className="flex items-start gap-1.5 rounded-lg px-2 py-2 text-[11px] leading-snug text-muted-foreground">
          <Info className="mt-0.5 size-3.5 shrink-0" />
          <p>{note}</p>
        </div>
      )}
    </aside>
  )
}

function NavItem({
  icon: Icon,
  label,
  active,
  onClick,
  badge,
}: {
  icon: typeof Puzzle
  label: string
  active: boolean
  onClick: () => void
  badge?: number
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm transition-colors",
        active ? "bg-primary text-primary-foreground" : "text-foreground hover:bg-accent"
      )}
    >
      <Icon className="size-4" />
      <span className="flex-1">{label}</span>
      {!!badge && (
        <span
          className={cn(
            "rounded-full px-1.5 text-[11px] font-medium",
            active ? "bg-primary-foreground/20" : "bg-warning/15 text-warning"
          )}
        >
          {badge}
        </span>
      )}
    </button>
  )
}

export default App

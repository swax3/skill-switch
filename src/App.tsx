import { useEffect, useState } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { Info, Minus, Plug, Puzzle, Square, X } from "lucide-react"
import { toast } from "sonner"
import {
  listSkills,
  readEnv,
  setSkillEnabled,
  setSkillManualOnly,
  type EnvInfo,
  type Skill,
} from "@/lib/api"
import { SkillList, type SkillState } from "@/components/skill-list"
import { ConfigPanel } from "@/components/config-panel"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

type View = "skills" | "config"

const appWindow = "__TAURI_INTERNALS__" in window ? getCurrentWindow() : null

function App() {
  const [view, setView] = useState<View>("skills")
  const [skills, setSkills] = useState<Skill[]>([])
  const [skillsLoading, setSkillsLoading] = useState(true)
  const [pending, setPending] = useState<Set<string>>(new Set())
  const [env, setEnv] = useState<EnvInfo | null>(null)
  const [envLoading, setEnvLoading] = useState(true)

  const refreshSkills = () =>
    listSkills()
      .then(setSkills)
      .catch((e) => toast.error(String(e)))
      .finally(() => setSkillsLoading(false))

  useEffect(() => {
    refreshSkills()
    readEnv()
      .then(setEnv)
      .catch((e) => toast.error(String(e)))
      .finally(() => setEnvLoading(false))
  }, [])

  const STATE_LABEL: Record<SkillState, string> = {
    active: "aktiv",
    manual: "nur manuell (per /name aufrufbar)",
    disabled: "deaktiviert",
  }

  async function handleStateChange(skill: Skill, next: SkillState) {
    setPending((p) => new Set(p).add(skill.name))
    try {
      if (next === "disabled") {
        await setSkillEnabled(skill.name, false)
      } else {
        if (!skill.enabled) await setSkillEnabled(skill.name, true)
        await setSkillManualOnly(skill.name, next === "manual")
      }
      toast.success(`„${skill.name}" ist jetzt ${STATE_LABEL[next]}. Wirkt ab der nächsten Claude-Code-Session.`)
      await refreshSkills()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setPending((p) => {
        const rest = new Set(p)
        rest.delete(skill.name)
        return rest
      })
    }
  }

  return (
    <TooltipProvider>
      <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
        <TitleBar />
        <div className="flex min-h-0 flex-1">
          <Sidebar view={view} onChange={setView} />
          <main className="min-w-0 flex-1 overflow-hidden">
            {view === "skills" ? (
              <SkillList
                skills={skills}
                loading={skillsLoading}
                pending={pending}
                onStateChange={handleStateChange}
                onRefresh={refreshSkills}
              />
            ) : (
              <ConfigPanel env={env} loading={envLoading} />
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

function Sidebar({ view, onChange }: { view: View; onChange: (v: View) => void }) {
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
      </nav>

      <div className="flex items-start gap-1.5 rounded-lg px-2 py-2 text-[11px] leading-snug text-muted-foreground">
        <Info className="mt-0.5 size-3.5 shrink-0" />
        <p>
          Alle drei Zustände wirken erst in der nächsten Claude-Code-Session. In laufenden Chats:
          die Kopier-Buttons nutzen.
        </p>
      </div>
    </aside>
  )
}

function NavItem({
  icon: Icon,
  label,
  active,
  onClick,
}: {
  icon: typeof Puzzle
  label: string
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm transition-colors",
        active
          ? "bg-primary text-primary-foreground"
          : "text-foreground hover:bg-accent"
      )}
    >
      <Icon className="size-4" />
      {label}
    </button>
  )
}

export default App

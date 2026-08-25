import { useEffect, useState } from "react"
import {
  AlertTriangle,
  Folder,
  Play,
  RefreshCw,
  Square,
  Terminal,
  Trash2,
} from "lucide-react"
import { toast } from "sonner"
import {
  addGitignoreEntry,
  omnirouteStatus,
  openOmnirouteTerminal,
  startOmniroute,
  stopOmniroute,
  type GlobalRouteWarning,
  type OmniState,
  type ProjectRoute,
} from "@/lib/api"
import { Input } from "@/components/ui/input"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

const WARNING_TEXT =
  'In markierten Projekten geht mein Quellcode an fremde Anbieter. Nicht für Kunden- oder Privatprojekte.'

type OmniroutePanelProps = {
  omni: OmniState | null
  loading: boolean
  pending: Set<string>
  globalWarn: GlobalRouteWarning | null
  onRefresh: () => void
  onSaveConfig: (baseUrl: string, apiKey?: string) => Promise<void>
  onAddProject: (path: string) => Promise<void>
  onRemoveProject: (path: string) => Promise<void>
  onToggleProject: (path: string, active: boolean) => Promise<void>
}

export function OmniroutePanel({
  omni,
  loading,
  pending,
  globalWarn,
  onRefresh,
  onSaveConfig,
  onAddProject,
  onRemoveProject,
  onToggleProject,
}: OmniroutePanelProps) {
  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-6">
      <WarnBanner />
      {globalWarn && <GlobalLeakBanner warn={globalWarn} />}

      {loading || !omni ? (
        <p className="text-sm text-muted-foreground">Wird geladen…</p>
      ) : (
        <>
          <ServiceCard baseUrl={omni.baseUrl} />
          <SettingsCard baseUrl={omni.baseUrl} apiKeySet={omni.apiKeySet} onSave={onSaveConfig} />
          <ProjectsSection
            omni={omni}
            pending={pending}
            onRefresh={onRefresh}
            onAddProject={onAddProject}
            onRemoveProject={onRemoveProject}
            onToggleProject={onToggleProject}
          />
        </>
      )}
    </div>
  )
}

function WarnBanner() {
  return (
    <p className="flex items-start gap-2 rounded-[10px] border border-warning/40 bg-warning/10 px-4 py-3 text-xs text-warning">
      <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
      {WARNING_TEXT}
    </p>
  )
}

function GlobalLeakBanner({ warn }: { warn: GlobalRouteWarning }) {
  const hits: string[] = []
  if (warn.settingsJson) hits.push(`~/.claude/settings.json: ${warn.settingsJson}`)
  if (warn.userEnv) hits.push(`Benutzer-Umgebungsvariable: ${warn.userEnv}`)
  if (warn.machineEnv) hits.push(`System-Umgebungsvariable: ${warn.machineEnv}`)
  if (hits.length === 0) return null
  return (
    <div className="rounded-[10px] border border-destructive/40 bg-destructive/10 px-4 py-3 text-xs text-destructive">
      <p className="flex items-center gap-2 font-medium">
        <AlertTriangle className="size-3.5 shrink-0" />
        ANTHROPIC_BASE_URL ist global gesetzt — das betrifft ALLE Projekte, nicht nur
        markierte.
      </p>
      <ul className="mt-1.5 list-disc space-y-0.5 pl-5">
        {hits.map((h) => (
          <li key={h}>{h}</li>
        ))}
      </ul>
    </div>
  )
}

function ServiceCard({ baseUrl }: { baseUrl: string }) {
  const [running, setRunning] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    let cancelled = false
    const check = () => omnirouteStatus().then((r) => !cancelled && setRunning(r))
    check()
    const id = setInterval(check, 2000)
    return () => {
      cancelled = true
      clearInterval(id)
    }
  }, [baseUrl])

  async function handleStart() {
    setBusy(true)
    try {
      await startOmniroute()
      toast.success("OmniRoute gestartet.")
      setRunning(await omnirouteStatus())
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusy(false)
    }
  }

  async function handleStop() {
    setBusy(true)
    try {
      await stopOmniroute()
      toast.success("OmniRoute gestoppt.")
      setRunning(await omnirouteStatus())
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="flex items-center justify-between rounded-[10px] border border-border bg-card px-4 py-3">
      <div className="flex items-center gap-2.5">
        <span
          className={cn(
            "inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium",
            running ? "bg-primary/15 text-primary" : "bg-muted text-muted-foreground"
          )}
        >
          <span
            className={cn(
              "size-1.5 rounded-full",
              running ? "bg-primary" : "bg-muted-foreground/50"
            )}
          />
          {running === null ? "prüfe…" : running ? "läuft" : "läuft nicht"}
        </span>
        <span className="text-xs text-muted-foreground">{baseUrl}</span>
      </div>
      <div className="flex gap-2">
        <button
          type="button"
          disabled={busy || running === true}
          onClick={handleStart}
          className="inline-flex items-center gap-1.5 rounded-lg bg-primary px-2.5 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary/80 disabled:pointer-events-none disabled:opacity-40"
        >
          <Play className="size-3.5" />
          Starten
        </button>
        <button
          type="button"
          disabled={busy || running === false}
          onClick={handleStop}
          className="inline-flex items-center gap-1.5 rounded-lg bg-secondary px-2.5 py-1.5 text-xs font-medium text-secondary-foreground transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-40"
        >
          <Square className="size-3.5" />
          Stoppen
        </button>
      </div>
    </section>
  )
}

function SettingsCard({
  baseUrl,
  apiKeySet,
  onSave,
}: {
  baseUrl: string
  apiKeySet: boolean
  onSave: (baseUrl: string, apiKey?: string) => Promise<void>
}) {
  const [urlInput, setUrlInput] = useState(baseUrl)
  const [keyInput, setKeyInput] = useState("")
  const [saving, setSaving] = useState(false)

  useEffect(() => setUrlInput(baseUrl), [baseUrl])

  async function handleSave() {
    setSaving(true)
    try {
      await onSave(urlInput, keyInput || undefined)
      toast.success("Einstellungen gespeichert.")
      setKeyInput("")
    } catch (e) {
      toast.error(String(e))
    } finally {
      setSaving(false)
    }
  }

  return (
    <section className="rounded-[10px] border border-border bg-card p-4">
      <h2 className="mb-3 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        Einstellungen
      </h2>
      <div className="space-y-2.5">
        <label className="block">
          <span className="mb-1 block text-xs text-muted-foreground">
            Basis-URL (mit oder ohne <code>/v1</code> — je nachdem, was bei dir funktioniert)
          </span>
          <Input
            value={urlInput}
            onChange={(e) => setUrlInput(e.target.value)}
            placeholder="http://localhost:20128/v1"
            className="bg-background"
          />
        </label>
        <label className="block">
          <span className="mb-1 block text-xs text-muted-foreground">
            {apiKeySet ? "API-Key (gesetzt — leer lassen, um ihn zu behalten)" : "API-Key"}
          </span>
          <Input
            value={keyInput}
            onChange={(e) => setKeyInput(e.target.value)}
            type="password"
            placeholder={apiKeySet ? "•••••••• (unverändert lassen)" : "sk-…"}
            className="bg-background"
          />
        </label>
        <button
          type="button"
          disabled={saving}
          onClick={handleSave}
          className="rounded-lg bg-secondary px-3 py-1.5 text-xs font-medium text-secondary-foreground transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-40"
        >
          Speichern
        </button>
      </div>
    </section>
  )
}

function ProjectsSection({
  omni,
  pending,
  onRefresh,
  onAddProject,
  onRemoveProject,
  onToggleProject,
}: {
  omni: OmniState
  pending: Set<string>
  onRefresh: () => void
  onAddProject: (path: string) => Promise<void>
  onRemoveProject: (path: string) => Promise<void>
  onToggleProject: (path: string, active: boolean) => Promise<void>
}) {
  return (
    <section>
      <div className="mb-2 flex items-center justify-between px-1">
        <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
          Projekte
        </h2>
        <button
          type="button"
          onClick={onRefresh}
          className="flex size-6 items-center justify-center rounded-md text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground"
          aria-label="Liste neu laden"
        >
          <RefreshCw className="size-3.5" />
        </button>
      </div>

      <div className="overflow-hidden rounded-[10px] border border-border bg-card">
        {omni.projects.length === 0 ? (
          <p className="px-4 py-3 text-sm text-muted-foreground">
            Noch kein Projekt hinzugefügt.
          </p>
        ) : (
          omni.projects.map((p, i) => (
            <div key={p.path}>
              {i > 0 && <div className="ml-4 h-px bg-border" />}
              <ProjectRowView
                project={p}
                disabled={pending.has(p.path)}
                onRemove={() => onRemoveProject(p.path)}
                onToggle={(active) => onToggleProject(p.path, active)}
                onRefresh={onRefresh}
              />
            </div>
          ))
        )}
        <div className="border-t border-border">
          <AddProjectRow onAdd={onAddProject} />
        </div>
      </div>
    </section>
  )
}

function ProjectRowView({
  project,
  disabled,
  onRemove,
  onToggle,
  onRefresh,
}: {
  project: ProjectRoute
  disabled: boolean
  onRemove: () => void
  onToggle: (active: boolean) => void
  onRefresh: () => void
}) {
  const [confirmRemove, setConfirmRemove] = useState(false)
  const [openingTerminal, setOpeningTerminal] = useState(false)
  const [fixingGitignore, setFixingGitignore] = useState(false)

  async function handleOpenTerminal() {
    setOpeningTerminal(true)
    try {
      await openOmnirouteTerminal(project.path)
      toast.success("Terminal geöffnet — Routing gilt nur für diese eine Session.")
    } catch (e) {
      toast.error(String(e))
    } finally {
      setOpeningTerminal(false)
    }
  }

  async function handleFixGitignore() {
    setFixingGitignore(true)
    try {
      await addGitignoreEntry(project.path)
      toast.success(".gitignore ergänzt.")
      onRefresh()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setFixingGitignore(false)
    }
  }

  return (
    <div
      className={cn(
        "flex flex-col gap-1.5 px-4 py-2.5",
        project.active && "border-l-2 border-l-warning bg-warning/5"
      )}
    >
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium" title={project.path}>
            {project.path}
          </p>
          {!project.exists && (
            <p className="text-xs text-destructive">Ordner nicht gefunden.</p>
          )}
          {project.error && <p className="text-xs text-destructive">{project.error}</p>}
        </div>

        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              disabled={disabled || openingTerminal}
              onClick={handleOpenTerminal}
              className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-30"
            >
              <Terminal className="size-4" />
            </button>
          </TooltipTrigger>
          <TooltipContent>
            OmniRoute-Terminal öffnen (einmalig, schreibt nichts auf die Platte)
          </TooltipContent>
        </Tooltip>

        {confirmRemove ? (
          <div className="flex shrink-0 items-center gap-1 text-xs">
            <span className="text-muted-foreground">Entfernen?</span>
            <button
              type="button"
              onClick={onRemove}
              className="rounded-md bg-destructive/10 px-2 py-1 font-medium text-destructive hover:bg-destructive/20"
            >
              Ja
            </button>
            <button
              type="button"
              onClick={() => setConfirmRemove(false)}
              className="rounded-md px-2 py-1 text-muted-foreground hover:bg-accent"
            >
              Abbrechen
            </button>
          </div>
        ) : (
          <Tooltip>
            <TooltipTrigger asChild>
              <button
                type="button"
                disabled={disabled}
                onClick={() => setConfirmRemove(true)}
                className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-30"
              >
                <Trash2 className="size-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent>Aus der Liste entfernen</TooltipContent>
          </Tooltip>
        )}

        <label className="inline-flex shrink-0 cursor-pointer items-center gap-2">
          <span className="text-xs text-muted-foreground">über OmniRoute</span>
          <span
            role="switch"
            aria-checked={project.active}
            aria-disabled={disabled}
            onClick={() => !disabled && onToggle(!project.active)}
            className={cn(
              "relative inline-flex h-[18px] w-8 shrink-0 items-center rounded-full transition-colors",
              project.active ? "bg-primary" : "bg-input",
              disabled && "pointer-events-none opacity-50"
            )}
          >
            <span
              className={cn(
                "block size-3.5 translate-x-0.5 rounded-full bg-background transition-transform",
                project.active && "translate-x-[15px]"
              )}
            />
          </span>
        </label>
      </div>

      <p className="text-[11px] text-muted-foreground">
        {project.active
          ? `Aktiv seit dem letzten Umschalten — schreibt in .claude\\settings.local.json (Fallback für manuell im Terminal gestartete Sessions).`
          : "Aus — .claude\\settings.local.json hat kein OmniRoute-Routing."}
      </p>

      {project.active && !project.gitignoreOk && (
        <div className="flex items-center gap-2 rounded-md bg-warning/10 px-2 py-1 text-[11px] text-warning">
          <AlertTriangle className="size-3 shrink-0" />
          <span className="flex-1">
            settings.local.json ist nicht in der .gitignore erfasst — der Key liegt im
            Klartext im Repo.
          </span>
          <button
            type="button"
            disabled={fixingGitignore}
            onClick={handleFixGitignore}
            className="shrink-0 rounded-md bg-warning/20 px-1.5 py-0.5 font-medium hover:bg-warning/30 disabled:opacity-50"
          >
            .gitignore ergänzen
          </button>
        </div>
      )}
    </div>
  )
}

function AddProjectRow({ onAdd }: { onAdd: (path: string) => Promise<void> }) {
  const [value, setValue] = useState("")
  const [adding, setAdding] = useState(false)

  async function handleAdd() {
    if (!value.trim()) return
    setAdding(true)
    try {
      await onAdd(value.trim())
      setValue("")
    } catch (e) {
      toast.error(String(e))
    } finally {
      setAdding(false)
    }
  }

  return (
    <div className="flex items-center gap-2 px-4 py-2.5">
      <Folder className="size-4 shrink-0 text-muted-foreground" />
      <Input
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && handleAdd()}
        placeholder="C:\Pfad\zum\Wegwerf-Projekt"
        className="h-7 flex-1 border-none bg-transparent px-0 shadow-none focus-visible:ring-0"
      />
      <button
        type="button"
        disabled={adding || !value.trim()}
        onClick={handleAdd}
        className="shrink-0 rounded-md bg-secondary px-2.5 py-1 text-xs font-medium text-secondary-foreground transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-40"
      >
        Hinzufügen
      </button>
    </div>
  )
}

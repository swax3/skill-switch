import { useEffect, useState } from "react"
import { AlertTriangle, Bell, Check, Copy, Info } from "lucide-react"
import { toast } from "sonner"
import {
  contextHookStatus,
  installContextHook,
  uninstallContextHook,
  type Advice,
  type HookStatus,
  type LiveSession,
} from "@/lib/api"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

const fmtK = (n: number) => (n >= 1_000_000 ? `${(n / 1e6).toFixed(2)} M` : `${Math.round(n / 1000)}k`)
const fmtAgo = (s: number) =>
  s < 60 ? `vor ${s} s` : s < 3600 ? `vor ${Math.round(s / 60)} min` : `vor ${Math.round(s / 3600)} h`
const fmtAge = (s: number) =>
  s >= 2 * 86_400 ? `${Math.round(s / 86_400)} Tage` : s >= 3600 ? `${Math.round(s / 3600)} h` : `${Math.max(1, Math.round(s / 60))} min`

const LEVEL_TEXT: Record<Advice["level"], string> = {
  high: "text-destructive",
  medium: "text-warning",
  ok: "text-primary",
  info: "text-muted-foreground",
}
const LEVEL_BG: Record<Advice["level"], string> = {
  high: "bg-destructive/10",
  medium: "bg-warning/10",
  ok: "bg-primary/10",
  info: "bg-muted/60",
}

type LivePanelProps = {
  sessions: LiveSession[] | null
  error: string | null
  updatedAt: number | null
  notifyEnabled: boolean
  notifyBlocked: boolean
  onNotifyEnabledChange: (enabled: boolean) => void
}

export function LivePanel({
  sessions,
  error,
  updatedAt,
  notifyEnabled,
  notifyBlocked,
  onNotifyEnabledChange,
}: LivePanelProps) {
  const activeCount = sessions?.filter((s) => s.active).length ?? 0

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-6">
      <section className="flex items-center justify-between gap-4 rounded-[10px] border border-border bg-card px-4 py-3">
        <div className="min-w-0">
          <p className="text-sm font-medium">
            {sessions ? `${activeCount} aktiv · ${sessions.length} in den letzten 6 h` : "Sessions"}
          </p>
          <p className="text-xs text-muted-foreground">
            Liest nur das Ende der Transcripts alle 5 s. Ausführen kann eine Empfehlung nur Claude Code
            selbst — Befehl kopieren und in den Chat einfügen.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-3">
          {updatedAt && (
            <span className="text-[11px] text-muted-foreground">
              aktualisiert {new Date(updatedAt).toLocaleTimeString("de-DE")}
            </span>
          )}
          <NotifyToggle enabled={notifyEnabled} blocked={notifyBlocked} onChange={onNotifyEnabledChange} />
        </div>
      </section>

      <ContextHookCard />

      {error && (
        <p className="flex items-start gap-2 rounded-[10px] border border-destructive/40 bg-destructive/10 px-4 py-3 text-xs text-destructive">
          <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
          {error}
        </p>
      )}

      {sessions && sessions.length === 0 && (
        <p className="rounded-[10px] border border-border bg-card px-4 py-3 text-sm text-muted-foreground">
          Keine Session hat in den letzten 6 Stunden geschrieben.
        </p>
      )}

      {sessions?.map((s) => (
        <SessionCard key={s.id} s={s} />
      ))}
    </div>
  )
}

function ContextHookCard() {
  const [status, setStatus] = useState<HookStatus | null>(null)
  const [busy, setBusy] = useState(false)

  const refresh = () => contextHookStatus().then(setStatus).catch((e) => toast.error(String(e)))
  useEffect(() => {
    refresh()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  async function handleToggle() {
    if (!status) return
    setBusy(true)
    try {
      if (status.installed) {
        await uninstallContextHook()
        toast.success("Kontext-Hook entfernt.")
      } else {
        await installContextHook()
        toast.success("Kontext-Hook installiert.")
      }
      await refresh()
    } catch (e) {
      toast.error(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="rounded-[10px] border border-border bg-card p-4">
      <h2 className="mb-1 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        Hinweis direkt im Chat
      </h2>
      <p className="text-xs text-muted-foreground">
        Zeigt in jeder Claude-Code-Session — Desktop und Terminal — vor deinem Prompt
        eine Zeile, sobald der Kontext 850k überschreitet. Blockiert nichts, landet
        nicht im Modell-Kontext.
      </p>

      {status && (
        <>
          <p className="mt-2.5 text-xs font-medium">
            {status.installed ? "installiert" : "nicht installiert"}
          </p>
          <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
            {status.settingsPath}
          </p>
          <p className="break-all font-mono text-[11px] text-muted-foreground">{status.scriptPath}</p>

          <button
            type="button"
            disabled={busy}
            onClick={handleToggle}
            className={cn(
              "mt-3 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors disabled:pointer-events-none disabled:opacity-40",
              status.installed
                ? "bg-secondary text-secondary-foreground hover:bg-accent"
                : "bg-primary text-primary-foreground hover:bg-primary/80"
            )}
          >
            {status.installed ? "Entfernen" : "Installieren"}
          </button>

          <p className="mt-2 text-[11px] text-muted-foreground">
            Wird sofort wirksam — Claude Code liest Hook-Änderungen live.
          </p>
        </>
      )}
    </section>
  )
}

function NotifyToggle({
  enabled,
  blocked,
  onChange,
}: {
  enabled: boolean
  blocked: boolean
  onChange: (enabled: boolean) => void
}) {
  return (
    <div className="flex flex-col items-end gap-1">
      <label className="inline-flex cursor-pointer items-center gap-2">
        <Bell className="size-3.5 text-muted-foreground" />
        <span className="text-xs text-muted-foreground">Benachrichtigungen</span>
        <span
          role="switch"
          aria-checked={enabled}
          onClick={() => onChange(!enabled)}
          className={cn(
            "relative inline-flex h-[18px] w-8 shrink-0 items-center rounded-full transition-colors",
            enabled ? "bg-primary" : "bg-input"
          )}
        >
          <span
            className={cn(
              "block size-3.5 translate-x-0.5 rounded-full bg-background transition-transform",
              enabled && "translate-x-[15px]"
            )}
          />
        </span>
      </label>
      {blocked && (
        <p className="max-w-56 text-right text-[11px] text-muted-foreground">
          Vom System blockiert — in den Windows-Einstellungen für Skill Switch erlauben.
        </p>
      )}
    </div>
  )
}

function SessionCard({ s }: { s: LiveSession }) {
  const ctxLevel = s.advice[0]?.level ?? "ok"
  return (
    <section className={cn("rounded-[10px] border bg-card", s.active ? "border-primary/40" : "border-border")}>
      <div className="flex items-center gap-2.5 px-4 pt-3">
        <span className={cn("size-2 shrink-0 rounded-full", s.active ? "bg-primary" : "bg-muted-foreground/40")} />
        <p className="min-w-0 flex-1 truncate text-sm font-medium" title={s.cwd ?? s.id}>
          {s.title ?? s.project}
          {s.title && <span className="ml-1.5 font-normal text-muted-foreground">· {s.project}</span>}
        </p>
        <Badge>{s.entrypoint}</Badge>
        {s.model && (
          <Badge>
            {s.model.replace(/^claude-/, "")}
            {s.effort && ` · ${s.effort}`}
          </Badge>
        )}
        <span className="shrink-0 text-[11px] text-muted-foreground">{fmtAgo(s.secondsSinceActivity)}</span>
      </div>

      <div className="grid grid-cols-3 gap-x-4 gap-y-2 px-4 py-3 md:grid-cols-6">
        <Stat label="Kontext" value={fmtK(s.context)} className={LEVEL_TEXT[ctxLevel]} hint="Was der letzte Request gelesen hat: Input + Cache-Read + Cache-Write." />
        <Stat label="Turns" value={String(s.turns)} hint="User-Nachrichten in dieser Session." />
        <Stat label="Requests" value={`${s.turnRequests} / ${s.requests}`} hint="Aktueller Turn / gesamt. Jeder Request liest den ganzen Kontext." />
        <Stat label="Fehler im Turn" value={String(s.turnErrors)} className={s.turnErrors >= 3 ? "text-warning" : undefined} hint="Fehlgeschlagene Tool-Aufrufe seit deiner letzten Nachricht." />
        <Stat label="Alter" value={fmtAge(s.ageSecs)} hint="Erste bis letzte Nachricht dieser Session." />
        <Stat label="Compacts · Agents" value={`${s.compactions} · ${s.agentsActive}`} hint="Bisherige Kompaktierungen · Subagents, die in den letzten 10 min geschrieben haben." />
      </div>

      <div className="space-y-1 border-t border-border px-3 pb-3 pt-2">
        {s.advice.map((a) => (
          <AdviceRow key={a.text} a={a} />
        ))}
      </div>
    </section>
  )
}

function AdviceRow({ a }: { a: Advice }) {
  const [copied, setCopied] = useState(false)
  async function copy() {
    if (!a.command) return
    try {
      await navigator.clipboard.writeText(a.command)
      setCopied(true)
      toast.success(a.command.endsWith(" ") ? "Kopiert — Thema ergänzen und in den Chat einfügen." : "Kopiert — in den Chat einfügen.")
      setTimeout(() => setCopied(false), 1500)
    } catch (e) {
      toast.error(String(e))
    }
  }
  return (
    <div className={cn("flex items-center gap-2 rounded-md px-2 py-1.5 text-xs", LEVEL_BG[a.level])}>
      {a.level === "high" || a.level === "medium" ? (
        <AlertTriangle className={cn("size-3.5 shrink-0", LEVEL_TEXT[a.level])} />
      ) : (
        <Info className={cn("size-3.5 shrink-0", LEVEL_TEXT[a.level])} />
      )}
      <span className="flex-1">{a.text}</span>
      {a.command && (
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              onClick={copy}
              className="inline-flex shrink-0 items-center gap-1 rounded-md bg-background/70 px-2 py-0.5 font-mono text-[11px] transition-colors hover:bg-accent"
            >
              {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
              {a.command.trim()}
            </button>
          </TooltipTrigger>
          <TooltipContent>Befehl kopieren</TooltipContent>
        </Tooltip>
      )}
    </div>
  )
}

function Stat({ label, value, hint, className }: { label: string; value: string; hint: string; className?: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div>
          <p className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</p>
          <p className={cn("text-sm font-semibold tabular-nums", className)}>{value}</p>
        </div>
      </TooltipTrigger>
      <TooltipContent className="max-w-60">{hint}</TooltipContent>
    </Tooltip>
  )
}

function Badge({ children }: { children: React.ReactNode }) {
  return (
    <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-[10px] font-medium text-muted-foreground">{children}</span>
  )
}

import { Fragment } from "react"
import { AlertTriangle, ArrowDown, ArrowRight, ArrowUp, Info, RefreshCw } from "lucide-react"
import type { DayRow, Finding, Named, Trend as TrendData, UsageReport } from "@/lib/api"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

type UsagePanelProps = {
  report: UsageReport | null
  loading: boolean
  error: string | null
  onRefresh: () => void
}

const fmt = (n: number) =>
  n >= 1e9 ? `${(n / 1e9).toFixed(2)} Mrd.` : n >= 1e6 ? `${(n / 1e6).toFixed(1)} M` : n >= 1e3 ? `${Math.round(n / 1e3)}k` : String(n)

const fmtDuration = (min: number) =>
  min >= 2880 ? `${Math.round(min / 1440)} T` : min >= 120 ? `${Math.round(min / 60)} h` : `${min} m`

export function UsagePanel({ report, loading, error, onRefresh }: UsagePanelProps) {
  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-6">
      <section className="flex items-center justify-between rounded-[10px] border border-border bg-card px-4 py-3">
        <div className="min-w-0">
          <p className="text-sm font-medium">
            {report
              ? `${report.files} Transcripts · ${fmt(report.total.requests)} Requests · ${report.firstDay ?? "?"} bis ${report.lastDay ?? "?"}`
              : "Claude-Code-Nutzung"}
          </p>
          <p className="text-xs text-muted-foreground">
            Liest nur Metadaten aus ~/.claude/projects (Token-Zahlen, Modell, Tool-Namen). Kein Prompt-Text, nichts
            wird gespeichert oder gesendet.
          </p>
        </div>
        <button
          type="button"
          disabled={loading}
          onClick={onRefresh}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-lg bg-secondary px-2.5 py-1.5 text-xs font-medium text-secondary-foreground transition-colors hover:bg-accent disabled:pointer-events-none disabled:opacity-40"
        >
          <RefreshCw className={cn("size-3.5", loading && "animate-spin")} />
          {loading ? "Analysiere…" : "Neu analysieren"}
        </button>
      </section>

      {error && (
        <p className="flex items-start gap-2 rounded-[10px] border border-destructive/40 bg-destructive/10 px-4 py-3 text-xs text-destructive">
          <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
          {error}
        </p>
      )}

      {loading && !report && (
        <p className="text-sm text-muted-foreground">Lese Transcripts — bei mehreren hundert MB dauert das ein paar Sekunden…</p>
      )}

      {report && (
        <>
          <Findings findings={report.findings} />
          <Trend trend={report.trend} days={report.days} />
          <KeyNumbers r={report} />
          <ContextBuckets r={report} />
          <div className="grid grid-cols-1 gap-4 xl:grid-cols-3">
            <Breakdown title="Nach Herkunft" rows={report.byKind} />
            <Breakdown title="Nach Modell" rows={report.byModel} />
            <Breakdown title="Nach Effort" rows={report.byEffort} />
          </div>
          <Sessions r={report} />
          <Tools r={report} />
        </>
      )}
    </div>
  )
}

const SEVERITY_STYLE: Record<Finding["severity"], string> = {
  high: "border-destructive/40 bg-destructive/10 text-destructive",
  medium: "border-warning/40 bg-warning/10 text-warning",
  low: "border-border bg-card text-foreground",
  info: "border-border bg-card text-muted-foreground",
}
const SEVERITY_LABEL: Record<Finding["severity"], string> = {
  high: "Hoch",
  medium: "Mittel",
  low: "Niedrig",
  info: "Info",
}

function Findings({ findings }: { findings: Finding[] }) {
  return (
    <section>
      <SectionTitle>Befunde</SectionTitle>
      <div className="space-y-2">
        {findings.map((f) => (
          <div key={f.title} className={cn("rounded-[10px] border px-4 py-3", SEVERITY_STYLE[f.severity])}>
            <div className="flex items-center gap-2">
              {f.severity === "high" || f.severity === "medium" ? (
                <AlertTriangle className="size-3.5 shrink-0" />
              ) : (
                <Info className="size-3.5 shrink-0" />
              )}
              <p className="flex-1 text-sm font-medium">{f.title}</p>
              <span className="rounded-full bg-background/60 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide">
                {SEVERITY_LABEL[f.severity]}
              </span>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="rounded-full bg-background/60 px-2 py-0.5 text-[10px] font-medium">{f.basis}</span>
                </TooltipTrigger>
                <TooltipContent className="max-w-60">
                  gemessen = direkt aus den Transcripts · berechnet = aus gemessenen Zahlen abgeleitet, Gewichtung
                  ist eine Heuristik · heuristik = Schwellenwert-Regel
                </TooltipContent>
              </Tooltip>
            </div>
            <p className="mt-1.5 text-xs opacity-90">{f.evidence}</p>
            <p className="mt-1 text-xs font-medium text-foreground">→ {f.recommendation}</p>
          </div>
        ))}
      </div>
    </section>
  )
}

// Lower is better for every one of these metrics (context, requests, effort, workflows,
// limit hits) — there is no metric here where "more" is the goal.
function delta(cur: number, prev: number): { text: string; className: string; Arrow?: typeof ArrowUp } {
  if (prev === 0) {
    if (cur === 0) return { text: "± 0 %", className: "text-muted-foreground" }
    return { text: "neu", className: "text-destructive" }
  }
  const pct = ((cur - prev) / prev) * 100
  const className = pct <= -5 ? "text-green-600 dark:text-green-400" : pct >= 5 ? "text-destructive" : "text-muted-foreground"
  const Arrow = pct < -0.5 ? ArrowDown : pct > 0.5 ? ArrowUp : ArrowRight
  return { text: `${Math.round(Math.abs(pct))} %`, className, Arrow }
}

const germanDate = (iso: string) => {
  const [y, m, d] = iso.split("-")
  return `${d}.${m}.${y}`
}

function Trend({ trend, days }: { trend: TrendData; days: DayRow[] }) {
  const curEmpty = trend.current.requests === 0
  const prevEmpty = trend.previous.requests === 0
  const rows: { label: string; cur: number; prev: number; format: (n: number) => string }[] = [
    { label: "Ø Kontext / Request", cur: trend.current.avgContext, prev: trend.previous.avgContext, format: fmt },
    { label: "Requests / Turn", cur: trend.current.requestsPerTurn, prev: trend.previous.requestsPerTurn, format: (n) => n.toFixed(1) },
    { label: "Effort xhigh", cur: trend.current.xhighPct, prev: trend.previous.xhighPct, format: (n) => `${n} %` },
    { label: "Workflow-Anteil", cur: trend.current.workflowPct, prev: trend.previous.workflowPct, format: (n) => `${n} %` },
    { label: "5h-Limit-Treffer", cur: trend.current.fiveHourHits, prev: trend.previous.fiveHourHits, format: String },
    { label: "Requests", cur: trend.current.requests, prev: trend.previous.requests, format: fmt },
  ]
  return (
    <section>
      <SectionTitle>Trend</SectionTitle>
      <div className="rounded-[10px] border border-border bg-card px-4 py-3">
        <div className="grid grid-cols-[1fr_auto_auto_auto] items-center gap-x-4 gap-y-1.5 text-xs">
          <span />
          <span className="text-right font-medium text-muted-foreground">Letzte 7 Tage</span>
          <span className="text-right font-medium text-muted-foreground">7 Tage davor</span>
          <span />
          {rows.map((row) => {
            const d = !curEmpty && !prevEmpty ? delta(row.cur, row.prev) : null
            return (
              <Fragment key={row.label}>
                <span className="text-muted-foreground">{row.label}</span>
                <span className="text-right tabular-nums font-medium">{curEmpty ? "–" : row.format(row.cur)}</span>
                <span className="text-right tabular-nums text-muted-foreground">{prevEmpty ? "–" : row.format(row.prev)}</span>
                <span className={cn("flex items-center justify-end gap-0.5 tabular-nums", d?.className ?? "text-muted-foreground")}>
                  {d ? (
                    <>
                      {d.Arrow && <d.Arrow className="size-3" />}
                      {d.text}
                    </>
                  ) : (
                    "–"
                  )}
                </span>
              </Fragment>
            )
          })}
        </div>

        <div className="mt-3 space-y-0.5 border-t border-border pt-2">
          {days.map((d) => (
            <div key={d.day} className="flex items-center gap-3 text-[11px] text-muted-foreground">
              <span className="w-20 shrink-0">{germanDate(d.day)}</span>
              <span className="w-16 shrink-0 text-right tabular-nums">{fmt(d.requests)} Req.</span>
              <span className="w-20 shrink-0 text-right tabular-nums">Ø {fmt(d.avgContext)}</span>
            </div>
          ))}
        </div>

        <p className="mt-2 text-[11px] text-muted-foreground">
          Vergleich derselben Heuristik-Gewichtung; Tage ohne Nutzung zählen nicht als Verbesserung.
        </p>
      </div>
    </section>
  )
}

function KeyNumbers({ r }: { r: UsageReport }) {
  const tiles: { label: string; value: string; hint: string }[] = [
    { label: "Ø Kontext / Request", value: fmt(r.avgContext), hint: "Input + Cache-Write + Cache-Read pro API-Request. Jeder Request schickt den ganzen Kontext erneut." },
    { label: "Kontextvolumen aus >200k", value: `${r.largeContextVolumePct} %`, hint: "Anteil aller gelesenen Kontext-Tokens, die aus Requests mit mehr als 200k Kontext stammen." },
    { label: "Effort xhigh", value: `${r.xhighSharePct} %`, hint: "Anteil der gewichteten Nutzung mit effort xhigh (Heuristik-Gewichtung)." },
    { label: "Workflow-Agents", value: `${r.workflowSharePct} %`, hint: `${r.workflowAgents} Workflow-Agent-Transcripts; Anteil an der gewichteten Nutzung.` },
    { label: "Ø Requests / Turn", value: String(r.avgRequestsPerTurn), hint: "API-Requests pro User-Nachricht in Hauptsessions. Tool-Aufrufe und Retries zählen mit." },
    { label: "5h-Limit erreicht", value: String(r.fiveHourHits), hint: "Gespeicherte quotaLimits-Marker mit rateLimitType five_hour und status rejected." },
    { label: "Sessions > 150k", value: `${r.sessionsOver150k} / ${r.mainSessions}`, hint: `${r.sessionsCompacted} Sessions haben je kompaktiert (${r.compactions} Compact-Events).` },
    { label: "Fehler", value: `${r.toolErrors} / ${r.apiErrors}`, hint: "Fehlgeschlagene Tool-Aufrufe / API-Fehlermeldungen." },
  ]
  return (
    <section>
      <SectionTitle>Kennzahlen</SectionTitle>
      <div className="grid grid-cols-2 gap-2 md:grid-cols-4">
        {tiles.map((t) => (
          <Tooltip key={t.label}>
            <TooltipTrigger asChild>
              <div className="rounded-[10px] border border-border bg-card px-3 py-2.5">
                <p className="text-[11px] text-muted-foreground">{t.label}</p>
                <p className="text-lg font-semibold tabular-nums">{t.value}</p>
              </div>
            </TooltipTrigger>
            <TooltipContent className="max-w-60">{t.hint}</TooltipContent>
          </Tooltip>
        ))}
      </div>
    </section>
  )
}

function ContextBuckets({ r }: { r: UsageReport }) {
  return (
    <section>
      <SectionTitle>Kontextgröße pro Request</SectionTitle>
      <div className="rounded-[10px] border border-border bg-card px-4 py-3">
        {r.contextBuckets.map((b) => (
          <div key={b.label} className="flex items-center gap-3 py-1 text-xs">
            <span className="w-16 shrink-0 text-muted-foreground">{b.label}</span>
            <div className="h-2 flex-1 overflow-hidden rounded-full bg-muted">
              <div
                className={cn("h-full rounded-full", b.label === ">200k" ? "bg-destructive" : "bg-primary")}
                style={{ width: `${b.volumePct}%` }}
              />
            </div>
            <span className="w-28 shrink-0 text-right tabular-nums">
              {b.volumePct} % Volumen · {b.requestsPct} % Req.
            </span>
          </div>
        ))}
        <p className="mt-2 text-[11px] text-muted-foreground">
          Balken = Anteil am gelesenen Kontextvolumen. Cache-Read-Anteil insgesamt: {r.cacheReadPct} % — günstig pro
          Token, aber jeder Request liest ihn komplett neu; läuft der Cache ab, wird alles als Cache-Write neu
          geschrieben ({fmt(r.total.cacheWrite)} Tokens bisher).
        </p>
      </div>
    </section>
  )
}

function Breakdown({ title, rows }: { title: string; rows: Named[] }) {
  return (
    <section>
      <SectionTitle>{title}</SectionTitle>
      <div className="overflow-hidden rounded-[10px] border border-border bg-card">
        {rows.map((n, i) => (
          <div key={n.name}>
            {i > 0 && <div className="ml-4 h-px bg-border" />}
            <div className="flex items-center gap-2 px-4 py-2 text-xs">
              <span className="min-w-0 flex-1 truncate font-medium" title={n.name}>
                {n.name.replace(/^claude-/, "")}
              </span>
              <span className="w-12 text-right tabular-nums text-muted-foreground">{fmt(n.tokens.requests)}</span>
              <span className="w-12 text-right tabular-nums font-medium">{n.sharePct} %</span>
            </div>
          </div>
        ))}
        <p className="px-4 pb-2 pt-1 text-[10px] text-muted-foreground">Requests · Anteil (gewichtet, Heuristik)</p>
      </div>
    </section>
  )
}

function Sessions({ r }: { r: UsageReport }) {
  const th = "px-2 py-1.5 text-left text-[10px] font-semibold uppercase tracking-wide text-muted-foreground"
  const td = "px-2 py-1.5 tabular-nums"
  return (
    <section>
      <SectionTitle>Teuerste Sessions</SectionTitle>
      <div className="overflow-x-auto rounded-[10px] border border-border bg-card">
        <table className="w-full text-xs">
          <thead>
            <tr className="border-b border-border">
              <th className={th}>Projekt</th>
              <th className={th}>Start</th>
              <th className={th}>Dauer</th>
              <th className={th}>Turns</th>
              <th className={th}>Req</th>
              <th className={th}>Req/Turn</th>
              <th className={th}>Max Kontext</th>
              <th className={th}>Agents</th>
              <th className={th}>Compact</th>
              <th className={th}>Fehler</th>
              <th className={th}>Anteil</th>
            </tr>
          </thead>
          <tbody>
            {r.sessions.map((s) => (
              <tr key={s.id} className="border-b border-border/60 last:border-0" title={`${s.id} · ${s.models}`}>
                <td className={cn(td, "font-medium")}>{s.project}</td>
                <td className={td}>{s.startDay}</td>
                <td className={cn(td, s.durationMin >= 4320 && "text-destructive")}>{fmtDuration(s.durationMin)}</td>
                <td className={td}>{s.turns}</td>
                <td className={td}>{s.requests}</td>
                <td className={td}>{s.requestsPerTurn}</td>
                <td className={cn(td, s.maxContext > 200_000 && "text-destructive")}>{fmt(s.maxContext)}</td>
                <td className={td}>
                  {s.subagents + s.workflowAgents}
                  {s.workflowAgents > 0 && <span className="text-muted-foreground"> ({s.workflowAgents} Wf)</span>}
                </td>
                <td className={td}>{s.compactions}</td>
                <td className={td}>{s.errors}</td>
                <td className={cn(td, "font-medium")}>{s.sharePct} %</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  )
}

function Tools({ r }: { r: UsageReport }) {
  const list = (rows: UsageReport["topToolsMain"]) =>
    rows.map((t) => `${t.name.replace(/^mcp__/, "")} ${t.count}`).join(" · ")
  return (
    <section>
      <SectionTitle>Tools</SectionTitle>
      <div className="space-y-1 rounded-[10px] border border-border bg-card px-4 py-3 text-xs">
        <p>
          <span className="text-muted-foreground">Hauptsession: </span>
          {list(r.topToolsMain)}
        </p>
        <p>
          <span className="text-muted-foreground">Agents: </span>
          {list(r.topToolsAgents)}
        </p>
      </div>
    </section>
  )
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h2 className="mb-2 px-1 text-xs font-semibold tracking-wide text-muted-foreground uppercase">{children}</h2>
  )
}

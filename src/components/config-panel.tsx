import { Activity, Plug, Puzzle } from "lucide-react"
import type { AttributionUsage, EnvInfo } from "@/lib/api"

type ConfigPanelProps = {
  env: EnvInfo | null
  loading: boolean
  /** From the Usage tab's transcript analysis — null until loaded once. */
  mcpUsage: AttributionUsage[] | null
  usageLoading: boolean
  onLoadUsage: () => void
}

const germanDate = (iso: string) => {
  const [y, m, d] = iso.split("-")
  return `${d}.${m}.${y}`
}

const usageDetail = (u: AttributionUsage) => `${u.messages} Nachrichten · zuletzt ${u.lastDay ? germanDate(u.lastDay) : "?"}`

export function ConfigPanel({ env, loading, mcpUsage, usageLoading, onLoadUsage }: ConfigPanelProps) {
  return (
    <div className="flex h-full flex-col gap-6 overflow-y-auto p-6">
      <div className="flex items-start justify-between gap-3 rounded-[10px] border border-border bg-card px-4 py-3">
        <p className="text-xs text-muted-foreground">
          Plugins/MCPs schaltest du in Claude Code selbst (/mcp bzw. Plugin-Manager). Plugin-eigene
          Skills erscheinen hier nicht – sie schalten mit ihrem Plugin.
        </p>
        {!mcpUsage && (
          <button
            type="button"
            disabled={usageLoading}
            onClick={onLoadUsage}
            className="shrink-0 rounded-full px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
          >
            {usageLoading ? "lädt…" : "Nutzungsdaten laden"}
          </button>
        )}
      </div>

      {loading ? (
        <p className="text-sm text-muted-foreground">Wird geladen…</p>
      ) : (
        <>
          <Group
            title="MCP-Server"
            icon={Plug}
            empty="Keine MCP-Server konfiguriert."
            items={(env?.mcpServers ?? []).map((s) => {
              const usage = mcpUsage?.find((u) => u.name === s.name)
              return {
                key: `${s.scope}:${s.name}`,
                name: s.name,
                detail: usage ? usageDetail(usage) : s.scope,
              }
            })}
          />
          <Group
            title="Plugins"
            icon={Puzzle}
            empty="Keine Plugins installiert."
            items={(env?.plugins ?? []).map((p) => ({
              key: p.id,
              name: p.id,
              detail: p.version ? `Version ${p.version}` : undefined,
              badge: p.enabled ? "Aktiv" : "Inaktiv",
            }))}
          />
          {mcpUsage && (
            <Group
              title="MCP-Nutzung laut Transcripts"
              icon={Activity}
              empty="Keine Marker in den Transcripts gefunden."
              hint="Korrelation, keine Kosten: die Marker-Namen (z. B. Connector-UUIDs) stimmen oft nicht mit den Konfignamen oben überein."
              items={mcpUsage.map((u) => ({ key: u.name, name: u.name, detail: usageDetail(u) }))}
            />
          )}
        </>
      )}
    </div>
  )
}

type Item = { key: string; name: string; detail?: string; badge?: string }

function Group({
  title,
  icon: Icon,
  items,
  empty,
  hint,
}: {
  title: string
  icon: typeof Plug
  items: Item[]
  empty: string
  hint?: string
}) {
  return (
    <section>
      <h2 className="mb-2 flex items-center gap-1.5 px-1 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        <Icon className="size-3.5" />
        {title}
      </h2>
      {hint && <p className="mb-2 px-1 text-[11px] text-muted-foreground">{hint}</p>}
      {items.length === 0 ? (
        <p className="rounded-[10px] border border-border bg-card px-4 py-3 text-sm text-muted-foreground">
          {empty}
        </p>
      ) : (
        <div className="overflow-hidden rounded-[10px] border border-border bg-card">
          {items.map((item, i) => (
            <div key={item.key}>
              {i > 0 && <div className="ml-4 h-px bg-border" />}
              <div className="flex items-center gap-3 px-4 py-2.5">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium">{item.name}</p>
                  {item.detail && (
                    <p className="truncate text-xs text-muted-foreground">{item.detail}</p>
                  )}
                </div>
                {item.badge && (
                  <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                    {item.badge}
                  </span>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </section>
  )
}

import { Plug, Puzzle } from "lucide-react"
import type { EnvInfo } from "@/lib/api"

type ConfigPanelProps = {
  env: EnvInfo | null
  loading: boolean
}

export function ConfigPanel({ env, loading }: ConfigPanelProps) {
  return (
    <div className="flex h-full flex-col gap-6 overflow-y-auto p-6">
      <p className="rounded-[10px] border border-border bg-card px-4 py-3 text-xs text-muted-foreground">
        Plugins/MCPs schaltest du in Claude Code selbst (/mcp bzw. Plugin-Manager). Plugin-eigene
        Skills erscheinen hier nicht – sie schalten mit ihrem Plugin.
      </p>

      {loading ? (
        <p className="text-sm text-muted-foreground">Wird geladen…</p>
      ) : (
        <>
          <Group
            title="MCP-Server"
            icon={Plug}
            empty="Keine MCP-Server konfiguriert."
            items={(env?.mcpServers ?? []).map((s) => ({
              key: `${s.scope}:${s.name}`,
              name: s.name,
              detail: s.scope,
            }))}
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
}: {
  title: string
  icon: typeof Plug
  items: Item[]
  empty: string
}) {
  return (
    <section>
      <h2 className="mb-2 flex items-center gap-1.5 px-1 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        <Icon className="size-3.5" />
        {title}
      </h2>
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

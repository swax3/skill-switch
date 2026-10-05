import { useMemo, useState } from "react"
import { ClipboardCopy, ClipboardX, ExternalLink, Link2, RefreshCw, Search } from "lucide-react"
import { toast } from "sonner"
import { openExternal, type AttributionUsage, type Skill } from "@/lib/api"
import { Input } from "@/components/ui/input"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

export type SkillState = "active" | "manual" | "disabled"

export function skillState(skill: Skill): SkillState {
  if (!skill.enabled) return "disabled"
  if (skill.manualOnly) return "manual"
  return "active"
}

type SourceGroup = {
  key: string
  source: string | null
  sourceUrl: string | null
  skills: Skill[]
}

const UNKNOWN_GROUP_KEY = "￿-unknown" // sorts last

function buildGroups(skills: Skill[]): SourceGroup[] {
  const byKey = new Map<string, SourceGroup>()
  for (const skill of skills) {
    const key = skill.source ?? UNKNOWN_GROUP_KEY
    let group = byKey.get(key)
    if (!group) {
      group = { key, source: skill.source, sourceUrl: skill.sourceUrl, skills: [] }
      byKey.set(key, group)
    }
    group.skills.push(skill)
  }
  return [...byKey.values()].sort((a, b) => a.key.localeCompare(b.key))
}

type SkillListProps = {
  skills: Skill[]
  loading: boolean
  pending: Set<string>
  onStateChange: (skill: Skill, next: SkillState) => void
  onGroupStateChange: (skills: Skill[], next: SkillState) => void
  onRefresh: () => void
  /** From the Usage tab's transcript analysis — null until loaded once. */
  usage: AttributionUsage[] | null
  usageLoading: boolean
  onLoadUsage: () => void
}

const germanDate = (iso: string) => {
  const [y, m, d] = iso.split("-")
  return `${d}.${m}.${y}`
}

function activationPrompt(skill: Skill): string {
  return `Lies ${skill.skillMdPath} und wende diesen Skill ab jetzt an.`
}

function ignorePrompt(skill: Skill): string {
  return `Ignoriere ab sofort den Skill ${skill.name} vollständig.`
}

async function copy(text: string, successMessage: string) {
  try {
    await navigator.clipboard.writeText(text)
    toast.success(successMessage)
  } catch {
    toast.error("Konnte nicht in die Zwischenablage kopieren.")
  }
}

export function SkillList({
  skills,
  loading,
  pending,
  onStateChange,
  onGroupStateChange,
  onRefresh,
  usage,
  usageLoading,
  onLoadUsage,
}: SkillListProps) {
  const [query, setQuery] = useState("")

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return skills
    return skills.filter(
      (s) =>
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.source?.toLowerCase().includes(q)
    )
  }, [skills, query])

  const groups = useMemo(() => buildGroups(filtered), [filtered])

  return (
    <div className="flex h-full flex-col gap-4 p-6">
      <div className="flex shrink-0 items-center gap-2">
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Skills oder Quelle durchsuchen…"
            className="rounded-full bg-card pl-9 shadow-none"
          />
        </div>
        <IconAction
          icon={RefreshCw}
          label="Liste neu laden (z. B. nach Änderungen im Explorer)"
          disabled={loading}
          onClick={onRefresh}
        />
        {!usage && (
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
        <p className="text-sm text-muted-foreground">Skills werden geladen…</p>
      ) : groups.length === 0 ? (
        <p className="text-sm text-muted-foreground">Keine Skills gefunden.</p>
      ) : (
        <div className="flex-1 space-y-6 overflow-y-auto">
          {groups.map((group) => (
            <SkillGroupSection
              key={group.key}
              group={group}
              pending={pending}
              onStateChange={onStateChange}
              onGroupStateChange={onGroupStateChange}
              usage={usage}
            />
          ))}
        </div>
      )}
    </div>
  )
}

function SkillGroupSection({
  group,
  pending,
  onStateChange,
  onGroupStateChange,
  usage,
}: {
  group: SourceGroup
  pending: Set<string>
  onStateChange: (skill: Skill, next: SkillState) => void
  onGroupStateChange: (skills: Skill[], next: SkillState) => void
  usage: AttributionUsage[] | null
}) {
  const anyPending = group.skills.some((s) => pending.has(s.name))
  return (
    <section>
      <div className="mb-2 flex items-center justify-between gap-2 px-1">
        <div className="flex min-w-0 items-center gap-1.5">
          {group.source && group.sourceUrl ? (
            <button
              type="button"
              onClick={() => openExternal(group.sourceUrl!)}
              className="inline-flex items-center gap-1 truncate text-xs font-semibold tracking-wide text-muted-foreground uppercase hover:text-primary hover:underline"
            >
              {group.source}
              <ExternalLink className="size-3 shrink-0" />
            </button>
          ) : (
            <span className="truncate text-xs font-semibold tracking-wide text-muted-foreground uppercase">
              Ohne bekannte Quelle
            </span>
          )}
          <span className="shrink-0 text-xs text-muted-foreground">
            · {group.skills.length}
          </span>
        </div>
        {group.skills.length > 1 && (
          <StateSegment
            value={groupState(group.skills)}
            disabled={anyPending}
            onChange={(next) => onGroupStateChange(group.skills, next)}
            label={`Alle ${group.skills.length} Skills aus ${group.source ?? "dieser Gruppe"} umschalten`}
          />
        )}
      </div>
      <div className="overflow-hidden rounded-[10px] border border-border bg-card">
        {group.skills.map((skill, i) => (
          <div key={skill.name}>
            {i > 0 && <div className="ml-4 h-px bg-border" />}
            <SkillRow
              skill={skill}
              disabled={pending.has(skill.name)}
              onStateChange={onStateChange}
              usageEntry={usage?.find((u) => u.name === skill.name) ?? null}
              usageLoaded={usage !== null}
            />
          </div>
        ))}
      </div>
    </section>
  )
}

function groupState(skills: Skill[]): SkillState | "mixed" {
  const states = new Set(skills.map(skillState))
  return states.size === 1 ? [...states][0] : "mixed"
}

function SkillRow({
  skill,
  disabled,
  onStateChange,
  usageEntry,
  usageLoaded,
}: {
  skill: Skill
  disabled: boolean
  onStateChange: (skill: Skill, next: SkillState) => void
  usageEntry: AttributionUsage | null
  usageLoaded: boolean
}) {
  return (
    <div className="flex items-center gap-3 px-4 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-sm font-medium">{skill.name}</span>
          {skill.linked && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Link2 className="size-3 shrink-0 text-muted-foreground" />
              </TooltipTrigger>
              <TooltipContent>Verknüpfter Ordner (Symlink/Junction)</TooltipContent>
            </Tooltip>
          )}
        </div>
        <p className="truncate text-xs text-muted-foreground">{skill.description}</p>
        {usageLoaded && (
          <Tooltip>
            <TooltipTrigger asChild>
              <p className="truncate text-xs text-muted-foreground/70">
                {usageEntry
                  ? `In ${usageEntry.messages} Nachrichten aktiv · ${usageEntry.sessions} Sessions${usageEntry.lastDay ? ` · zuletzt ${germanDate(usageEntry.lastDay)}` : ""}`
                  : "Keine Nutzung in den Transcripts"}
              </p>
            </TooltipTrigger>
            <TooltipContent>Zählt Nachrichten, bei denen der Skill aktiv war — Korrelation, keine Kosten.</TooltipContent>
          </Tooltip>
        )}
      </div>

      <IconAction
        icon={ClipboardCopy}
        label="Aktivierungs-Prompt kopieren"
        disabled={!skill.skillMdPath}
        onClick={() =>
          skill.skillMdPath &&
          copy(activationPrompt(skill), "Aktivierungs-Prompt kopiert.")
        }
      />
      <IconAction
        icon={ClipboardX}
        label="Ignorieren-Prompt kopieren"
        onClick={() => copy(ignorePrompt(skill), "Ignorieren-Prompt kopiert.")}
      />

      <StateSegment
        value={skillState(skill)}
        disabled={disabled}
        onChange={(next) => onStateChange(skill, next)}
        label={`${skill.name}: Zustand ändern`}
      />
    </div>
  )
}

const STATE_OPTIONS: { value: SkillState; label: string; tooltip: string }[] = [
  { value: "active", label: "Aktiv", tooltip: "Claude sieht den Skill und nutzt ihn automatisch." },
  {
    value: "manual",
    label: "Manuell",
    tooltip: "Claude schlägt den Skill nicht mehr vor, nur noch per /name aufrufbar.",
  },
  { value: "disabled", label: "Aus", tooltip: "Skill-Ordner nach skills-disabled verschoben." },
]

function StateSegment({
  value,
  disabled,
  onChange,
  label,
}: {
  value: SkillState | "mixed"
  disabled?: boolean
  onChange: (next: SkillState) => void
  label: string
}) {
  return (
    <div
      role="group"
      aria-label={label}
      className="inline-flex shrink-0 rounded-full bg-muted p-0.5"
    >
      {STATE_OPTIONS.map((opt) => (
        <Tooltip key={opt.value}>
          <TooltipTrigger asChild>
            <button
              type="button"
              disabled={disabled}
              aria-pressed={value === opt.value}
              onClick={() => value !== opt.value && onChange(opt.value)}
              className={cn(
                "rounded-full px-2.5 py-1 text-xs font-medium transition-colors disabled:pointer-events-none disabled:opacity-50",
                value === opt.value
                  ? "bg-primary text-primary-foreground"
                  : "text-muted-foreground hover:text-foreground"
              )}
            >
              {opt.label}
            </button>
          </TooltipTrigger>
          <TooltipContent>{opt.tooltip}</TooltipContent>
        </Tooltip>
      ))}
    </div>
  )
}

function IconAction({
  icon: Icon,
  label,
  onClick,
  disabled,
}: {
  icon: typeof ClipboardCopy
  label: string
  onClick: () => void
  disabled?: boolean
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          aria-label={label}
          disabled={disabled}
          onClick={onClick}
          className={cn(
            "flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground/60 transition-colors hover:bg-accent hover:text-foreground focus-visible:bg-accent focus-visible:text-foreground focus-visible:outline-none disabled:pointer-events-none disabled:opacity-30"
          )}
        >
          <Icon className="size-4" />
        </button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

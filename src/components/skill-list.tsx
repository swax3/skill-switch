import { useMemo, useState } from "react"
import { ClipboardCopy, ClipboardX, Link2, RefreshCw, Search } from "lucide-react"
import { toast } from "sonner"
import type { Skill } from "@/lib/api"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

type SkillListProps = {
  skills: Skill[]
  loading: boolean
  pending: Set<string>
  onToggle: (skill: Skill) => void
  onRefresh: () => void
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

export function SkillList({ skills, loading, pending, onToggle, onRefresh }: SkillListProps) {
  const [query, setQuery] = useState("")

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    if (!q) return skills
    return skills.filter(
      (s) => s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q)
    )
  }, [skills, query])

  const active = filtered.filter((s) => s.enabled)
  const inactive = filtered.filter((s) => !s.enabled)

  return (
    <div className="flex h-full flex-col gap-4 p-6">
      <div className="flex shrink-0 items-center gap-2">
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Skills durchsuchen…"
            className="rounded-full bg-card pl-9 shadow-none"
          />
        </div>
        <IconAction
          icon={RefreshCw}
          label="Liste neu laden (z. B. nach Änderungen im Explorer)"
          disabled={loading}
          onClick={onRefresh}
        />
      </div>

      {loading ? (
        <p className="text-sm text-muted-foreground">Skills werden geladen…</p>
      ) : filtered.length === 0 ? (
        <p className="text-sm text-muted-foreground">Keine Skills gefunden.</p>
      ) : (
        <div className="flex-1 space-y-6 overflow-y-auto">
          <SkillGroup title="Aktiv" skills={active} pending={pending} onToggle={onToggle} />
          <SkillGroup title="Inaktiv" skills={inactive} pending={pending} onToggle={onToggle} />
        </div>
      )}
    </div>
  )
}

function SkillGroup({
  title,
  skills,
  pending,
  onToggle,
}: {
  title: string
  skills: Skill[]
  pending: Set<string>
  onToggle: (skill: Skill) => void
}) {
  if (skills.length === 0) return null
  return (
    <section>
      <h2 className="mb-2 px-1 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        {title}
      </h2>
      <div className="overflow-hidden rounded-[10px] border border-border bg-card">
        {skills.map((skill, i) => (
          <div key={skill.name}>
            {i > 0 && <div className="ml-4 h-px bg-border" />}
            <SkillRow skill={skill} disabled={pending.has(skill.name)} onToggle={onToggle} />
          </div>
        ))}
      </div>
    </section>
  )
}

function SkillRow({
  skill,
  disabled,
  onToggle,
}: {
  skill: Skill
  disabled: boolean
  onToggle: (skill: Skill) => void
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

      <Switch
        checked={skill.enabled}
        disabled={disabled}
        onCheckedChange={() => onToggle(skill)}
        aria-label={`${skill.name} ${skill.enabled ? "deaktivieren" : "aktivieren"}`}
      />
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

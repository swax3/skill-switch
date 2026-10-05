import type { Advice, LiveSession } from "@/lib/api"

// One memo per session, carried across polls in App.tsx. Pure so it's easy to test
// by hand: same inputs, same { events, next } every time.
export type Memo = {
  compactHigh: boolean
  /** turns value we already notified "loop" for, so it fires at most once per turn. */
  loopTurnAt: number | null
  /** approximate reset timestamp (minutes) we already notified for. */
  limitResetAt: number | null
}

export type Notice = { sessionId: string; title: string; body: string }

const emptyMemo = (): Memo => ({ compactHigh: false, loopTurnAt: null, limitResetAt: null })

const findAdvice = (advice: Advice[], pred: (a: Advice) => boolean) => advice.find(pred)

/** Decides which sessions deserve a notification since the last poll, and returns the
 *  updated memo to carry forward. First poll (prev empty) only seeds the memo — no
 *  burst of notifications for conditions that already existed when the app started. */
export function notificationsFor(
  prev: Map<string, Memo>,
  sessions: LiveSession[]
): { events: Notice[]; next: Map<string, Memo> } {
  const firstPoll = prev.size === 0
  const events: Notice[] = []
  const next = new Map<string, Memo>()

  for (const s of sessions) {
    const memo = { ...(prev.get(s.id) ?? emptyMemo()) }
    const notify = (body: string) => {
      if (!firstPoll) events.push({ sessionId: s.id, title: `Skill Switch · ${s.title ?? s.project}`, body })
    }

    const isHigh = s.advice[0]?.level === "high"
    if (isHigh && !memo.compactHigh) notify(s.advice[0].text)
    memo.compactHigh = isHigh

    const looping = s.turnRequests >= 15 || s.turnErrors >= 3
    if (looping && memo.loopTurnAt !== s.turns) {
      const a = findAdvice(s.advice, (a) => a.text.includes("Schleife"))
      if (a) notify(a.text)
      memo.loopTurnAt = s.turns
    }

    if (s.limitResetInSecs != null) {
      const resetAt = Math.round((Date.now() / 1000 + s.limitResetInSecs) / 60)
      if (memo.limitResetAt !== resetAt) {
        const a = findAdvice(s.advice, (a) => a.level === "info" && a.text.includes("5h-Limit"))
        if (a) notify(a.text)
        memo.limitResetAt = resetAt
      }
    } else {
      memo.limitResetAt = null
    }

    next.set(s.id, memo)
  }

  return { events, next }
}

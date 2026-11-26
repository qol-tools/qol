export type Kind = 'ask' | 'test' | 'failed' | 'blocked' | 'waiting' | 'merged' | 'note'

export type Entry = {
  agentId: string
  name: string
  kind?: Kind
  headline: string
  recommend?: string
  report: string
  at: number
  isWorking: boolean
  isDismissed: boolean
  project?: string
  successor?: string
  launcher?: string
  resumedFrom?: string
  isStoppedByUser?: boolean
  isClosed?: true
  isInterrupted?: boolean
  isRunOver?: true
  key?: string
  isDone?: boolean
  hasNewReply?: boolean
  task?: string
  doneWhen?: string
}

export type Pull = {
  agentId: string
  pullRequest: string
  number?: number
  mergeCommit?: string
  isWatched: boolean
}

export type PullState = 'open' | 'draft' | 'closed' | 'merged'

export type CheckResult = 'pass' | 'fail' | 'running'

export type PullDetail = {
  at: number
  state: PullState
  passed: number
  failed: number
  running: number
  total: number
  review?: CheckResult
  liveTest?: 'needed' | 'not needed'
  merge?: 'auto' | 'queued'
  title?: string
}

export type StrayPull = { url: string; number: number; author: string; detail: PullDetail }

export type ModelChoice = { model?: string; effort?: 'low' | 'medium' | 'high' | 'xhigh' | 'max'; at: number }

export type ModelSeen = { model: string; effort?: string; at: number }

export type RunState = { state: string; pulls?: string[] }

export type ActionKind = 'search' | 'read' | 'edit' | 'shell' | 'other'

export type Doing = { counts: Partial<Record<ActionKind, number>>; recent: string[] }

export type Lit = { writing?: string; writingUnder?: string[] }

export type Tone = 'info' | 'good' | 'warn' | 'bad'

export type Verdict = 'pass' | 'fail' | 'skip'

export type Check = { id: string; text: string; detail?: string }

export type PageBlock =
  | { kind: 'kv'; rows: { label: string; value: string; tone?: Tone }[] }
  | { kind: 'checklist'; items: Check[] }
  | { kind: 'steps'; items: { text: string; state: 'done' | 'now' | 'todo' }[] }
  | { kind: 'links'; items: { label: string; url: string }[] }
  | { kind: 'callout'; tone: Tone; text: string }
  | { kind: 'actions'; items: { id: string; label: string }[] }

export type Page = { id: string; owner: string; title: string; tab?: string; blocks: PageBlock[] }

export type PageAnswer = { item: string; verdict: Verdict; note?: string; at_ms: number }

export type LivePage = { page: Page; set_ms: number; updated_ms: number; answers: PageAnswer[] }

declare module 'claude-code' {
  interface PluginState {
    'qol-switchboard': {
      board: Shaped<Entry[]>
      confirming: string
      pulls: Pull[]
      writingTo: string
      writingSaid: { isUser: boolean; text: string }[]
      pullDetails: Record<string, PullDetail>
      modelChoices: Record<string, ModelChoice>
      modelsSeen: Record<string, ModelSeen>
      forkStates: Record<string, RunState>
      idleShown: boolean
      closedShown: boolean
      focusDoing: Doing
      litRows: Lit
      freshReply: string
      focusShown: { id: string; prevId: string; agentId: string; key: string; rows: { isUser: boolean; text: string }[] }
      rowsCapped: boolean
      starting: string
      current: string
      pages: LivePage[]
      pageShown: string
      pageCursor: string
      pageNoting: string
    }
  }
}

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

export type ModelChoice = { model?: string; effort?: 'low' | 'medium' | 'high' | 'xhigh' | 'max'; at: number }

export type ModelSeen = { model: string; effort?: string; at: number }

export type RunState = { state: string; pulls?: string[] }

export type ActionKind = 'search' | 'read' | 'edit' | 'shell' | 'other'

export type Doing = { counts: Partial<Record<ActionKind, number>>; recent: string[] }

export type Lit = { writing?: string; writingUnder?: string[] }

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
    }
  }
}

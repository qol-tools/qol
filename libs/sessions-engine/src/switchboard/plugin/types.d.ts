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
  isInterrupted?: boolean
  isRunOver?: true
  key?: string
  isDone?: boolean
  hasNewReply?: boolean
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
}

export type ModelChoice = { model?: string; effort?: 'low' | 'medium' | 'high' | 'xhigh' | 'max'; at: number }

export type ModelSeen = { model: string; effort?: string; at: number }

declare module 'claude-code' {
  interface PluginState {
    'qol-switchboard': {
      board: Shaped<Entry[]>
      isDoneOpen: boolean
      confirming: string
      pulls: Pull[]
      writingTo: string
      writingSaid: { isUser: boolean; text: string }[]
      switchboardLog: { who: string; text: string; at?: number }[]
      pullDetails: Record<string, PullDetail>
      modelChoices: Record<string, ModelChoice>
      modelsSeen: Record<string, ModelSeen>
      focusActivity: string
      freshReply: string
      focusShown: { id: string; prevId: string; agentId: string; key: string; rows: { isUser: boolean; text: string }[] }
      starting: string
      current: string
    }
  }
}

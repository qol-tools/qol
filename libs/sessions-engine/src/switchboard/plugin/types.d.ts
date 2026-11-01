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
}

declare module 'claude-code' {
  interface PluginState {
    'qol-switchboard': { board: Shaped<Entry[]>; isDoneOpen: boolean; confirming: string }
  }
}

import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Entry, Kind, Pull } from '../types'

const PROJECT = /^\s*Project:\s*(.+?)\s*$/
const FORK = 'project-fork'
const PANE = 'board'
const TITLE = 'Fork board'
const TOOL = 'fork_board'
const TOOL_NAME = 'mcp__qol-switchboard__fork_board'
const SEND = 'SendMessage'
const STORE_PREFIX = 'board:'
const CLOSED_KEY = 'pane-closed'
const INTAKE_DOORS: readonly string[] = ['prompt', 'delivery', 'attachment']
const KEEP = 100
const REPORT_CHARS = 16_000
const HAIKU_MS = 20_000
const MINUTE = 60_000
const DELIVER_MS = 2_000
const DELIVER_TIMEOUT_MS = 20_000
const WAKE_DELAY_MS = 1_500
const WAKE_FRESH_MS = 10 * MINUTE
const SAFE_ARG = /^[A-Za-z0-9_@%+=:,./-]+$/
const WATCHER = /^\s*node\s+([A-Za-z0-9_@%+=:,./-]*pr-watch\.cjs)((?:\s+[A-Za-z0-9_@%+=:,./-]+)*)\s*$/
const HELP_ARGS: readonly string[] = ['help', '--help', '-h']
const KINDS: readonly Kind[] = ['ask', 'test', 'failed', 'blocked', 'waiting', 'merged', 'note']
const NEEDS_YOU: readonly Kind[] = ['ask', 'test', 'failed', 'blocked']
const DONE: readonly Kind[] = ['merged', 'note']
const HEADER = new RegExp(`^[\\s*_\`#>-]*(${KINDS.join('|')})[*_\`]*\\s*:[*_\`]*\\s*(.+?)[\\s*_\`]*$`, 'i')
const RECOMMEND = /^[\s*_`>-]*Recommend(?:ed)?(?: answer)?[*_`]*\s*:[*_`]*\s*(.+?)[\s*_`]*$/im
const HANDBACK = /^(Another Claude session sent a message:\n<agent-message from="([A-Za-z0-9_-]+)">\n\[Subagent hand-back\][^\n]*?The report follows:\n)((?:(?: {2}[^\n]*)?\n)*?)(<\/agent-message>\n)/
const NOTIFICATION = /^<task-notification>\n[\s\S]*<\/task-notification>$/
const RESULT = /(<result>)([\s\S]*?)(<\/result>)/
const HANDED_BACK = /delivered to you as a message from/
const AGENT_SUMMARY = /^Agent "([\s\S]*)" /
const STOPPED = /\bwas stopped by the user\b|\bwon't be resumed\b/i
const TAKEOVER = /^The message above is for the fork .+, agent id ([A-Za-z0-9_-]+), which was stopped, so you take over /
const COLOR: Record<Kind, string> = {
  ask: 'warning',
  test: 'suggestion',
  failed: 'error',
  blocked: 'error',
  waiting: 'permission',
  merged: 'success',
  note: 'inactive',
}
const HAIKU_SYSTEM = `You file a coding agent's final report on a board. Answer with one line \`<kind>: <headline>\` and nothing else, where kind is one of ${KINDS.join(', ')} and the headline says in under 80 characters what happened. ask: the report asks the user a question; then add a second line \`Recommend: <the answer the report recommends>\` when it recommends one. test: the user must try something by hand. failed or blocked: something broke or stops the work. waiting: a pull request is open and waits on checks, review or the merge queue. merged: a pull request merged. note: anything else.`

const board = atom({ plugin: 'qol-switchboard', key: 'board' } as const, [], { shape: 'board-1' })
const isDoneOpen = atom({ plugin: 'qol-switchboard', key: 'isDoneOpen' } as const, false)
const confirming = atom({ plugin: 'qol-switchboard', key: 'confirming' } as const, '')
const pulls = atom({ plugin: 'qol-switchboard', key: 'pulls' } as const, [] as Pull[])

type Header = { kind: Kind; headline: string; recommend?: string }
type Report = { agentId: string; name?: string; status: string; report: string }
type Delivery = { id: string; agent_id: string; conversation: string; name?: string | null; cwd: string; pull_request?: string | null; message: string }
type LedgerRow = { agent_id: string; pull_request: string; number?: number | null; merge_commit?: string | null; is_watched: boolean }
type WakeFork = { agent_id: string; name?: string | null; description: string; cwd?: string | null; transcript: string; model?: string | null; effort?: string | null; message: string }
type Wake = { conversation: string; launched_at: string; is_handoff: boolean; forks: WakeFork[]; claimed?: boolean }

const pending = new Map<string, Promise<Header>>()

const textOf = (content: unknown): string =>
  Array.isArray(content)
    ? content.map(block => (block?.type === 'text' && typeof block.text === 'string' ? block.text : '')).join('')
    : typeof content === 'string'
      ? content
      : ''

const keyName = (name: string): string => name.split(':', 1)[0]?.trim() || name

const dedent = (text: string): string => text.split('\n').map(line => line.replace(/^ {2}/, '')).join('\n')

const field = (block: string, tag: string): string | undefined =>
  new RegExp(`<${tag}>([\\s\\S]*?)</${tag}>`).exec(block)?.[1]?.trim()

const firstLine = (text: string): string => text.split('\n').find(line => line.trim() !== '')?.trim() ?? ''

const clip = (text: string, length: number): string => (text.length > length ? `${text.slice(0, length - 1)}…` : text)

export const parseHeader = (report: string): Header | undefined => {
  const [, kind, headline] = HEADER.exec(firstLine(report)) ?? []
  if (kind === undefined || headline === undefined) return undefined
  const recommend = RECOMMEND.exec(report)?.[1]
  return { kind: kind.toLowerCase() as Kind, headline, ...(recommend === undefined ? {} : { recommend }) }
}

const fallback = (report: string): Header => ({ kind: 'note', headline: clip(firstLine(report) || 'no report', 100) })

const summarize = async ($: EngineInterface, report: string): Promise<Header> => {
  try {
    const reply = await $.model.complete({
      model: 'haiku',
      system: HAIKU_SYSTEM,
      prompt: report.slice(0, 12_000),
      maxTokens: 200,
      timeoutMs: HAIKU_MS,
    })
    return (reply.isAnswered ? parseHeader(reply.text) : undefined) ?? fallback(report)
  } catch {
    return fallback(report)
  }
}

const headerOf = ($: EngineInterface, report: string): Promise<Header> => {
  const parsed = parseHeader(report)
  if (parsed !== undefined) return Promise.resolve(parsed)
  const known = pending.get(report)
  if (known !== undefined) return known
  const asked = summarize($, report).finally(() => {
    pending.delete(report)
  })
  pending.set(report, asked)
  return asked
}

const boardLine = (entry: Entry): string =>
  [`[fork board] ${keyName(entry.name)} · ${entry.kind ?? 'working'} · ${entry.headline}`, entry.recommend === undefined ? undefined : `Recommend: ${entry.recommend}`]
    .filter(line => line !== undefined)
    .join('\n')

const rememberClosed = async ($: EngineInterface): Promise<void> => {
  try {
    await $.store.set(CLOSED_KEY, true)
  } catch {}
}

const persist = async ($: EngineInterface): Promise<void> => {
  try {
    await $.store.set(STORE_PREFIX + (await $.session.id()), await read($, board))
  } catch {}
}

const change = async ($: EngineInterface, fn: (list: Entry[]) => Entry[]): Promise<void> => {
  await update($, board, list => fn(list ?? []).slice(-KEEP))
  await persist($)
  await refreshStatus($)
}

const refreshStatus = async ($: EngineInterface): Promise<void> => {
  const list = await read($, board)
  const count = list.filter(entry => needsYou(entry)).length
  $.ui.status(count === 0 ? undefined : `Fork board · ${count} need${count === 1 ? 's' : ''} you · /board`)
}

const needsYou = (entry: Entry): boolean =>
  !entry.isDismissed && !entry.isWorking && entry.kind !== undefined && NEEDS_YOU.includes(entry.kind)

const upsert = (list: Entry[], agentId: string, fn: (entry: Entry | undefined) => Entry): Entry[] => {
  const index = list.findIndex(entry => entry.agentId === agentId)
  const next = fn(index === -1 ? undefined : list[index])
  return index === -1 ? [...list, next] : [...list.slice(0, index), ...list.slice(index + 1), next]
}

const markWorking = async ($: EngineInterface, agentId: string, name: string | undefined, project?: string): Promise<void> => {
  const now = await $.clock.now()
  const placed = project === undefined ? {} : { project }
  await change($, list =>
    upsert(list, agentId, entry =>
      entry === undefined
        ? { agentId, name: name ?? agentId, headline: '', report: '', at: now, isWorking: true, isDismissed: false, ...placed }
        : { ...entry, name: name || entry.name, at: now, isWorking: true, isDismissed: false, ...placed },
    ),
  )
}

const projectOf = (prompt: string): string | undefined => PROJECT.exec(prompt.split('\n', 1)[0] ?? '')?.[1]

export const addressed = (list: Entry[], to: string): Entry | undefined => {
  const newest = [...list].reverse()
  let entry = newest.find(one => one.agentId === to) ?? newest.find(one => !one.isDismissed && keyName(one.name) === to)
  const seen = new Set<string>()
  while (entry?.successor !== undefined && !seen.has(entry.agentId)) {
    seen.add(entry.agentId)
    const successor = entry.successor
    const next = newest.find(one => one.agentId === successor)
    if (next === undefined) break
    entry = next
  }
  return entry
}

const subagentFile = async ($: EngineInterface, agentId: string, suffix: string): Promise<string | undefined> => {
  const conversations = await $.env.get('QOL_SWITCHBOARD_CONVERSATIONS')
  if (conversations === undefined || conversations === '') return undefined
  const path = `${conversations}/${await $.session.id()}/subagents/agent-${agentId}${suffix}`
  return (await $.fs.exists(path)) ? path : undefined
}

const recordedProject = async ($: EngineInterface, agentId: string): Promise<string | undefined> => {
  const meta = await subagentFile($, agentId, '.meta.json')
  if (meta === undefined) return undefined
  const cwd = JSON.parse(String(await $.fs.read(meta)))?.cwd
  return typeof cwd === 'string' && cwd !== '' ? cwd : undefined
}

const stoppedFork = (prompt: string): string | undefined => TAKEOVER.exec(prompt.split('\n').at(-1) ?? '')?.[1]

const succeed = async ($: EngineInterface, stopped: string, successor: string): Promise<void> => {
  if (stopped === successor) return
  await change($, list => list.map(one => (one.agentId === stopped ? { ...one, successor, isDismissed: true } : one)))
}

export const takeoverPrompt = (project: string, message: string, stopped: Entry, transcript: string | undefined): string =>
  [
    `Project: ${project}`,
    message,
    '',
    `The message above is for the fork ${keyName(stopped.name)}, agent id ${stopped.agentId}, which was stopped, so you take over its work and its conversation. ${
      transcript === undefined
        ? `Its conversation is the file agent-${stopped.agentId}.jsonl in a subagents folder under the projects folder of your Claude config directory`
        : `Its conversation is in ${transcript}`
    }: read the end of it to learn where it stopped and what it was doing, then act on the message above in that context, and start its pull request watcher again in the background if it was waiting on one.`,
  ].join('\n')

type Takeover = { isStarted: false } | { isStarted: true; agentId?: string }

const startTakeover = async ($: EngineInterface, stopped: Entry, message: string, recorded?: string): Promise<Takeover> => {
  const project = await (stopped.project !== undefined ? Promise.resolve(stopped.project) : recordedProject($, stopped.agentId)).catch(() => undefined)
  if (project === undefined) return { isStarted: false }
  const transcript = recorded ?? (await subagentFile($, stopped.agentId, '.jsonl').catch(() => undefined))
  const started = await $.agent.spawn({ subagentType: FORK, description: stopped.name, prompt: takeoverPrompt(project, message, stopped, transcript) })
  if (started.deny !== undefined) return { isStarted: false }
  if (started.agentId === undefined) return { isStarted: true }
  await succeed($, stopped.agentId, started.agentId).catch(() => undefined)
  return { isStarted: true, agentId: started.agentId }
}

const takeovers = new Map<string, Promise<Takeover>>()

const takeOver = async ($: EngineInterface, stopped: Entry, message: string, transcript?: string): Promise<Takeover & { isFirst: boolean }> => {
  const running = takeovers.get(stopped.agentId)
  if (running !== undefined) return { ...(await running), isFirst: false }
  const started = startTakeover($, stopped, message, transcript).catch((): Takeover => ({ isStarted: false }))
  takeovers.set(stopped.agentId, started)
  const takeover = await started
  if (!takeover.isStarted) takeovers.delete(stopped.agentId)
  return { ...takeover, isFirst: true }
}

const continued = (name: string, takeover: Takeover & { isStarted: true }): string =>
  takeover.agentId === undefined
    ? `${keyName(name)} was stopped, so it continues as a new fork that reads its conversation and has your message.`
    : `${keyName(name)} was stopped, so it continues as a new fork, agent id ${takeover.agentId}, that reads its conversation and has your message. Address ${takeover.agentId} from now on.`

const isStopped = (reason: unknown): boolean => STOPPED.test(typeof reason === 'string' ? reason : JSON.stringify(reason ?? ''))

const isUndelivered = (answer: { deny?: string; isError?: true; result?: unknown }): boolean =>
  answer.deny === undefined &&
  (answer.isError === true || (typeof answer.result === 'object' && answer.result !== null && (answer.result as { success?: unknown }).success === false)) &&
  isStopped(answer.result)

const file = async ($: EngineInterface, report: Report): Promise<Entry> => {
  const header =
    report.status === 'completed' || report.report !== ''
      ? await headerOf($, report.report)
      : { kind: report.status === 'failed' ? ('failed' as const) : ('note' as const), headline: report.status === 'failed' ? 'the fork failed' : 'stopped' }
  const now = await $.clock.now()
  let filed: Entry | undefined
  let isNew = true
  await change($, list =>
    upsert(list, report.agentId, entry => {
      const isSame = entry !== undefined && entry.kind !== undefined && entry.report === clip(report.report, REPORT_CHARS)
      isNew = !isSame
      filed = {
        agentId: report.agentId,
        name: report.name || entry?.name || report.agentId,
        ...header,
        report: clip(report.report, REPORT_CHARS),
        at: isSame ? entry.at : now,
        isWorking: false,
        isDismissed: isSame ? entry.isDismissed : false,
      }
      return filed
    }),
  )
  const entry = filed as Entry
  if (isNew && needsYou(entry)) {
    void $.ui.notify(`${entry.kind}: ${entry.headline}`, { title: keyName(entry.name) }).catch(() => {})
  }
  return entry
}

export const quote = (arg: string): string => (SAFE_ARG.test(arg) ? arg : `'${arg.replace(/'/g, `'\\''`)}'`)

export const watcherArgs = (command: string): string[] | undefined => {
  const match = WATCHER.exec(command)
  if (match === null || match[1] === undefined) return undefined
  const args = ['node', match[1], ...(match[2] ?? '').split(/\s+/).filter(arg => arg !== '')]
  return args.some(arg => HELP_ARGS.includes(arg)) ? undefined : args
}

export const watchCommand = (program: string, agentId: string, conversation: string, name: string | undefined, args: readonly string[]): string =>
  [program, 'fork-watch', '--agent', agentId, '--conversation', conversation, ...(name === undefined || name === '' ? [] : ['--title', name]), '--', ...args].map(quote).join(' ')

export const pullText = (pull: Pull): string =>
  `#${pull.number ?? '?'} ${pull.mergeCommit !== undefined ? `merged ${pull.mergeCommit.slice(0, 7)}` : pull.isWatched ? 'open · CLI Sessions watching' : 'open · no watcher'}`

const pullsOf = (all: readonly Pull[], agentId: string): Pull[] => all.filter(pull => pull.agentId === agentId)

const ledgerOf = (rows: readonly LedgerRow[]): Pull[] =>
  rows.map(row => ({
    agentId: row.agent_id,
    pullRequest: row.pull_request,
    ...(typeof row.number === 'number' ? { number: row.number } : {}),
    ...(typeof row.merge_commit === 'string' ? { mergeCommit: row.merge_commit } : {}),
    isWatched: row.is_watched,
  }))

const conversationFile = async ($: EngineInterface, conversation: string, agentId: string): Promise<string | undefined> => {
  const conversations = await $.env.get('QOL_SWITCHBOARD_CONVERSATIONS')
  if (conversations === undefined || conversations === '') return undefined
  const path = `${conversations}/${conversation}/subagents/agent-${agentId}.jsonl`
  return (await $.fs.exists(path)) ? path : undefined
}

const undelivered = async ($: EngineInterface, agentId: string, name: string, reason: string, message: string): Promise<void> => {
  await file($, {
    agentId,
    name,
    status: 'failed',
    report: `failed: the switchboard could not reach ${keyName(name)}\n\n${reason}\n\nWhat it would have been sent:\n${message}`,
  })
}

const stand = (agentId: string, name: string, project: string | undefined): Entry => ({
  agentId,
  name,
  headline: '',
  report: '',
  at: 0,
  isWorking: false,
  isDismissed: true,
  ...(project === undefined ? {} : { project }),
})

const reach = async ($: EngineInterface, agentId: string, name: string, project: string | undefined, message: string): Promise<void> => {
  const target = addressed(await read($, board), agentId) ?? stand(agentId, name, project)
  const sent = await $.session.send({ to: { agentId: target.agentId }, text: message })
  if (sent.isDelivered) {
    await markWorking($, target.agentId, name)
    return
  }
  if (isStopped(sent.reason)) {
    const takeover = await takeOver($, target, message)
    if (takeover.isStarted) return
  }
  await undelivered($, target.agentId, name, String(sent.reason ?? 'not delivered'), message)
}

export const relay = async ($: EngineInterface, result: Delivery): Promise<void> => {
  const name = result.name ?? result.agent_id
  if (result.conversation === (await $.session.id())) return reach($, result.agent_id, name, result.cwd, result.message)
  const transcript = await conversationFile($, result.conversation, result.agent_id).catch(() => undefined)
  const takeover = await takeOver($, stand(result.agent_id, name, result.cwd), result.message, transcript)
  if (!takeover.isStarted) await undelivered($, result.agent_id, name, 'it belongs to an earlier switchboard conversation, and no new fork could take it over', result.message)
}

let isDelivering = false
let delivered = ''

export const deliver = async ($: EngineInterface): Promise<void> => {
  if (isDelivering) return
  isDelivering = true
  try {
    const program = await $.env.get('QOL_CLI_SESSIONS')
    const watches = await $.env.get('QOL_SWITCHBOARD_WATCHES')
    if (program === undefined || program === '' || watches === undefined || watches === '' || !(await $.fs.exists(watches))) return
    const listing = (await $.fs.list(watches))
      .map(entry => `${entry.name}:${entry.size}:${entry.mtimeMs}`)
      .sort()
      .join('|')
    if (listing === delivered) return
    const ran = await $.process.run([program, 'fork-watch', '--deliver'], { timeoutMs: DELIVER_TIMEOUT_MS })
    if (ran.exitCode !== 0) return
    delivered = listing
    const parsed = JSON.parse(ran.stdout) as { results?: Delivery[]; pulls?: LedgerRow[] }
    await update($, pulls, () => ledgerOf(parsed.pulls ?? []))
    for (const result of parsed.results ?? []) await relay($, result).catch(() => undefined)
    $.ui.invalidate('ui.render')
  } catch {
  } finally {
    isDelivering = false
  }
}

const wakeFork = async ($: EngineInterface, wake: Wake, fork: WakeFork): Promise<void> => {
  const name = fork.name ?? fork.description
  if (!wake.is_handoff) return reach($, fork.agent_id, name, fork.cwd ?? undefined, fork.message)
  if (fork.cwd === undefined || fork.cwd === null || fork.cwd === '') {
    await undelivered($, fork.agent_id, name, 'its project folder is unknown, so no new fork could take over its work', fork.message)
    return
  }
  const started = await $.agent.spawn({
    subagentType: FORK,
    description: fork.description,
    prompt: `Project: ${fork.cwd}\n${fork.message}`,
    ...(typeof fork.model === 'string' ? { model: fork.model } : {}),
    ...(typeof fork.effort === 'string' ? { effort: fork.effort } : {}),
  })
  if (started.deny !== undefined) await undelivered($, fork.agent_id, name, started.deny, fork.message)
}

export const wakeForks = async ($: EngineInterface): Promise<void> => {
  const path = await $.env.get('QOL_SWITCHBOARD_WAKE')
  if (path === undefined || path === '' || !(await $.fs.exists(path))) return
  const wake = JSON.parse(String(await $.fs.read(path))) as Wake
  if (wake.claimed === true || !Array.isArray(wake.forks) || wake.forks.length === 0) return
  const launched = Date.parse(wake.launched_at)
  if (!Number.isFinite(launched) || (await $.clock.now()) - launched > WAKE_FRESH_MS) return
  if (!wake.is_handoff && wake.conversation !== (await $.session.id())) return
  await $.fs.write(path, JSON.stringify({ ...wake, claimed: true }))
  for (const fork of wake.forks) await wakeFork($, wake, fork).catch(() => undefined)
}

const once = (text: string, needle: string): boolean => text.indexOf(needle) !== -1 && text.indexOf(needle) === text.lastIndexOf(needle)

export type Origin = 'peer' | 'task-notification'

export const intake = async (
  $: EngineInterface,
  text: string,
  origin: Origin,
  isFork: (agentId: string) => boolean,
): Promise<string | undefined> => {
  if (origin === 'peer') {
    if (!once(text, '<agent-message ') || !once(text, '</agent-message>')) return undefined
    const [whole, head, agentId, body, tail] = HANDBACK.exec(text) ?? []
    if (whole === undefined || head === undefined || agentId === undefined || body === undefined || tail === undefined) return undefined
    if (!isFork(agentId)) return undefined
    const entry = await file($, { agentId, status: 'completed', report: dedent(body).trim() })
    return `${head}  ${boardLine(entry).split('\n').join('\n  ')}\n${tail}${text.slice(whole.length)}`
  }
  const block = text.trim()
  if (!NOTIFICATION.test(block)) return undefined
  for (const tag of ['<task-notification>', '</task-notification>', '<task-id>', '<status>']) {
    if (!once(block, tag)) return undefined
  }
  const agentId = field(block, 'task-id')
  if (agentId === undefined || !isFork(agentId)) return undefined
  const status = field(block, 'status') ?? ''
  const name = AGENT_SUMMARY.exec(field(block, 'summary') ?? '')?.[1]
  const result = RESULT.exec(block)
  if (result !== null && (!once(block, '<result>') || !once(block, '</result>'))) return undefined
  const report = result?.[2]?.trim() ?? ''
  if (HANDED_BACK.test(report)) return undefined
  const entry = await file($, { agentId, name, status, report })
  const line = boardLine(entry)
  const filed =
    result === null
      ? block.replace('</task-notification>', () => `<result>${line}</result>\n</task-notification>`)
      : block.replace(RESULT, (_, open: string, __, close: string) => `${open}${line}${close}`)
  return text.replace(block, () => filed)
}

const age = (ms: number): string => {
  const minutes = Math.max(0, Math.floor(ms / MINUTE))
  if (minutes < 60) return `${minutes}m`
  const hours = Math.floor(minutes / 60)
  return hours < 24 ? `${hours}h` : `${Math.floor(hours / 24)}d`
}

const pullNote = (all: readonly Pull[], agentId: string): string => {
  const own = pullsOf(all, agentId)
  return own.length === 0 ? '' : ` [${own.map(pullText).join('; ')}]`
}

export const readText = (list: Entry[], fork: string | undefined, all: readonly Pull[] = []): string => {
  if (fork !== undefined && fork.trim() !== '') {
    const wanted = fork.trim()
    const entry = [...list].reverse().find(one => one.agentId === wanted || keyName(one.name) === wanted || one.name === wanted)
    if (entry === undefined) return `No fork board row for ${wanted}.`
    const own = pullsOf(all, entry.agentId)
    const prs = own.length === 0 ? '' : `\npull requests: ${own.map(pull => `${pullText(pull)} ${pull.pullRequest}`).join('; ')}`
    return `${boardLine(entry)}\nagent id: ${entry.agentId}${prs}\n\n${entry.report || '(no report yet)'}`
  }
  const shown = list.filter(entry => !entry.isDismissed)
  if (shown.length === 0) return 'The fork board is empty.'
  return shown
    .map(entry => `${entry.isWorking ? `[fork board] ${keyName(entry.name)} · working` : boardLine(entry)} (agent id ${entry.agentId})${pullNote(all, entry.agentId)}`)
    .join('\n')
}

export const register: Register = on => {
  on('agent.spawn', { subagentType: FORK }, ($, e, next) => {
    if (e.resume !== undefined) return next(e)
    const project = PROJECT.exec(e.prompt.split('\n', 1)[0] ?? '')
    if (project === null) {
      return { deny: "A project-fork's prompt must start with the line `Project: <absolute path of an existing project folder>`." }
    }
    return next({ ...e, cwd: project[1] })
  }).catch(($, e, next) =>
    next.called ? next(e) : { deny: 'The qol-switchboard plugin could not start this fork in its project folder.' },
  )

  on('agent.spawn', { subagentType: FORK }, async ($, e, next) => {
    const started = await next(e)
    const agentId = started.agentId ?? e.resume?.agentId
    if (agentId === undefined) return started
    await markWorking($, agentId, e.description, e.resume === undefined ? projectOf(e.prompt) : undefined)
    const stopped = e.resume === undefined ? stoppedFork(e.prompt) : undefined
    if (stopped !== undefined) await succeed($, stopped, agentId)
    return started
  }).catch(($, e, next) => next(e))

  on('session.start', async ($, e, next) => {
    const started = await next(e)
    try {
      const id = await $.session.id()
      if ((await read($, board)).length === 0) {
        const saved = await $.store.get(STORE_PREFIX + id)
        if (Array.isArray(saved)) await update($, board, () => saved as Entry[])
      }
      for (const key of await $.store.keys()) {
        if (key.startsWith(STORE_PREFIX) && key !== STORE_PREFIX + id) await $.store.delete(key)
      }
      await $.command.register({ name: PANE, description: "Show the fork board: each fork's last report, what needs you first", immediate: true })
      await $.tool.register({
        name: TOOL,
        description:
          "Reads the fork board, where the qol-switchboard plugin files every fork's final report. With no fork, one line per fork: key-name, kind (ask, test, failed, blocked, waiting, merged, note, or working) and headline. With fork set to a key-name or agent id, that fork's full last report.",
        inputSchema: {
          type: 'object',
          properties: { fork: { type: 'string', description: "A fork's key-name or agent id; leave it out for every row." } },
        },
        isDeferred: false,
      })
      await refreshStatus($)
      if (e.isInteractive && (await $.store.get(CLOSED_KEY)) !== true) void $.ui.open({ id: PANE, title: TITLE })
      $.clock.every(MINUTE, () => $.ui.invalidate('ui.render'))
      $.clock.every(DELIVER_MS, () => void deliver($))
      $.clock.after(WAKE_DELAY_MS, () => void wakeForks($).catch(() => undefined))
    } catch {}
    return started
  })

  on('command.run', { command: PANE }, async $ => {
    await $.store.delete(CLOSED_KEY).catch(() => {})
    const opened = await $.ui.open({ id: PANE, title: TITLE, focus: true })
    return { text: opened.isPlaced ? 'Fork board opened.' : `The fork board waits for room: ${opened.reason}` }
  }).catch(() => ({ text: 'The fork board could not open.' }))

  on('ui.close', async ($, e, next) => {
    const closed = await next(e)
    if (e.id === PANE && e.origin?.kind === 'person') await rememberClosed($)
    return closed
  })

  on('tool.call', { tool: TOOL_NAME }, async ($, e) => {
    const fork = typeof e.fork === 'string' ? e.fork : undefined
    return { result: readText(await read($, board), fork, await read($, pulls)) }
  }).catch(() => ({ deny: 'The fork board could not be read.' }))

  on('tool.call', { tool: SEND }, async ($, e, next) => {
    if (e.agentId !== undefined || typeof e.to !== 'string' || typeof e.message !== 'string') return next(e)
    const target = addressed(await read($, board), e.to)
    if (target === undefined) return next(e)
    const answer = await next(target.agentId === e.to ? e : { ...e, to: target.agentId })
    if (!isUndelivered(answer)) return answer
    const takeover = await takeOver($, target, e.message)
    if (!takeover.isStarted) return answer
    if (takeover.isFirst) return { result: { success: true, message: continued(target.name, takeover) } }
    const successor = takeover.agentId ?? addressed(await read($, board), target.agentId)?.agentId
    return successor === undefined || successor === target.agentId ? answer : next({ ...e, to: successor })
  }).catch(($, e, next) => next(e))

  on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
    if (e.agentId === undefined || e.run_in_background !== true || typeof e.command !== 'string') return next(e)
    const args = watcherArgs(e.command)
    if (args === undefined) return next(e)
    const entry = (await read($, board)).find(one => one.agentId === e.agentId)
    const program = await $.env.get('QOL_CLI_SESSIONS')
    if (entry === undefined || program === undefined || program === '') return next(e)
    return next({ ...e, command: watchCommand(program, e.agentId, await $.session.id(), entry.name, args) })
  }).catch(($, e, next) => next(e))

  on('turn.start', async ($, e, next) => {
    if (e.agentId !== undefined) {
      const list = await read($, board)
      if (list.some(entry => entry.agentId === e.agentId && !entry.isWorking)) await markWorking($, e.agentId, undefined)
    }
    return next(e)
  }).catch(($, e, next) => next(e))

  on('session.append', async ($, e, next) => {
    const origin = e.origin.kind
    if (e.agentId !== undefined || !INTAKE_DOORS.includes(e.door) || (origin !== 'peer' && origin !== 'task-notification')) return next(e)
    const content = e.message.content
    const text = textOf(content)
    if (!text.includes('<task-notification>') && !text.includes('[Subagent hand-back]')) return next(e)
    const list = await read($, board)
    const rewritten = await intake($, text, origin, agentId => list.some(entry => entry.agentId === agentId))
    if (rewritten === undefined) return next(e)
    return next({ ...e, message: { ...e.message, content: [{ type: 'text', text: rewritten }] } })
  }).catch(($, e, next) => next(e))

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const list = (await read($, board)).filter(entry => !entry.isDismissed)
    const isOpen = await read($, isDoneOpen)
    const asking = await read($, confirming)
    const ledger = await read($, pulls)
    const now = await $.clock.now()
    const viewed = e.props.view?.agentId
    const isFocused = e.props.isFocused
    const needs = list.filter(needsYou).sort((a, b) => a.at - b.at)
    const working = list.filter(entry => entry.isWorking).sort((a, b) => a.at - b.at)
    const waiting = list.filter(entry => !entry.isWorking && entry.kind === 'waiting')
    const done = list.filter(entry => !entry.isWorking && entry.kind !== undefined && DONE.includes(entry.kind)).reverse()
    const firstAsk = needs.find(entry => entry.recommend !== undefined)
    let n = 0
    const hotkey = () => {
      n += 1
      return n <= 9 ? { hotkey: String(n) } : {}
    }
    const open = (entry: Entry) => () => {
      void $.ui.view({ agentId: entry.agentId })
    }
    const dismiss = (agentIds: string[]) => () => {
      void change($, all => all.map(one => (agentIds.includes(one.agentId) ? { ...one, isDismissed: true } : one)))
    }
    const confirm = (agentId: string) => () => {
      void update($, confirming, () => agentId)
    }
    const send = (entry: Entry) => async () => {
      await update($, confirming, () => '')
      const text = entry.recommend ?? ''
      const sent = await $.session.send({ to: { agentId: entry.agentId }, text })
      if (!sent.isDelivered) {
        const takeover = isStopped(sent.reason) ? await takeOver($, entry, text) : { isStarted: false }
        $.ui.toast(
          takeover.isStarted
            ? `${keyName(entry.name)} was stopped, so it continues as a new fork with: ${text}`
            : `Not sent to ${keyName(entry.name)}: ${sent.reason}`,
        )
        return
      }
      await markWorking($, entry.agentId, undefined)
      $.ui.toast(`Sent to ${keyName(entry.name)}: ${entry.recommend}`)
    }
    const title = (entry: Entry, label: string) => (
      <Button key={`open-${entry.agentId}`} plain {...hotkey()} onPress={open(entry)}>
        <Text color={entry.isWorking ? 'claude' : COLOR[entry.kind ?? 'note']}>●</Text> <Text bold={entry.agentId === viewed}>{keyName(entry.name)}</Text>{' '}
        <Text dimColor>
          {label} · {age(now - entry.at)}
        </Text>
      </Button>
    )
    const prs = (entry: Entry) =>
      pullsOf(ledger, entry.agentId).map(pull => (
        <Text key={`pr-${entry.agentId}-${pull.pullRequest}`} dimColor={pull.mergeCommit === undefined && pull.isWatched} color={pull.mergeCommit !== undefined ? 'success' : pull.isWatched ? undefined : 'warning'} wrap="wrap">
          {pullText(pull)}
        </Text>
      ))
    const heading = (text: string) => (
      <Box marginTop={1}>
        <Text dimColor bold>
          {text}
        </Text>
      </Box>
    )

    const keys = isFocused ? 'digit opens a fork · tab walks · esc back to the prompt' : 'ctrl+x tab to use the board · ctrl+x x closes it'
    const close = (
      <Button key="close" plain dimColor {...(isFocused ? { hotkey: 'x' } : {})} onPress={async () => {
          await rememberClosed($)
          await $.ui.close({ id: PANE })
        }}>
        close
      </Button>
    )

    if (list.length === 0) {
      return (
        <Box flexDirection="column">
          <Text dimColor>No forks yet. Each fork's report lands here as one line.</Text>
          <Box flexDirection="row" gap={2}>
            <Text dimColor>{keys}</Text>
            {close}
          </Box>
        </Box>
      )
    }
    return (
      <Box flexDirection="column">
        <Text>{needs.length === 0 ? 'Nothing needs you.' : `${needs.length} need${needs.length === 1 ? 's' : ''} you.`}</Text>
        <Box flexDirection="row" gap={2}>
          <Text dimColor wrap="wrap">
            {keys}
          </Text>
          {close}
        </Box>
        {needs.length > 0 && heading('NEEDS YOU')}
        {needs.map(entry => (
          <Box key={`need-${entry.agentId}`} flexDirection="column">
            {title(entry, entry.kind ?? 'note')}
            <Box paddingLeft={2} flexDirection="column">
              <Text wrap="wrap">{entry.headline}</Text>
              {prs(entry)}
              {entry.recommend !== undefined && <Text dimColor wrap="wrap">Recommend: {entry.recommend}</Text>}
              {entry.recommend !== undefined && asking === entry.agentId && (
                <Text wrap="wrap">
                  Send "{entry.recommend}" to {keyName(entry.name)}?
                </Text>
              )}
              <Box flexDirection="row" gap={2}>
                {entry.recommend !== undefined && asking === entry.agentId && (
                  <Button key={`confirm-${entry.agentId}`} plain hotkey="y" onPress={send(entry)}>
                    send
                  </Button>
                )}
                {entry.recommend !== undefined && asking === entry.agentId && (
                  <Button key={`cancel-${entry.agentId}`} plain hotkey="n" onPress={confirm('')}>
                    cancel
                  </Button>
                )}
                {entry.recommend !== undefined && asking !== entry.agentId && (
                  <Button key={`send-${entry.agentId}`} plain {...(entry === firstAsk && asking === '' ? { hotkey: 'y' } : {})} onPress={confirm(entry.agentId)}>
                    send recommended
                  </Button>
                )}
                <Button key={`dismiss-${entry.agentId}`} plain dimColor onPress={dismiss([entry.agentId])}>
                  dismiss
                </Button>
              </Box>
            </Box>
          </Box>
        ))}
        {working.length > 0 && heading('WORKING')}
        {working.map(entry => (
          <Box key={`work-${entry.agentId}`} flexDirection="column">
            {title(entry, 'working')}
            <Box paddingLeft={2} flexDirection="column">
              {prs(entry)}
            </Box>
          </Box>
        ))}
        {waiting.length > 0 && heading('WAITING ON A PULL REQUEST')}
        {waiting.map(entry => (
          <Box key={`wait-${entry.agentId}`} flexDirection="column">
            {title(entry, 'waiting')}
            <Box paddingLeft={2} flexDirection="column">
              <Text dimColor wrap="wrap">
                {entry.headline}
              </Text>
              {prs(entry)}
            </Box>
          </Box>
        ))}
        {done.length > 0 && (
          <Box marginTop={1} flexDirection="row" gap={2}>
            <Button key="done-toggle" plain hotkey="d" onPress={() => void update($, isDoneOpen, value => !value)}>
              {`${isOpen ? 'hide' : 'show'} done · ${done.length}`}
            </Button>
            <Button key="done-clear" plain dimColor hotkey="c" onPress={dismiss(done.map(entry => entry.agentId))}>
              clear done
            </Button>
          </Box>
        )}
        {isOpen &&
          done.map(entry => (
            <Box key={`done-${entry.agentId}`} flexDirection="column">
              {title(entry, entry.kind ?? 'note')}
              <Box paddingLeft={2} flexDirection="column">
                <Text dimColor wrap="wrap">
                  {entry.headline}
                </Text>
                {prs(entry)}
              </Box>
            </Box>
          ))}
      </Box>
    )
  })
}

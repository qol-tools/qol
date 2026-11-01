import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Entry, Kind } from '../types'

const PROJECT = /^\s*Project:\s*(.+?)\s*$/
const FORK = 'project-fork'
const PANE = 'board'
const TITLE = 'Fork board'
const TOOL = 'fork_board'
const TOOL_NAME = 'mcp__qol-switchboard__fork_board'
const SEND = 'SendMessage'
const STORE_PREFIX = 'board:'
const CLOSED_KEY = 'pane-closed'
const TYPED_TOGGLE = /^\s*qols\s+board\s*$/i
const CLEAR_TYPED_MS = 100
const INTAKE_DOORS: readonly string[] = ['prompt', 'delivery', 'attachment']
const KEEP = 100
const REPORT_CHARS = 16_000
const HAIKU_MS = 20_000
const MINUTE = 60_000
const KINDS: readonly Kind[] = ['ask', 'test', 'failed', 'blocked', 'waiting', 'merged', 'note']
const NEEDS_YOU: readonly Kind[] = ['ask', 'test', 'failed', 'blocked']
const DONE: readonly Kind[] = ['merged', 'note']
const MARKUP = /^[\s*_`#>-]+|[\s*_`]+$/g
const HANDBACK = /^(Another Claude session sent a message:\n<agent-message from="([A-Za-z0-9_-]+)">\n\[Subagent hand-back\][^\n]*?The report follows:\n)((?:(?: {2}[^\n]*)?\n)*?)(<\/agent-message>\n)/
const NOTIFICATION = /^<task-notification>\n[\s\S]*<\/task-notification>$/
const RESULT = /(<result>)([\s\S]*?)(<\/result>)/
const HANDED_BACK = /delivered to you as a message from/
const AGENT_SUMMARY = /^Agent "([\s\S]*)" /
const STOPPED = /\bwas stopped by the user\b|\bwon't be resumed\b/i
const LAUNCHER = 'Launcher: '
const FORK_TO_FORK =
  'Not sent: an agent inside a fork messages only, by agent id, the fork it started or the fork that started it. Another fork\'s work is not yours; put what it needs in your report instead.'
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
const HAIKU_SYSTEM = `You sort a coding agent's final report on a board. Answer with one of these words on the first line and nothing else on it: ${KINDS.join(', ')}. ask: the report asks the user a question. test: the user must try something by hand. failed or blocked: something broke or stops the work. waiting: a pull request is open and waits on checks, review or the merge queue. merged: a pull request merged. note: anything else. For ask, when the report recommends an answer to its question, write that answer on the second line, in the words the user would send back; otherwise write nothing more.`

const board = atom({ plugin: 'qol-switchboard', key: 'board' } as const, [], { shape: 'board-1' })
const isDoneOpen = atom({ plugin: 'qol-switchboard', key: 'isDoneOpen' } as const, false)
const confirming = atom({ plugin: 'qol-switchboard', key: 'confirming' } as const, '')

type Filing = { kind: Kind; headline: string; recommend?: string }
type Sorting = { kind: Kind; recommend?: string }
type Report = { agentId: string; name?: string; status: string; report: string }

const pending = new Map<string, Promise<Sorting>>()

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

export const headlineOf = (report: string): string => clip(firstLine(report).replace(MARKUP, '') || 'no report', 100)

export const parseSorting = (reply: string): Sorting | undefined => {
  const [first = '', ...rest] = reply.split('\n').map(line => line.replace(MARKUP, '')).filter(line => line !== '')
  const kind = KINDS.find(one => one === first.toLowerCase().replace(/[.:]$/, ''))
  if (kind === undefined) return undefined
  const recommend = kind === 'ask' ? rest[0] : undefined
  return { kind, ...(recommend === undefined ? {} : { recommend }) }
}

const fallback: Sorting = { kind: 'note' }

const sort = async ($: EngineInterface, report: string): Promise<Sorting> => {
  try {
    const reply = await $.model.complete({
      model: 'haiku',
      system: HAIKU_SYSTEM,
      prompt: report.slice(0, 12_000),
      maxTokens: 200,
      timeoutMs: HAIKU_MS,
    })
    return (reply.isAnswered ? parseSorting(reply.text) : undefined) ?? fallback
  } catch {
    return fallback
  }
}

const sortingOf = ($: EngineInterface, report: string): Promise<Sorting> => {
  const known = pending.get(report)
  if (known !== undefined) return known
  const asked = sort($, report).finally(() => {
    pending.delete(report)
  })
  pending.set(report, asked)
  return asked
}

const filingOf = async ($: EngineInterface, report: string): Promise<Filing> => ({ ...(await sortingOf($, report)), headline: headlineOf(report) })

const boardLine = (entry: Entry): string => `[fork board] ${keyName(entry.name)}: ${entry.headline}`

const stateOf = (entry: Entry): string =>
  entry.isWorking
    ? 'working'
    : entry.kind !== undefined && NEEDS_YOU.includes(entry.kind)
      ? 'needs the user'
      : entry.kind === 'waiting'
        ? 'waiting on its pull request'
        : 'done'

const rememberClosed = async ($: EngineInterface): Promise<void> => {
  try {
    await $.store.set(CLOSED_KEY, true)
  } catch {}
}

const toggleBoard = async ($: EngineInterface): Promise<string> => {
  const isShown = (await $.ui.panes()).some(pane => pane.id === PANE && pane.isShown && pane.isPlaced)
  if (isShown) {
    await rememberClosed($)
    await $.ui.close({ id: PANE })
    return 'Fork board closed. qols board or /board opens it again.'
  }
  await $.store.delete(CLOSED_KEY).catch(() => {})
  const opened = await $.ui.open({ id: PANE, title: TITLE, focus: true })
  return opened.isPlaced ? 'Fork board opened. qols board or /board closes it.' : `The fork board waits for room: ${opened.reason}`
}

const clearTypedToggle = ($: EngineInterface): void => {
  $.clock.after(CLEAR_TYPED_MS, () => {
    void (async () => {
      if (TYPED_TOGGLE.test((await $.prompt.read()).text)) await $.prompt.fill({ text: '', mode: 'replace' })
    })().catch(() => {})
  })
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

const markWorking = async ($: EngineInterface, agentId: string, name: string | undefined, project?: string, launcher?: string): Promise<void> => {
  const now = await $.clock.now()
  const placed = { ...(project === undefined ? {} : { project }), ...(launcher === undefined ? {} : { launcher }) }
  await change($, list =>
    upsert(list, agentId, entry =>
      entry === undefined
        ? { agentId, name: name ?? agentId, headline: '', report: '', at: now, isWorking: true, isDismissed: false, ...placed }
        : { ...entry, name: name || entry.name, at: now, isWorking: true, isDismissed: false, ...placed },
    ),
  )
}

export const launcherLine = (parentAgentId: string | undefined): string =>
  parentAgentId === undefined
    ? `${LAUNCHER}the switchboard, whose messages reach you as from the coordinator.`
    : `${LAUNCHER}the fork with agent id ${parentAgentId}, whose messages reach you as an agent-message from "${parentAgentId}".`

export const withLauncher = (prompt: string, parentAgentId: string | undefined): string => {
  const [first = '', ...rest] = prompt.split('\n')
  const body = rest[0]?.startsWith(LAUNCHER) ? rest.slice(1) : rest
  return [first, launcherLine(parentAgentId), ...body].join('\n')
}

const recipientId = (to: unknown): string | undefined =>
  typeof to === 'string' ? to : typeof to === 'object' && to !== null && typeof (to as { agentId?: unknown }).agentId === 'string' ? (to as { agentId: string }).agentId : undefined

export const mayMessage = (list: Entry[], from: string, to: unknown): boolean => {
  const id = recipientId(to)
  const sender = list.find(one => one.agentId === from)
  const target = id === undefined ? undefined : list.find(one => one.agentId === id)
  if (sender === undefined || target === undefined) return false
  return target.launcher === from || (sender.launcher !== undefined && sender.launcher === target.agentId)
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
    launcherLine(undefined),
    message,
    '',
    `The message above is for the fork ${keyName(stopped.name)}, agent id ${stopped.agentId}, which was stopped, so you take over its work and its conversation. ${
      transcript === undefined
        ? `Its conversation is the file agent-${stopped.agentId}.jsonl in a subagents folder under the projects folder of your Claude config directory`
        : `Its conversation is in ${transcript}`
    }: read the end of it to learn where it stopped and what it was doing, then act on the message above in that context, and start its pull request watcher again in the background if it was waiting on one.`,
  ].join('\n')

type Takeover = { isStarted: false } | { isStarted: true; agentId?: string }

const startTakeover = async ($: EngineInterface, stopped: Entry, message: string): Promise<Takeover> => {
  const project = await (stopped.project !== undefined ? Promise.resolve(stopped.project) : recordedProject($, stopped.agentId)).catch(() => undefined)
  if (project === undefined) return { isStarted: false }
  const transcript = await subagentFile($, stopped.agentId, '.jsonl').catch(() => undefined)
  const started = await $.agent.spawn({ subagentType: FORK, description: stopped.name, prompt: takeoverPrompt(project, message, stopped, transcript) })
  if (started.deny !== undefined) return { isStarted: false }
  if (started.agentId === undefined) return { isStarted: true }
  await succeed($, stopped.agentId, started.agentId).catch(() => undefined)
  return { isStarted: true, agentId: started.agentId }
}

const takeovers = new Map<string, Promise<Takeover>>()

const takeOver = async ($: EngineInterface, stopped: Entry, message: string): Promise<Takeover & { isFirst: boolean }> => {
  const running = takeovers.get(stopped.agentId)
  if (running !== undefined) return { ...(await running), isFirst: false }
  const started = startTakeover($, stopped, message).catch((): Takeover => ({ isStarted: false }))
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
  const filing: Filing =
    report.status === 'completed' || report.report !== ''
      ? await filingOf($, report.report)
      : { kind: report.status === 'failed' ? 'failed' : 'note', headline: report.status === 'failed' ? 'The fork failed.' : 'Stopped.' }
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
        ...filing,
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
    void $.ui.notify(entry.headline, { title: keyName(entry.name) }).catch(() => {})
  }
  return entry
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

const readText = (list: Entry[], fork: string | undefined): string => {
  if (fork !== undefined && fork.trim() !== '') {
    const wanted = fork.trim()
    const entry = [...list].reverse().find(one => one.agentId === wanted || keyName(one.name) === wanted || one.name === wanted)
    if (entry === undefined) return `No fork board row for ${wanted}.`
    return `${boardLine(entry)}\nagent id: ${entry.agentId}\n\n${entry.report || '(no report yet)'}`
  }
  const shown = list.filter(entry => !entry.isDismissed)
  if (shown.length === 0) return 'The fork board is empty.'
  return shown
    .map(entry => `${entry.isWorking ? `[fork board] ${keyName(entry.name)}` : boardLine(entry)} (${stateOf(entry)}, agent id ${entry.agentId})`)
    .join('\n')
}

export const register: Register = on => {
  let isAutoOpenPending = false

  on('agent.spawn', { subagentType: FORK }, ($, e, next) => {
    if (e.resume !== undefined) return next(e)
    const project = PROJECT.exec(e.prompt.split('\n', 1)[0] ?? '')
    if (project === null) {
      return { deny: "A project-fork's prompt must start with the line `Project: <absolute path of an existing project folder>`." }
    }
    return next({ ...e, cwd: project[1], prompt: withLauncher(e.prompt, e.parentAgentId) })
  }).catch(($, e, next) =>
    next.called ? next(e) : { deny: 'The qol-switchboard plugin could not start this fork in its project folder.' },
  )

  on('agent.spawn', { subagentType: FORK }, async ($, e, next) => {
    const started = await next(e)
    const agentId = started.agentId ?? e.resume?.agentId
    if (agentId === undefined) return started
    const isNew = e.resume === undefined
    await markWorking($, agentId, e.description, isNew ? projectOf(e.prompt) : undefined, isNew ? e.parentAgentId : undefined)
    const stopped = isNew && e.parentAgentId === undefined ? stoppedFork(e.prompt) : undefined
    if (stopped !== undefined) await succeed($, stopped, agentId)
    return started
  }).catch(($, e, next) => next(e))

  on('session.start', async ($, e, next) => {
    const started = await next(e)
    isAutoOpenPending = false
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
          "Reads the fork board, where the qol-switchboard plugin files every fork's final report. With no fork, one line per fork: key-name, the first sentence of its last report, and whether it is working, needs the user, is waiting on its pull request or is done. With fork set to a key-name or agent id, that fork's full last report.",
        inputSchema: {
          type: 'object',
          properties: { fork: { type: 'string', description: "A fork's key-name or agent id; leave it out for every row." } },
        },
        isDeferred: false,
      })
      await refreshStatus($)
      isAutoOpenPending = e.isInteractive && (await $.store.get(CLOSED_KEY)) !== true
      $.clock.every(MINUTE, () => $.ui.invalidate('ui.render'))
    } catch {}
    return started
  })

  on('ui.render', { component: 'AbovePrompt' }, ($, e, next) => {
    const docks = e.viewport?.isFullscreen
    if (isAutoOpenPending && docks !== undefined) {
      isAutoOpenPending = false
      if (docks) void $.ui.open({ id: PANE, title: TITLE }).catch(() => {})
    }
    return next(e)
  })

  on('command.run', { command: PANE }, async $ => {
    isAutoOpenPending = false
    return { text: await toggleBoard($) }
  }).catch(() => ({ text: 'The fork board could not open.' }))

  on('prompt.submit', async ($, e, next) => {
    if (e.command !== undefined || e.isMeta === true || e.origin.kind !== 'composer' || !TYPED_TOGGLE.test(e.text)) return next(e)
    isAutoOpenPending = false
    clearTypedToggle($)
    return { drop: await toggleBoard($) }
  }).catch(($, e, next) => (next.called ? next(e) : { drop: 'The fork board could not open.' }))

  on('ui.close', async ($, e, next) => {
    const closed = await next(e)
    if (e.id === PANE && e.origin?.kind === 'person') await rememberClosed($)
    return closed
  })

  on('tool.call', { tool: TOOL_NAME }, async ($, e) => {
    const fork = typeof e.fork === 'string' ? e.fork : undefined
    return { result: readText(await read($, board), fork) }
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

  on('session.send', async ($, e, next) => {
    if (e.agentId === undefined) return next(e)
    return mayMessage(await read($, board), e.agentId, e.to) ? next(e) : { isDelivered: false, reason: FORK_TO_FORK }
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
    const heading = (text: string) => (
      <Box marginTop={1}>
        <Text dimColor bold>
          {text}
        </Text>
      </Box>
    )

    const keys = isFocused ? 'digit opens a fork · tab walks · x closes · esc back to the prompt' : 'ctrl+x tab to use the board · qols board or ctrl+x x closes it'
    const summary = needs.length === 0 ? 'Nothing needs you.' : `${needs.length} need${needs.length === 1 ? 's' : ''} you.`
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
        <Text>{summary}</Text>
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
          <Box key={`work-${entry.agentId}`}>{title(entry, 'working')}</Box>
        ))}
        {waiting.length > 0 && heading('WAITING ON A PULL REQUEST')}
        {waiting.map(entry => (
          <Box key={`wait-${entry.agentId}`} flexDirection="column">
            {title(entry, 'waiting')}
            <Box paddingLeft={2}>
              <Text dimColor wrap="wrap">
                {entry.headline}
              </Text>
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
              <Box paddingLeft={2}>
                <Text dimColor wrap="wrap">
                  {entry.headline}
                </Text>
              </Box>
            </Box>
          ))}
      </Box>
    )
  })
}

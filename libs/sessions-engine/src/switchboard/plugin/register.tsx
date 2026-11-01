import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { Entry, Kind } from '../types'

const PROJECT = /^\s*Project:\s*(.+?)\s*$/
const FORK = 'project-fork'
const PANE = 'board'
const TITLE = 'Fork board'
const TOOL = 'fork_board'
const TOOL_NAME = 'mcp__qol-switchboard__fork_board'
const STORE_PREFIX = 'board:'
const INTAKE_DOORS: readonly string[] = ['prompt', 'delivery', 'attachment']
const KEEP = 100
const REPORT_CHARS = 16_000
const HAIKU_MS = 20_000
const MINUTE = 60_000
const KINDS: readonly Kind[] = ['ask', 'test', 'failed', 'blocked', 'waiting', 'merged', 'note']
const NEEDS_YOU: readonly Kind[] = ['ask', 'test', 'failed', 'blocked']
const DONE: readonly Kind[] = ['merged', 'note']
const HEADER = new RegExp(`^[\\s*_\`#>-]*(${KINDS.join('|')})[*_\`]*\\s*:[*_\`]*\\s*(.+?)[\\s*_\`]*$`, 'i')
const RECOMMEND = /^[\s*_`>-]*Recommend(?:ed)?(?: answer)?[*_`]*\s*:[*_`]*\s*(.+?)[\s*_`]*$/im
const HANDBACK = /(<agent-message from="([^"]+)">\n\[Subagent hand-back\][^\n]*?The report follows:\n)([\s\S]*?)(\n<\/agent-message>)/g
const NOTIFICATION = /<task-notification>[\s\S]*?<\/task-notification>/g
const RESULT = /(<result>)([\s\S]*?)(<\/result>)/
const HANDED_BACK = /delivered to you as a message from/
const AGENT_SUMMARY = /^Agent "([\s\S]*)" /
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

type Header = { kind: Kind; headline: string; recommend?: string }
type Report = { agentId: string; name?: string; status: string; report: string }

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

const markWorking = async ($: EngineInterface, agentId: string, name: string | undefined): Promise<void> => {
  const now = await $.clock.now()
  await change($, list =>
    upsert(list, agentId, entry =>
      entry === undefined
        ? { agentId, name: name ?? agentId, headline: '', report: '', at: now, isWorking: true, isDismissed: false }
        : { ...entry, name: name || entry.name, at: now, isWorking: true, isDismissed: false },
    ),
  )
}

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

export const intake = async ($: EngineInterface, text: string, known: (agentId: string) => boolean): Promise<string | undefined> => {
  let changed = false
  let out = text
  for (const [whole, head = '', agentId = '', body = '', tail = ''] of [...text.matchAll(HANDBACK)]) {
    const entry = await file($, { agentId, status: 'completed', report: dedent(body).trim() })
    out = out.replace(whole, `${head}  ${boardLine(entry).split('\n').join('\n  ')}${tail}`)
    changed = true
  }
  const blocks = [...out.matchAll(NOTIFICATION)].map(match => match[0])
  for (const block of blocks) {
    const agentId = field(block, 'task-id')
    const status = field(block, 'status') ?? ''
    const name = AGENT_SUMMARY.exec(field(block, 'summary') ?? '')?.[1]
    if (agentId === undefined || (name === undefined && !known(agentId))) continue
    const result = RESULT.exec(block)
    const report = result?.[2]?.trim() ?? ''
    if (HANDED_BACK.test(report)) continue
    const entry = await file($, { agentId, name, status, report })
    const line = boardLine(entry)
    out = out.replace(block, result === null ? block.replace('</task-notification>', `<result>${line}</result>\n</task-notification>`) : block.replace(RESULT, `$1${line}$3`))
    changed = true
  }
  return changed ? out : undefined
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
    .map(entry => `${entry.isWorking ? `[fork board] ${keyName(entry.name)} · working` : boardLine(entry)} (agent id ${entry.agentId})`)
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
    if (agentId !== undefined) await markWorking($, agentId, e.description)
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
      if (e.isInteractive) void $.ui.open({ id: PANE, title: TITLE })
      $.clock.every(MINUTE, () => $.ui.invalidate('ui.render'))
    } catch {}
    return started
  })

  on('command.run', { command: PANE }, async $ => {
    const opened = await $.ui.open({ id: PANE, title: TITLE, focus: true })
    return { text: opened.isPlaced ? 'Fork board opened.' : `The fork board waits for room: ${opened.reason}` }
  }).catch(() => ({ text: 'The fork board could not open.' }))

  on('tool.call', { tool: TOOL_NAME }, async ($, e) => {
    const fork = typeof e.fork === 'string' ? e.fork : undefined
    return { result: readText(await read($, board), fork) }
  }).catch(() => ({ deny: 'The fork board could not be read.' }))

  on('turn.start', async ($, e, next) => {
    if (e.agentId !== undefined) {
      const list = await read($, board)
      if (list.some(entry => entry.agentId === e.agentId && !entry.isWorking)) await markWorking($, e.agentId, undefined)
    }
    return next(e)
  }).catch(($, e, next) => next(e))

  on('session.append', async ($, e, next) => {
    if (e.agentId !== undefined || !INTAKE_DOORS.includes(e.door)) return next(e)
    const content = e.message.content
    const text = textOf(content)
    if (!text.includes('<task-notification>') && !text.includes('[Subagent hand-back]')) return next(e)
    const list = await read($, board)
    const rewritten = await intake($, text, agentId => list.some(entry => entry.agentId === agentId))
    if (rewritten === undefined) return next(e)
    return next({ ...e, message: { ...e.message, content: [{ type: 'text', text: rewritten }] } })
  }).catch(($, e, next) => next(e))

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const list = (await read($, board)).filter(entry => !entry.isDismissed)
    const isOpen = await read($, isDoneOpen)
    const now = await $.clock.now()
    const viewed = e.props.view?.agentId
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
    const send = (entry: Entry) => async () => {
      const sent = await $.session.send({ to: { agentId: entry.agentId }, text: entry.recommend ?? '' })
      if (!sent.isDelivered) {
        $.ui.toast(`Not sent to ${keyName(entry.name)}: ${sent.reason}`)
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

    if (list.length === 0) {
      return (
        <Box flexDirection="column">
          <Text dimColor>No forks yet. Each fork's report lands here as one line.</Text>
        </Box>
      )
    }
    return (
      <Box flexDirection="column">
        <Text>
          {needs.length === 0 ? 'Nothing needs you.' : `${needs.length} need${needs.length === 1 ? 's' : ''} you.`}{' '}
          <Text dimColor>Digit opens a fork · tab walks · esc back</Text>
        </Text>
        {needs.length > 0 && heading('NEEDS YOU')}
        {needs.map(entry => (
          <Box key={`need-${entry.agentId}`} flexDirection="column">
            {title(entry, entry.kind ?? 'note')}
            <Box paddingLeft={2} flexDirection="column">
              <Text wrap="wrap">{entry.headline}</Text>
              {entry.recommend !== undefined && <Text dimColor wrap="wrap">Recommend: {entry.recommend}</Text>}
              <Box flexDirection="row" gap={2}>
                {entry.recommend !== undefined && (
                  <Button key={`send-${entry.agentId}`} plain {...(entry === firstAsk ? { hotkey: 'y' } : {})} onPress={send(entry)}>
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

import { expect, test } from 'claude-code/testing'

const PANE = {
  plugin: 'qol-switchboard',
  component: 'Pane',
  requestId: 'board',
  props: {
    title: 'Fork board',
    isFocused: true,
    bodyColumns: 60,
    placement: 'dock',
    scroll: { offset: 0, bodyRows: 40 },
    view: {},
  },
} as const

const SURFACES = ['terminal', 'desktop'] as const

const notification = (agentId: string, name: string, result: string) =>
  `<task-notification>\n<task-id>${agentId}</task-id>\n<tool-use-id>toolu_1</tool-use-id>\n<status>completed</status>\n<summary>Agent "${name}" finished</summary>\n<note>A task-notification fires each time this agent stops.</note>\n<result>${result}</result>\n<usage><subagent_tokens>10</subagent_tokens></usage>\n</task-notification>`

const handback = (agentId: string, report: string) =>
  `Another Claude session sent a message:\n<agent-message from="${agentId}">\n[Subagent hand-back] The text below is the final report of a subagent. The report follows:\n${report
    .split('\n')
    .map(line => `  ${line}`)
    .join('\n')}\n</agent-message>\n\nThat "other Claude session" is an agent working inside this same session.`

const deliver = async ($: any, text: string, kind: 'task-notification' | 'peer' = 'task-notification') => {
  const stored = await $.session.append({
    message: { type: 'user', role: 'user', isMeta: true, content: [{ type: 'text', text }] },
    door: 'prompt',
    origin: { kind },
    uuid: `row-${Math.random()}`,
  })
  return stored.message.content.map((block: { text?: string }) => block.text ?? '').join('')
}

const engine = (on: any, sent: unknown[] = []) => {
  const store = new Map<string, unknown>()
  on('session.start', (_: unknown, e: { cwd: string }) => ({ cwd: e.cwd }))
  on('session.id', () => ({ value: 'switchboard-session' }))
  on('store.get', (_: unknown, e: { key: string }) => ({ value: store.get(e.key) }))
  on('store.set', (_: unknown, e: { key: string; value: unknown }) => {
    store.set(e.key, e.value)
    return { value: undefined }
  })
  on('store.keys', () => ({ value: [...store.keys()] }))
  on('store.delete', (_: unknown, e: { key: string }) => {
    store.delete(e.key)
    return { value: undefined }
  })
  on('ui.status', () => ({ value: undefined }))
  on('ui.toast', () => ({ value: undefined }))
  on('clock.every', () => ({ value: undefined }))
  on('clock.now', () => ({ value: 1_800_000_000_000 }))
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('ui.notify', () => ({ value: { isSent: false, reason: 'no-surface' } }))
  on('command.register', () => ({ value: { command: 'board' } }))
  on('tool.register', () => ({ value: { tool: 'mcp__qol-switchboard__fork_board' } }))
  on('session.send', (_: unknown, e: unknown) => {
    sent.push(e)
    return { isDelivered: true }
  })
  on('model.complete', () => ({ value: { isAnswered: true, text: 'merged: #77 the icon fix landed', usage: {} } }))
}

test('a headed report reaches the model as one board line and the pane lists it under Needs you', async ($, on) => {
  const sent: unknown[] = []
  engine(on, sent)
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const report = 'ask: Build artifacts, block hook and block stats?\nRecommend: build all three, in that order\n\nLong body the model should not read.'
  const read = await deliver($, notification('a1', 'cc-agent-list-polish: agent list issues', report))

  expect(read).toContain('<result>[fork board] cc-agent-list-polish · ask · Build artifacts, block hook and block stats?\nRecommend: build all three, in that order</result>')
  expect(read).toContain('<task-id>a1</task-id>')
  expect(read).not.toContain('Long body')

  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ ...PANE, surface })
    expect((await ui.find({ type: 'Text', text: /NEEDS YOU/ }))?.text).toBe('NEEDS YOU')
    expect(await ui.find({ type: 'Text', text: /Recommend: build all three/ })).toBeDefined()
    await ui.unmount()
  }

  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
  await ui.press({ key: 'send-a1' })
  expect(sent).toHaveLength(1)
  expect(JSON.stringify(sent[0])).toContain('build all three, in that order')
  expect(await ui.find({ type: 'Text', text: /NEEDS YOU/ })).toBeUndefined()
  expect(await ui.find({ type: 'Text', text: /WORKING/ })).toBeDefined()
  await ui.unmount()
})

test('a hand-back report is rewritten in place and a report without a header gets one from Haiku', async ($, on) => {
  engine(on)
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const read = await deliver($, handback('a2', 'PR #77 merged.\nIt fixes the icon.'), 'peer')

  expect(read).toContain('The report follows:\n  [fork board] a2 · merged · #77 the icon fix landed\n</agent-message>')
  expect(read).not.toContain('It fixes the icon.')

  const ui = await $.ui.mount({ ...PANE, surface: 'terminal' })
  expect(await ui.find({ type: 'Text', text: /Nothing needs you/ })).toBeDefined()
  await ui.press({ key: 'done-toggle' })
  expect(await ui.find({ type: 'Text', text: /#77 the icon fix landed/ })).toBeDefined()
  await ui.unmount()
})

test('notifications of other background tasks and plain agent messages pass unchanged', async ($, on) => {
  engine(on)
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  const shell = '<task-notification>\n<task-id>b9</task-id>\n<status>completed</status>\n<summary>Background command "sleep" completed</summary>\n</task-notification>'
  expect(await deliver($, shell)).toBe(shell)
  const plain = 'Another Claude session sent a message:\n<agent-message from="a3">\nquick question\n</agent-message>'
  expect(await deliver($, plain, 'peer')).toBe(plain)
})

test('fork_board lists every row and returns one fork in full', async ($, on) => {
  engine(on)
  await $.session.start({ cwd: '/', surface: 'terminal', isInteractive: true })
  await deliver($, notification('a4', 'kcd2-modding: continue KCD2 modding', 'test: Hip light fix is live; 3 steps to try\n\n1. Walk backwards.'))

  const all = await $.tool.call({ tool: 'mcp__qol-switchboard__fork_board' })
  expect(String(all.result)).toContain('[fork board] kcd2-modding · test · Hip light fix is live; 3 steps to try (agent id a4)')
  const one = await $.tool.call({ tool: 'mcp__qol-switchboard__fork_board', fork: 'kcd2-modding' })
  expect(String(one.result)).toContain('1. Walk backwards.')
})

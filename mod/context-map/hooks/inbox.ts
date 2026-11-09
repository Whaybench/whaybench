// Prompts Whaybench queued for this session (the input panel, Whaybench buttons), submitted in order once each.
import type { EngineInterface } from 'claude-code'

import { logLine } from './log'
import { INBOX } from './paths'
import { touchSession } from './session-record'

type Inbound = { seq: number; text: string }

// Submits what Whaybench queued for this session since the last pass.
export async function drainInbox($: EngineInterface, sessionId: string) {
  const path = `${INBOX}/${sessionId}.json`
  if (!(await $.fs.exists(path))) return
  const queued = JSON.parse(String(await $.fs.read(path))) as Inbound[]
  const done = ((await $.store.get(`inbox:${sessionId}`)) as number | undefined) ?? 0
  const fresh = queued.filter(m => m.seq > done).sort((a, b) => a.seq - b.seq)
  if (!fresh.length) return
  await $.store.set(`inbox:${sessionId}`, fresh[fresh.length - 1].seq)
  for (const m of fresh) {
    // Our own submits skip our prompt.submit hook, so record the prompt here.
    await touchSession($, m.text.replace(/\s+/g, ' ').slice(0, 200)).catch(() => undefined)
    await $.prompt.submit({ text: m.text, asUser: true })
    const tag = m.text.startsWith('[') && m.text.indexOf(']') > 0 && m.text.indexOf(']') < 40 ? m.text.slice(1, m.text.indexOf(']')) : m.text.startsWith('/') ? `command ${m.text.split(/\s/)[0]}` : "person's input"
    await logLine($, sessionId, 'decide', 'request', 'request submitted', undefined, { request: m.seq, kind: tag, seconds_after: Math.round(Date.now() / 1000 - m.seq / 1e6) })
  }
}

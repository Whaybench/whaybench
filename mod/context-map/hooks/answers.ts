// Each turn's whole answer for Whaybench's answer panel, and the turn count the map's stamps use.
import type { EngineInterface, Register } from 'claude-code'

import { writeMap } from './map-data'
import { TRANSCRIPTS } from './paths'
import { record } from './session-record'

type On = Parameters<Register>[0]

export async function wholeTurn($: EngineInterface, last: string): Promise<string> {
  const dir = (await $.session.cwd()).replace(/[^A-Za-z0-9]/g, '-')
  const file = `${TRANSCRIPTS}/${dir}/${await $.session.id()}.jsonl`
  const read = await $.process.run(['python3', '-c', [
    'import json,sys',
    'rows=[]',
    'for l in open(sys.argv[1]):',
    '  try: rows.append(json.loads(l))',
    '  except ValueError: pass',
    'def asked(d):',
    '  c=(d.get("message") or {}).get("content")',
    '  if d.get("type")!="user" or d.get("isMeta"): return False',
    '  if isinstance(c,str): return not c.startswith("<")',
    '  return isinstance(c,list) and any(x.get("type")=="text" for x in c) and not any(x.get("type")=="tool_result" for x in c)',
    'start=max([i for i,d in enumerate(rows) if asked(d)] or [0])',
    'out=[]',
    'for d in rows[start+1:]:',
    '  c=(d.get("message") or {}).get("content")',
    '  if d.get("type")=="assistant" and isinstance(c,list):',
    '    t="".join(x.get("text","") for x in c if x.get("type")=="text").strip()',
    '    if t: out.append(t)',
    'print("\\n\\n".join(out))',
  ].join('\n'), file]).catch(() => null)
  const whole = read?.stdout.trim() ?? ''
  if (!whole) return last
  return whole.endsWith(last.trim()) ? whole : `${whole}\n\n${last.trim()}`
}

// At the end of a turn: count it, and keep its answer.
export function answersHooks(on: On) {
  on('turn.complete', async ($, e, next) => {
    // The answer comes back from next(e), not on the event.
    const result = await next(e)
    const last = (result as { text?: string }).text
    // Count finished turns (any input path: typed, input panel, Whaybench buttons), so the map's stamps can
    // say in which turn a node opened or changed. A subagent's turn is not the conversation's.
    if (!(e as { agentId?: string }).agentId) await writeMap($, await $.session.id(), null, '--end-turn').catch(() => undefined)
    // The result holds only the turn's last block of text; when tools ran in between (a map
    // update before a closing line), the answer is all of the turn's text, read from the transcript.
    const text = !(e as { agentId?: string }).agentId && last?.trim() ? await wholeTurn($, last).catch(() => last) : last
    if (!(e as { agentId?: string }).agentId && text?.trim()) {
      // The prompt is the record's last one, which whaybench record fills in.
      await record($, await $.session.id(), ['answer'], JSON.stringify({ text })).catch(() => undefined)
    }
    return result
  })
}

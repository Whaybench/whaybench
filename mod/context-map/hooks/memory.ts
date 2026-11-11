// What Claude is told from the memory (all sessions' maps): the whole memory list when the session starts and
// after the conversation was compacted, and only what changed on later turns, so the list does not pile up.
import type { EngineInterface, Register } from 'claude-code'

import { SHARE_DIR, WHAYBENCH } from './paths'

type On = Parameters<Register>[0]

// The next prompt gets the whole list: at start, and once a compaction dropped the last one.
let wholeDue = true

export function memoryHooks(on: On) {
  on('classic.PreCompact', async ($, e, next) => {
    wholeDue = true
    return next(e)
  })
}

export async function memoryNote($: EngineInterface): Promise<string[]> {
  const id = await $.session.id()
  const whole = wholeDue
  const run = await $.process.run([WHAYBENCH, 'memory', '--data-dir', SHARE_DIR, whole ? 'list' : 'changes', id]).catch(() => null)
  if (!run || run.exitCode !== 0) return []
  wholeDue = false
  const lines = run.stdout.trim()
  if (!lines) return []
  return [whole
    ? 'Memory list: your open nodes, then related and recent nodes of other sessions (memory id · label · state or kind · session · last change). ' +
      'Give a node about the same problem as one here its memory id (memid); when a decision settles one, link it with resolves. ' +
      'An open node of yours that looks finished or no longer needed (for one, a node elsewhere with its memory id settled it): ' +
      'close it if sure, else give it suggest; the person approves it in the alerts panel, so no need to ask first. ' +
      '`whaybench memory show <memid>` for detail, `whaybench memory search <words>` for more.\n' + lines
    : 'Memory changes since your last turn (type · by · node · change). If one makes an open node of yours finished or ' +
      'no longer needed, close it if sure, else give it suggest:\n' + lines]
}

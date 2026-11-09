// The session's record for Whaybench: state, heartbeat owner, prompts, /background and /resume moves.
import type { EngineInterface, Register } from 'claude-code'

import { inheritMap } from './map-data'
import { FORKS, SESSIONS, SHARE_DIR, TRANSCRIPTS, WHAYBENCH } from './paths'

type On = Parameters<Register>[0]
type ForkNote = { from: string; cwd: string; at: number }

type SessionState = 'working' | 'waiting' | 'done'
let lastState: SessionState | undefined
// Set by /rewind, cleared by the next prompt, which tells Claude the map may be ahead of the conversation.
let rewound = false
export function takeRewound() {
  const was = rewound
  rewound = false
  return was
}
export const REWIND_NOTE =
  'The conversation was rewound (/rewind) since your last reply. The conversation map was not: nodes added, ' +
  'closed or decided after the point rewound to may no longer hold. Check the map (show: true); mark nodes whose ' +
  'result changed with state "changed", reopen questions that are unsettled again, and remove nodes for work that no longer exists.'

// The session's record, state, heartbeat and answers are written by `whaybench record`,
// for every agent alike; a failure throws with its reason.
export async function record($: EngineInterface, id: string, args: string[], stdin?: string) {
  const run = await $.process.run([WHAYBENCH, 'record', '--data-dir', SHARE_DIR, id, ...args], stdin === undefined ? {} : { stdin })
  if (run.exitCode !== 0) throw new Error(run.stderr.replace(/^whaybench: /, '').trim() || `whaybench record exited ${run.exitCode}`)
}

export async function setState($: EngineInterface, state: SessionState, note?: string) {
  if (state === lastState && !note) return
  lastState = state
  await record($, await $.session.id(), ['state', state, ...(note ? ['--note', note] : [])]).catch(() => undefined)
}

// What Claude changed in a file, before and after, so Whaybench can show it. Capped: large

export async function hasTranscript($: EngineInterface, id: string) {
  const dir = (await $.session.cwd()).replace(/[^A-Za-z0-9]/g, '-')
  const found = await $.process.run(['bash', '-c', `test -s ${TRANSCRIPTS}/${dir}/${id}.jsonl && echo yes`]).catch(() => null)
  return found?.stdout.trim() === 'yes'
}

// Returns whether this session took over the noted one.
export async function adoptFork($: EngineInterface, id: string): Promise<boolean> {
  if (!(await $.fs.exists(FORKS))) return false
  const note = JSON.parse(String(await $.fs.read(FORKS)) || '{}') as Partial<ForkNote>
  // The folder can differ when the old session had moved into a subfolder (cd) before /background:
  // one inside the other counts as the same place.
  const cwd = await $.session.cwd()
  const near = (a = '', b = '') => a === b || a.startsWith(b.replace(/\/$/, '') + '/') || b.startsWith(a.replace(/\/$/, '') + '/')
  if (!note.from || note.from === id || Date.now() - (note.at ?? 0) > 60_000 || !near(note.cwd, cwd)) return false
  // Claude Code keeps spare processes ready for the next /background; they start with an id but no
  // conversation, and must not take the note meant for the session that carries the conversation on.
  if (!(await hasTranscript($, id))) return false
  await $.fs.write(FORKS, '{}')
  await inheritMap($, id, note.from)
  await record($, id, ['carry-from', note.from])
  return true
}

// Set when this Claude runs in a Whaybench tab, so Whaybench can tell which tab holds which session.
let whaybenchTab: string | undefined
export function setTab(tab?: string) {
  whaybenchTab = tab
}
// /resume of a session the supervisor runs in the background makes this process only a window
// onto that session; this session id then stands for nothing, so it stops looking alive.
let shownElsewhere = false
export function isShownElsewhere() {
  return shownElsewhere
}

async function noteResumedElsewhere($: EngineInterface) {
  const me = await $.session.id()
  const dir = (await $.session.cwd()).replace(/[^A-Za-z0-9]/g, '-')
  // Claude Code writes a notice row in the transcript when it opens a session that runs in the background.
  // Only that row counts (a system local_command row from the last minute): the same words can also
  // appear in the conversation itself.
  const found = await $.process.run(['python3', '-c', [
    'import json,sys,re,datetime',
    'hit=""',
    'for l in open(sys.argv[1]):',
    '  if "running in the background (" not in l: continue',
    '  try: d=json.loads(l)',
    '  except ValueError: continue',
    '  if d.get("type")!="system" or d.get("subtype")!="local_command": continue',
    '  c=str(d.get("content",""))',
    '  m=re.match(r"<local-command-stdout>Opening .*running in the background \\(([0-9a-f]{8})\\)",c)',
    '  if not m: continue',
    '  t=datetime.datetime.fromisoformat(d.get("timestamp","1970-01-01T00:00:00+00:00").replace("Z","+00:00"))',
    '  if (datetime.datetime.now(datetime.timezone.utc)-t).total_seconds()<90: hit=m.group(1)',
    'print(hit)',
  ].join('\n'), `${TRANSCRIPTS}/${dir}/${me}.jsonl`]).catch(() => null)
  const short = found?.stdout.trim().match(/^([0-9a-f]{8})$/)?.[1]
  if (!short) return
  const full = await $.process.run(['bash', '-c', `ls ${SESSIONS} | grep '^${short}' | head -1`]).catch(() => null)
  const target = full?.stdout.trim().replace(/\.json$/, '')
  if (!target || target === me) return
  shownElsewhere = true
  // Claude Code also carries the session this tab was in on in the background ("continued-in").
  // For an empty new tab that is a background session with no conversation. It is named in the
  // record with the move, and Whaybench stops it: Whaybench soon closes this tab's process, which
  // could cut a stop started here short.
  const next = await $.process.run(['bash', '-c',
    `grep -ao '"continuedInSessionId":"[0-9a-f-]*"' ${TRANSCRIPTS}/${dir}/${me}.jsonl | tail -1`]).catch(() => null)
  const carried = next?.stdout.match(/"continuedInSessionId":"([0-9a-f-]+)"/)?.[1]
  const empty = carried && carried !== target && !(await hasTranscript($, carried))
  await record($, me, ['moved', target, ...(empty ? ['--empty-carried', carried] : [])])
}

export async function touchSession($: EngineInterface, prompt?: string) {
  const args = ['touch', '--cwd', await $.session.cwd(), ...(prompt ? ['--prompt', prompt] : []), ...(whaybenchTab ? ['--tab', whaybenchTab] : [])]
  await record($, await $.session.id(), args)
}

// State hooks and the commands that move a session (/rewind, /background, /resume).
export function sessionRecordHooks(on: On) {
  // Session state for Whaybench: a prompt or a tool run means working, a question or permission
  // prompt means waiting on the person, the end of a turn means done.
  on('classic.UserPromptSubmit', async ($, e, next) => {
    await setState($, 'working')
    return next(e)
  })
  on('classic.Notification', async ($, e, next) => {
    await setState($, e.notification_type === 'idle_prompt' ? 'done' : 'waiting', e.message)
    return next(e)
  })
  on('classic.Stop', async ($, e, next) => {
    await setState($, 'done')
    return next(e)
  })
  // A session can end mid-turn (/exit, Esc then /exit, Whaybench stopping it): no Stop comes, so
  // the last state would stay "working" for good. The end of the session settles it.
  on('classic.SessionEnd', async ($, e, next) => {
    await setState($, 'done', 'ended')  // a note makes it write even when the last state was done
    return next(e)
  })
  on('tool.call', async ($, e, next) => {
    const result = await next(e)
    if (lastState === 'waiting') await setState($, 'working')  // the person answered the prompt
    return result
  })

  on('command.run', async ($, e, next) => {
    const name = String((e as { command?: string }).command ?? '')
    // /rewind rolls the conversation back; the map does not roll back with it, so the next prompt
    // carries a note asking to set the map straight.
    if (name === 'rewind' || name === 'checkpoint' || name === 'undo') rewound = true
    if (name === 'background' || name === 'bg') {
      const note: ForkNote = { from: await $.session.id(), cwd: await $.session.cwd(), at: Date.now() }
      await $.fs.write(FORKS, JSON.stringify(note)).catch(() => undefined)
    }
    const result = await next(e)
    if (name === 'resume') {
      // The picker finishes after the command returns; look a few times for where it went.
      void (async () => {
        for (let i = 0; i < 6 && !shownElsewhere; i++) {
          await $.process.run(['sleep', '2']).catch(() => undefined)
          await noteResumedElsewhere($).catch(() => undefined)
        }
      })()
    }
    return result
  })
}

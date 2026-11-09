// Debug record: one file per session id, logs/sessions/<id>.log, lines in Whaybench's format
//   <HH:MM:SS.mmm> [<level>] [<part>] <event> · pid=<n> <field>=<value> … · why: <why>
// so Whaybench's log view can merge them with its own run file by time. Writing never throws.
import type { EngineInterface } from 'claude-code'

import { LOG_DETAIL, LOGS_SESSIONS } from './paths'

type Fields = Record<string, string | number | boolean | string[] | undefined | null>

let lastDay = ''
const headed = new Set<string>()

function pad(n: number, w = 2) {
  return String(n).padStart(w, '0')
}

function day(d: Date) {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

function value(v: Fields[string]): string {
  if (v === undefined || v === null) return '-'
  if (Array.isArray(v)) return v.map(x => value(x)).join(',')
  let s = String(v).replace(/\n/g, ' ')
  if (/^[0-9a-f]{8}-[0-9a-f-]{27}$/.test(s)) s = s.slice(0, 8)
  return s.includes(' ') ? `'${s}'` : s
}

async function detailOn($: EngineInterface, part: string) {
  try {
    const parts = (JSON.parse(String(await $.fs.read(LOG_DETAIL))) as { parts?: string[] }).parts ?? []
    return parts.includes(part)
  } catch {
    return false
  }
}

// Appends one line (with the file's header first when the file is new, and a date line when the day
// turned). bash appends with O_APPEND, so lines of two processes on one session never break each other.
export async function logLine($: EngineInterface, sessionId: string, level: 'error' | 'warn' | 'decide' | 'detail', part: string,
  event: string, why?: string, fields: Fields = {}) {
  try {
    if (!sessionId) return
    if (level === 'detail' && !(await detailOn($, part))) return
    const now = new Date()
    const t = `${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}.${pad(now.getMilliseconds(), 3)}`
    const extra = Object.entries(fields).filter(([, v]) => v !== undefined).map(([k, v]) => `${k}=${value(v)}`).join(' ')
    let text = `${t} [${level}] [${part}] ${event} · pid=PID${extra ? ' ' + extra : ''}`
    if (why) text += ` · why: ${why.replace(/\n/g, ' ')}`
    if (text.length > 2000) text = text.slice(0, 2000) + '…(cut)'
    const path = `${LOGS_SESSIONS}/${sessionId}.log`
    let lead = ''
    if (!headed.has(sessionId)) {
      headed.add(sessionId)
      if (!(await $.fs.exists(path))) lead = await headerText($, sessionId, now)
    }
    if (lastDay !== day(now)) {
      if (!lead) lead = `# date ${day(now)}\n`
      lastDay = day(now)
    }
    await $.process.run(['bash', '-c', 'mkdir -p "$(dirname "$2")" && printf "%s%s\\n" "$3" "${1//PID/$PPID}" >> "$2"', 'log', text, path, lead])
  } catch {
    // a record that cannot be written must not stop the session
  }
}

async function headerText($: EngineInterface, sessionId: string, now: Date) {
  const v = await $.session.version().catch(() => null)
  const commit = (await $.process.run(['git', '-C', $.plugin.root, 'log', '-1', '--format=%h %cd', '--date=format:%m-%d %H:%M']).catch(() => null))?.stdout.trim()
  const env = async (name: string) => (await $.process.run(['printenv', name]).catch(() => null))?.stdout.trim() || '-'
  const t = `${day(now)} ${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`
  return [
    `# session log · started ${t} · session=${sessionId}`,
    `# versions plugin ${commit || 'unknown'} · Claude Code ${v?.version ?? 'unknown'}`,
    `# environment in_whaybench=${(await env('WHAYBENCH')) === '1' ? 'yes' : 'no'} tab=${await env('WHAYBENCH_TAB')}`,
  ].join('\n') + '\n'
}

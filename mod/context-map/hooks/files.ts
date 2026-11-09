// Files Claude read or changed, with the node the conversation was on; Claude's edits; outside changes.
import type { EngineInterface, Register } from 'claude-code'

import { decisionsFor, toldDecisions } from './decisions'
import { currentNode } from './map-data'
import { FILES } from './paths'

type On = Parameters<Register>[0]
export type FileTouch = { path: string; op: 'read' | 'edit'; at: number; node?: string }

const EDITS_KEPT = 200
const TEXT_KEPT = 40_000
type FileEdit = { path: string; at: number; node?: string; before: string; after: string; whole: boolean }

export async function noteEdit($: EngineInterface, edit: Omit<FileEdit, 'at' | 'node'>, node?: string) {
  const id = await $.session.id()
  const file = `${FILES}/${id}.edits.json`
  const old = (await $.fs.exists(file)) ? (JSON.parse(String(await $.fs.read(file))) as FileEdit[]) : []
  const cut = (text: string) => (text.length > TEXT_KEPT ? text.slice(0, TEXT_KEPT) + '\n…' : text)
  const entry: FileEdit = { ...edit, before: cut(edit.before), after: cut(edit.after), at: Date.now(), node }
  await $.fs.write(file, JSON.stringify([...old, entry].slice(-EDITS_KEPT)))
}

export async function noteFile($: EngineInterface, path: string, op: FileTouch['op']) {
  const id = await $.session.id()
  const file = `${FILES}/${id}.json`
  const old = (await $.fs.exists(file)) ? (JSON.parse(String(await $.fs.read(file))) as FileTouch[]) : []
  // The shared copy of the map, not the atom: outside drawing the atom can still hold its first value.
  const touch: FileTouch = { path, op, at: Date.now(), node: await currentNode($) }
  await $.fs.write(file, JSON.stringify([...old, touch].slice(-1000)))
}

// Files Claude read or changed in this session that changed on disk afterwards: the person (or
// another program) saved them. Each change is told once, when it is found.
export const toldChange = new Map<string, number>()  // path -> mtime already told
// Claude also changes files through Bash (sed, scripts); a change made while one of its commands
// ran is its own.
export const bashRuns: [number, number][] = []

export async function changedSince($: EngineInterface, only?: string): Promise<{ path: string; at: number }[]> {
  const id = await $.session.id()
  const file = `${FILES}/${id}.json`
  if (!(await $.fs.exists(file).catch(() => false))) return []
  const touches = JSON.parse(String(await $.fs.read(file))) as FileTouch[]
  const last = new Map<string, number>()
  for (const t of touches) if (!only || t.path === only) last.set(t.path, Math.max(last.get(t.path) ?? 0, t.at))
  if (last.size === 0) return []
  const stat = await $.process.run(['python3', '-c',
    'import os,sys\nfor p in sys.argv[1:]:\n  try: print(int(os.stat(p).st_mtime*1000), p)\n  except OSError: pass',
    ...last.keys()]).catch(() => null)
  const found: { path: string; at: number }[] = []
  for (const line of (stat?.stdout ?? '').split('\n')) {
    const cut = line.indexOf(' ')
    if (cut < 0) continue
    const at = Number(line.slice(0, cut)), path = line.slice(cut + 1)
    // Claude's own write lands a moment before its touch is noted; a later change is someone else's.
    const own = bashRuns.some(([from, to]) => at >= from - 500 && at <= to + 1500)
    if (at > (last.get(path) ?? Infinity) + 1500 && toldChange.get(path) !== at && !own) found.push({ path, at })
  }
  return found
}

// Bash run times, so a change made while one of Claude's commands ran is told apart from the person's.
export function filesHooks(on: On) {
  on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
    const from = Date.now()
    try {
      return await next(e)
    } finally {
      bashRuns.push([from, Date.now()])
      if (bashRuns.length > 500) bashRuns.splice(0, bashRuns.length - 500)
      // Whaybench's files panel reads these too, to tell the person's changes from Claude's.
      await $.fs.write(`${FILES}/${await $.session.id()}.bash.json`, JSON.stringify(bashRuns)).catch(() => undefined)
    }
  })

}

// After every read and change: its record, Claude's edit, and the decisions that come with a file.
export function filesRecordHooks(on: On) {
  on('tool.call', { tool: ['Read', 'Edit', 'Write', 'NotebookEdit'] }, async ($, e, next) => {
    // A whole-file write keeps what was there before, read before the write replaces it.
    const before =
      e.tool === 'Write' && (await $.fs.exists(e.file_path).catch(() => false))
        ? String(await $.fs.read(e.file_path).catch(() => ''))
        : ''
    const result = await next(e)
    const path = 'notebook_path' in e ? e.notebook_path : e.file_path
    if (e.tool === 'Read' && result.deny === undefined && !result.isError && path && !toldDecisions.has(path)) {
      toldDecisions.add(path)
      const decisions = await decisionsFor($, path).catch(() => [])
      if (decisions.length) {
        await noteFile($, path, 'read').catch(() => undefined)
        return { ...result, context: [...(result.context ?? []), `Decisions recorded in the conversation map for the work ${path} belongs to ` +
          '(follow them; if the task needs something else, ask the person first):\n' + decisions.join('\n')] }
      }
    }
    if (result.deny === undefined && !result.isError && path) {
      await noteFile($, path, e.tool === 'Read' ? 'read' : 'edit').catch(() => undefined)
      const node = await currentNode($).catch(() => undefined)
      if (e.tool === 'Edit') {
        await noteEdit($, { path, before: e.old_string, after: e.new_string, whole: false }, node).catch(() => undefined)
      } else if (e.tool === 'Write') {
        await noteEdit($, { path, before, after: e.content, whole: true }, node).catch(() => undefined)
      }
    }
    return result
  })
}

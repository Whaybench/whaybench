// The conversation map itself: its values, how a change applies, taking over another session's map.
import { atom, read, update } from 'claude-code'
import type { EngineInterface } from 'claude-code'

import type { ContextMap, MapNode, OtherView, Stamp, StoredMap } from '../types'
import { PREFIX, SHARE_DIR, WHAYBENCH } from './paths'

const map = atom({ plugin: 'context-map', key: 'map' } as const, { nodes: [] } as ContextMap, {
  shape: 'map-1',
})
const other = atom({ plugin: 'context-map', key: 'other' } as const, null as OtherView, {
  shape: 'other-1',
})
// Branches whose folding the person flipped from the default by pressing them.
const toggled = atom({ plugin: 'context-map', key: 'toggled' } as const, [] as string[], {
  shape: 'toggled-1',
})

export async function currentNode($: EngineInterface) {
  const id = await $.session.id()
  const shared = `${SHARE_DIR}/${id}.json`
  const saved = (await $.fs.exists(shared))
    ? (JSON.parse(String(await $.fs.read(shared))) as StoredMap)
    : ((await $.store.get(PREFIX + id)) as StoredMap | undefined)
  return saved?.current ?? (await read($, map)).current
}

// Takes another session's map as this one's. The shared copy is read first: the store is kept per
// plugin copy, so a session run by a development copy of this mod is not found there.
export async function inheritMap($: EngineInterface, id: string, from: string) {
  const shared = `${SHARE_DIR}/${from}.json`
  const inherited = (await $.fs.exists(shared))
    ? (JSON.parse(String(await $.fs.read(shared))) as StoredMap)
    : ((await $.store.get(PREFIX + from)) as StoredMap | undefined)
  if (!inherited?.nodes) return false
  await writeMap($, id, inherited, '--take-over')
  // Whaybench follows this to show the old session's file records under the new one too.
  const links = `${SHARE_DIR}/forks/inherited.json`
  const known = (await $.fs.exists(links)) ? (JSON.parse(String(await $.fs.read(links))) as Record<string, string>) : {}
  await $.fs.write(links, JSON.stringify({ ...known, [id]: from })).catch(() => undefined)
  return true
}

export function carriedSummary(m: ContextMap, from: string) {
  const by = new Map(m.nodes.map(n => [n.id, n]))
  const path: string[] = []
  for (let n = m.current ? by.get(m.current) : undefined; n; n = n.parent ? by.get(n.parent) : undefined) path.unshift(n.label)
  const open = (n: MapNode) => n.isOpen !== false && n.state !== 'done'
  const pick = (f: (n: MapNode) => boolean, cap: number) => m.nodes.filter(f).slice(-cap)
  const questions = pick(n => open(n) && n.state !== 'plan' && n.state !== 'doing', 20)
  const later = pick(n => open(n) && n.state === 'plan', 10)
  const decided = pick(n => !open(n) && n.kind === 'conclusion', 20)
  const line = (n: MapNode) => `- ${n.label}${n.reason ? ` (reason: ${n.reason})` : ''}`
  return [
    `This session started by taking over the conversation map of ${from.startsWith('imported') ? from : `session ${from.slice(0, 8)}`}. The conversation itself did not come along: for details read the memory files or ask the user (no guessing).`,
    path.length ? `Current position: ${path.join(' › ')}` : '',
    questions.length ? `Open questions and ideas:\n${questions.map(line).join('\n')}` : '',
    later.length ? `Put-off work:\n${later.map(line).join('\n')}` : '',
    decided.length ? `Decided (recent):\n${decided.map(line).join('\n')}` : '',
  ].filter(Boolean).join('\n\n')
}

export async function storedMaps($: EngineInterface) {
  const keys = (await $.store.keys()).filter(k => k.startsWith(PREFIX))
  const maps: { sessionId: string; map: StoredMap }[] = []
  for (const key of keys) {
    const map = (await $.store.get(key)) as StoredMap | undefined
    if (map?.nodes) maps.push({ sessionId: key.slice(PREFIX.length), map })
  }
  return maps.sort((a, b) => b.map.updatedAt - a.map.updatedAt)
}

export function summary(map: ContextMap) {
  const here = map.nodes.find(n => n.id === map.current)
  const open = map.nodes.filter(n => n.isOpen).length
  return `${here ? here.label : '(no position)'} · ${open} open`
}

export type Change = Partial<MapNode> & { id: string; parent?: string }

// Every change of the map goes through `whaybench map`, which keeps the rules of a change
// for every agent: merge by id, subtrees removed together, stamps, no broken references. What it saved
// becomes this session's map; a change it refuses throws with its reason and leaves the map as it was.
export async function writeMap($: EngineInterface, sessionId: string, change: object | null, how: '--write' | '--take-over' | '--start-from' | '--end-turn' = '--write') {
  const run = await $.process.run([WHAYBENCH, 'map', '--data-dir', SHARE_DIR, sessionId, how], change ? { stdin: JSON.stringify(change) } : {})
  if (run.exitCode !== 0) throw new Error(run.stderr.replace(/^whaybench: /, '').trim() || `whaybench map exited ${run.exitCode}`)
  const saved = JSON.parse(run.stdout) as StoredMap & { current?: string | null }
  await update($, map, () => ({ nodes: saved.nodes, current: saved.current ?? undefined, turn: saved.turn, version: saved.version }))
  await $.store.set(PREFIX + sessionId, { ...saved, current: saved.current ?? undefined })
  return saved
}

// The map's values are read and written here only (the engine's scan wants each value's own file).
export function mapNow($: EngineInterface) {
  return read($, map)
}
export function changeMap($: EngineInterface, fn: (m: ContextMap) => ContextMap) {
  return update($, map, fn)
}
export function otherNow($: EngineInterface) {
  return read($, other)
}
export function setOther($: EngineInterface, view: OtherView) {
  return update($, other, () => view)
}
export function toggledNow($: EngineInterface) {
  return read($, toggled)
}
export function flipToggled($: EngineInterface, id: string) {
  return update($, toggled, list => (list.includes(id) ? list.filter(x => x !== id) : [...list, id]))
}

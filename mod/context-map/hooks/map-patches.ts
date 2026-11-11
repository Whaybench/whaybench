// Changes to the map that come from outside the conversation, through files Whaybench writes.
import type { EngineInterface } from 'claude-code'

import type { MapNode } from '../types'
import { mapNow, writeMap } from './map-data'
import type { Change } from './map-data'
import { logLine } from './log'
import { PATCHES } from './paths'

// Fields a side worker filled for a map style, outside the conversation: Whaybench's
// A map style's fill runs the map fill (whaybench-panel mapfill), which writes <session id>.json here; only these keys are taken.
const KINDS = ['question', 'option', 'pro', 'con', 'conclusion', 'task', 'appointment', 'problem', 'idea', 'fact', 'rule', 'note']
const RELS = ['cause', 'leads', 'replaces', 'related', 'resolves']

export async function applyPatches($: EngineInterface, sessionId: string) {
  const path = `${PATCHES}/${sessionId}.json`
  if (!(await $.fs.exists(path))) return
  let items: Record<string, unknown>[] = []
  try {
    items = JSON.parse(String(await $.fs.read(path))) as Record<string, unknown>[]
  } catch {
    items = []
  }
  await $.process.run(['rm', '-f', path]).catch(() => undefined)
  if (!Array.isArray(items) || !items.length) return
  const ids = new Set((await mapNow($)).nodes.map(n => n.id))
  const upsert = items.filter(p => p && typeof p.id === 'string' && ids.has(p.id)).map(p => {
    const c: Change = { id: p.id as string }
    if (typeof p.kind === 'string' && KINDS.includes(p.kind)) c.kind = p.kind as MapNode['kind']
    if (typeof p.reason === 'string' && p.reason.trim()) c.reason = p.reason.trim().slice(0, 80)
    if (Array.isArray(p.links))
      c.links = (p.links as { to?: unknown; rel?: unknown }[])
        .filter(l => l && typeof l.to === 'string' && ids.has(l.to) && typeof l.rel === 'string' && RELS.includes(l.rel))
        .map(l => ({ to: l.to as string, rel: l.rel as NonNullable<MapNode['links']>[number]['rel'] }))
    return c
  })
  if (upsert.length) await writeMap($, sessionId, { upsert })
  await logLine($, sessionId, 'decide', 'map', 'map file applied', undefined, { kind: 'fill', items: items.length })
}

// A branch or a whole map brought in from another session or an exported file (Whaybench's map
// menu): <session id>.graft.json = { parent: node id or null, nodes: [...] }. Ids get a prefix so
// they cannot clash; the time fields start over in this session (apply stamps them as opened now).
export async function applyGraft($: EngineInterface, sessionId: string) {
  const path = `${PATCHES}/${sessionId}.graft.json`
  if (!(await $.fs.exists(path))) return
  let graft: { parent?: string | null; nodes?: MapNode[] } = {}
  try {
    graft = JSON.parse(String(await $.fs.read(path)))
  } catch {
    graft = {}
  }
  await $.process.run(['rm', '-f', path]).catch(() => undefined)
  const incoming = Array.isArray(graft.nodes) ? graft.nodes.filter(n => n && typeof n.id === 'string' && typeof n.label === 'string') : []
  if (!incoming.length) return
  await logLine($, sessionId, 'decide', 'map', 'map file applied', undefined, { kind: 'import', nodes: incoming.length, under: graft.parent ?? 'top' })
  const tag = `g${Date.now().toString(36)}-`
  const ids = new Set(incoming.map(n => n.id))
  const old = await mapNow($)
  const parent = graft.parent && old.nodes.some(n => n.id === graft.parent) ? graft.parent : undefined
  // Stamps sent along are dropped by whaybench map, which stamps these nodes as opened now.
  const upsert: Change[] = incoming.map(n => {
    const c: Record<string, unknown> = { ...n, id: tag + n.id }
    c.parent = n.parent && ids.has(n.parent) ? tag + n.parent : (parent ?? '')
    if (Array.isArray(n.links)) c.links = n.links.filter(l => ids.has(l.to)).map(l => ({ ...l, to: tag + l.to }))
    return c as Change
  })
  await writeMap($, sessionId, { upsert })
}

// Deletions from Whaybench's map menu: <session id>.edit.json = { remove: [ids], lift: [ids] }.
// remove drops a node with everything under it; lift drops only the node, its children moving up
// to its parent. Whaybench keeps a copy of what was dropped before asking.
export async function applyEdit($: EngineInterface, sessionId: string) {
  const path = `${PATCHES}/${sessionId}.edit.json`
  if (!(await $.fs.exists(path))) return
  let edit: { remove?: string[]; lift?: string[] } = {}
  try {
    edit = JSON.parse(String(await $.fs.read(path)))
  } catch {
    edit = {}
  }
  await $.process.run(['rm', '-f', path]).catch(() => undefined)
  const remove = Array.isArray(edit.remove) ? edit.remove.filter(i => typeof i === 'string') : []
  const lift = Array.isArray(edit.lift) ? edit.lift.filter(i => typeof i === 'string') : []
  if (!remove.length && !lift.length) return
  // A lifted node's children move up to its parent; then the lifted and removed nodes go in one change.
  const parents = new Map((await mapNow($)).nodes.map(n => [n.id, n.parent ?? '']))
  const moved = new Set<string>()
  for (const id of lift) {
    if (!parents.has(id)) continue
    const up = parents.get(id) ?? ''
    for (const [child, p] of parents) if (p === id) parents.set(child, up), moved.add(child)
    parents.delete(id)
  }
  const upsert = [...moved].filter(id => parents.has(id)).map(id => ({ id, parent: parents.get(id) ?? '' }))
  await writeMap($, sessionId, { upsert, remove: [...lift, ...remove] })
  await logLine($, sessionId, 'decide', 'map', 'map file applied', undefined, { kind: 'delete', removed: remove.length, lifted: lift.length })
}

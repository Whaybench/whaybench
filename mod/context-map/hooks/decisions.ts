// Decisions from the map that come with a file Claude reads (settled nodes of kind conclusion).
import type { EngineInterface } from 'claude-code'

import type { MapNode, StoredMap } from '../types'
import type { FileTouch } from './files'
import { FILES, SHARE_DIR } from './paths'

// Decisions are the map's settled nodes of kind conclusion. When Claude reads a file,
// the decisions of the branches that file belongs to (attached to a node, or touched while the
// conversation was on it) come with it, once per file: the record of why, where the code is.
export const toldDecisions = new Set<string>()

async function lineageIds($: EngineInterface): Promise<string[]> {
  const ids = [await $.session.id()]
  const links = (await $.fs.exists(`${SHARE_DIR}/forks/inherited.json`))
    ? (JSON.parse(String(await $.fs.read(`${SHARE_DIR}/forks/inherited.json`))) as Record<string, string>)
    : {}
  for (let i = 0; i < 30 && links[ids[ids.length - 1]] && !ids.includes(links[ids[ids.length - 1]]); i++)
    ids.push(links[ids[ids.length - 1]])
  return ids
}

const globRe = (pattern: string) =>
  new RegExp('^' + pattern.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*\*/g, '\u0000').replace(/\*/g, '[^/]*')
    .replace(/\u0000/g, '.*').replace(/\?/g, '[^/]') + '$')

export async function decisionsFor($: EngineInterface, path: string): Promise<string[]> {
  const ids = await lineageIds($)
  const nodeIds = new Set<string>()
  for (const id of ids) {
    const attached = `${SHARE_DIR}/node-files/${id}.json`
    if (await $.fs.exists(attached)) {
      const byNode = JSON.parse(String(await $.fs.read(attached))) as Record<string, string[]>
      for (const [node, patterns] of Object.entries(byNode)) if (patterns.some(p => globRe(p).test(path))) nodeIds.add(node)
    }
    const touched = `${FILES}/${id}.json`
    if (await $.fs.exists(touched)) {
      for (const t of JSON.parse(String(await $.fs.read(touched))) as FileTouch[]) if (t.path === path && t.node) nodeIds.add(t.node)
    }
  }
  const mapFile = `${SHARE_DIR}/${ids[0]}.json`
  if (nodeIds.size === 0 || !(await $.fs.exists(mapFile))) return []
  const nodes = ((JSON.parse(String(await $.fs.read(mapFile))) as StoredMap).nodes ?? []) as MapNode[]
  const byId = new Map(nodes.map(n => [n.id, n]))
  const isDecision = (n: MapNode) => !n.isOpen && n.kind === 'conclusion'
  const found: MapNode[] = []
  const add = (n?: MapNode) => { if (n && isDecision(n) && !found.includes(n)) found.push(n) }
  for (const id of nodeIds) {
    const n = byId.get(id)
    if (!n) continue
    add(n)
    // A top-level branch holds everything under it; below that, a branch's settled children are its decisions.
    if (n.parent && byId.has(n.parent)) {
      const walk = (pid: string, depth: number) => {
        for (const c of nodes) if (c.parent === pid && depth < 4) { add(c); walk(c.id, depth + 1) }
      }
      walk(n.id, 0)
      // Its siblings' settled ends too: the same part of the work, decided alongside.
      const parent = byId.get(n.parent)
      if (parent?.parent && byId.has(parent.parent)) for (const c of nodes) if (c.parent === parent.id) add(c)
    }
    for (let p = byId.get(n.parent ?? ''), i = 0; p && i < 10; p = byId.get(p.parent ?? ''), i++) add(p)
  }
  return found.slice(0, 15).map(n => `- ${n.label}`)
}

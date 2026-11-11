// The map as text rows: marks, tree prefixes, folding. Used by the map tool's show and the pane.
import type { ContextMap, MapNode } from '../types'

export function asText(map: ContextMap) {
  return rows(map.nodes)
    .map(({ node, prefix }) => `${prefix}${glyph(node, map.current)} ${node.label} [${node.id}]`)
    .join('\n')
}

// The mark before a node: where the conversation is, else the part's state, else open or settled.
// □ work that could be coded now but waits; ◇ an open question or idea, thought about and paused;
// ✕ dropped; ‖ put on hold by the person.
const STATE_GLYPH: Record<string, string> = { plan: '□', doing: '◐', done: '✓', changed: '✎', dropped: '✕', postponed: '‖' }
export function glyph(node: MapNode, current?: string) {
  if (node.id === current) return '●'
  if (node.state && STATE_GLYPH[node.state]) return STATE_GLYPH[node.state]
  return node.isOpen ? '◇' : '✓'
}

// Flattens the tree into drawable rows, depth-first in the given order, skipping folded branches.
export function rows(all: MapNode[], folded: Set<string> = new Set(), wide = false) {
  // The trash is the person's, in the alerts panel; the map shows what is left.
  const nodes = all.filter(n => n.state !== 'removed')
  const ids = new Set(nodes.map(n => n.id))
  const kids = (id?: string) =>
    nodes.filter(n => (id === undefined ? !n.parent || !ids.has(n.parent) : n.parent === id))
  const count = (id: string): number => kids(id).reduce((sum, k) => sum + 1 + count(k.id), 0)
  const out: { node: MapNode; prefix: string; hidden: number; hasKids: boolean }[] = []
  const walk = (id: string | undefined, indent: string) => {
    const list = kids(id)
    list.forEach((node, i) => {
      const isLast = i === list.length - 1
      const branch = id === undefined ? '' : isLast ? (wide ? '└── ' : '└ ') : (wide ? '├── ' : '├ ')
      const hasKids = kids(node.id).length > 0
      const isFolded = hasKids && folded.has(node.id)
      out.push({ node, prefix: indent + branch, hidden: isFolded ? count(node.id) : 0, hasKids })
      if (!isFolded) walk(node.id, id === undefined ? indent : indent + (isLast ? (wide ? '    ' : '  ') : (wide ? '│   ' : '│ ')))
    })
  }
  walk(undefined, '')
  return out
}

// Folded unless flipped: a branch that is settled all the way down and does not hold the current node.
export function foldedSet(nodes: MapNode[], current: string | undefined, flipped: string[]) {
  const below = (id: string): MapNode[] => nodes.filter(n => n.parent === id).flatMap(k => [k, ...below(k.id)])
  const folded = new Set<string>()
  for (const n of nodes) {
    const sub = below(n.id)
    if (sub.length && !n.isOpen && sub.every(k => !k.isOpen) && !sub.some(k => k.id === current)) folded.add(n.id)
  }
  for (const id of flipped) folded.has(id) ? folded.delete(id) : folded.add(id)
  return folded
}

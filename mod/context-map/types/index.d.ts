export type MapNode = {
  id: string
  label: string
  parent?: string
  isOpen: boolean
  // done, dropped and removed (the trash) close a node; the others, and none, leave it open.
  state?: 'plan' | 'doing' | 'done' | 'changed' | 'dropped' | 'postponed' | 'removed'
  kind?: 'question' | 'option' | 'pro' | 'con' | 'conclusion' | 'task' | 'appointment' | 'problem' | 'idea' | 'fact' | 'rule' | 'note'
  // What does not fit the short label: the full sentence, a reason, symptoms.
  detail?: string
  reason?: string
  links?: { to: string; rel: 'cause' | 'leads' | 'replaces' | 'related' | 'resolves' }[]
  // The memory id: nodes about the same problem, in any session, share it.
  memid?: string
  // When and on what the node depends: a time, a due date, and conditions over node ids with ∧ ∨ ¬.
  at?: string
  due?: string
  requires?: string
  assumes?: string
  // Set by the person only: important (!).
  mark?: 'important'
  // Where a node taken up from an imported map came from (Whaybench's continue-from-this-map).
  source?: { file: string; node: string; title?: string; path?: string[] }
  // Recorded by the mod itself, never by Claude: when the node opened, last changed and closed,
  // and the turns in which it became the conversation's current node.
  opened?: Stamp
  changed?: Stamp
  closed?: Stamp
  visits?: number[]
}

// A moment in the conversation: wall-clock time and the person's turn number.
export type Stamp = { at: number; turn: number }

// version: the map data format; turn: the person's prompts counted so far.
export type ContextMap = { nodes: MapNode[]; current?: string; turn?: number; version?: number }

// What `$.store` keeps per session under `map:<session id>`.
export type StoredMap = ContextMap & { updatedAt: number }

// Another session's map the pane is showing instead of this session's own.
export type OtherView = { sessionId: string; map: StoredMap } | null

declare module 'claude-code' {
  interface PluginState {
    'context-map': { map: Shaped<ContextMap>; other: Shaped<OtherView>; toggled: Shaped<string[]> }
  }
}

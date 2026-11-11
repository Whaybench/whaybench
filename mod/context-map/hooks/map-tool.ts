// The context_map tool Claude calls, and the /map command.
import type { Register } from 'claude-code'

import type { MapNode } from '../types'
import { inheritMap, mapNow, setOther, storedMaps, summary, writeMap } from './map-data'
import type { Change } from './map-data'
import { asText } from './map-text'
// Literal here, not imported: the engine reads hook matchers from the hook's own file.
const PANE = 'context-map'
const TOOL = 'mcp__context-map__context_map'

type On = Parameters<Register>[0]

export const DESCRIPTION = `Updates the conversation map pane beside the chat: a tree of the topics this conversation has branched into, and where the conversation stands now. The map is kept between calls, so send only what changed.

Call it in the same turn whenever the structure changes: a new topic, task or side branch starts, the conversation returns to an earlier topic, or a question gets settled. The person follows the conversation on this map, so do not put an update off because the change seems small. A turn in which nothing changed needs no call.
- \`upsert\`: nodes to add or change, by id. For an existing node give only the fields that change (e.g. \`{ "id": "n4", "state": "done" }\`); a new node needs \`label\`.
- \`remove\`: ids to drop, with everything under them.
- \`current\`: the node the conversation is on now.
- \`nodes\`: the whole map, replacing it. Only for a real regrouping, rarely.
- \`show\`: true returns the map as text, when you have lost track of its ids.
Labels are short (about 25 characters) in the conversation's language, shaped by kind: a topic or fact is a noun phrase (a fact as "what: value"), a question ends with a question mark, an option reads "option: what", a conclusion reads "question → answer", a task starts or ends with the verb, an appointment starts with its date, a problem says what goes wrong, a rule says what is always or never done; anything longer (the full sentence, a reason, symptoms) goes in \`detail\`. Open or closed comes from \`state\` alone: a node is open while its question is unsettled; close it once settled (\`state: done\`), drop it when given up (\`state: dropped\`), and open it again (\`state: ""\`, or the state the work is in) when the conversation comes back to it to go further (a closed note or fact the person asks more about is open again, without being told). When a decision settles an open problem, here or in another session, say so at once with a \`resolves\` link to it. Give a node about the same problem as a node in the memory list that node's \`memid\`. Put times, due dates and what a node depends on in \`at\`, \`due\`, \`requires\` and \`assumes\`. Record topics and questions. When the conversation builds something (a game, an app, a setup), keep a branch for it whose children are its parts as in its structure diagram, each with \`state\` (plan, doing, done, changed) updated as the work moves, so the person sees which part is being made, finished or changed without reading code. Keep the two kinds of open node apart: work that could be done now but is put off for later gets \`state: plan\` (drawn □), while an idea or question thought about and paused stays open with no state (drawn ◇). When you lay out a list for the person (options, items, steps), add every item as a child node at once so the list stays beside the chat, and close each as it is done or declined; do not invent next steps nobody has raised.`

export const SCHEMA = {
  type: 'object',
  properties: {
    nodes: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          label: { type: 'string' },
          parent: { type: 'string', description: 'id of the parent node; omit for a top-level topic' },
          state: { type: 'string', enum: ['', 'plan', 'doing', 'done', 'changed', 'dropped', 'postponed'], description: 'open or closed comes from this alone. plan = work put off for later, doing, done = finished or settled (closes), changed, dropped = given up (closes), postponed = put on hold by the person, empty = open again with no state. An open idea or question has no state' },
          kind: { type: 'string', enum: ['question', 'option', 'pro', 'con', 'conclusion', 'task', 'appointment', 'problem', 'idea', 'fact', 'rule', 'note'], description: 'what the node is: question, option (pro and con under it), conclusion (a settled decision), task, appointment (with at), problem (what goes wrong; a decision resolves it), idea, fact, rule (a standing rule or preference), note. A topic has no kind' },
          detail: { type: 'string', description: 'what does not fit the short label: the full sentence, a decision\'s reason, a problem\'s symptoms. Omit when the label says it all' },
          memid: { type: 'string', description: 'the memory id of an existing node in the memory list that is about the same problem; omit for a new problem' },
          at: { type: 'string', description: 'when it happens: YYYY-MM-DD or YYYY-MM-DD HH:MM (an appointment)' },
          due: { type: 'string', description: 'by when: YYYY-MM-DD or YYYY-MM-DD HH:MM' },
          requires: { type: 'string', description: 'what must be done first: node ids of this map with ∧ (and), ∨ (or), ¬ (not), parentheses' },
          assumes: { type: 'string', description: 'what must hold for it to make sense: node ids with ∧ ∨ ¬' },
          reason: { type: 'string', description: 'one short line: why the conversation moved to this node; give it when a map style asks for it' },
          links: { type: 'array', items: { type: 'object', properties: { to: { type: 'string' }, rel: { type: 'string', enum: ['cause', 'leads', 'replaces', 'related', 'resolves'] } }, required: ['to', 'rel'] }, description: 'ties to nodes in other branches. resolves: this decision settles that problem (it closes, with what is open under it); replaces: this takes the place of that' },
          source: { type: 'object', properties: { file: { type: 'string' }, node: { type: 'string' }, title: { type: 'string' }, path: { type: 'array', items: { type: 'string' } } }, required: ['file', 'node'], description: 'for a node taken up from an imported map: that map file and node; give it when Whaybench asks' },
        },
        required: ['id', 'label'],
      },
    },
    upsert: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          label: { type: 'string' },
          parent: { type: 'string', description: 'id of the parent node; empty string for a top-level topic' },
          state: { type: 'string', enum: ['', 'plan', 'doing', 'done', 'changed', 'dropped', 'postponed'], description: 'open or closed comes from this alone. plan = work put off for later, doing, done = finished or settled (closes), changed, dropped = given up (closes), postponed = put on hold by the person, empty = open again with no state. An open idea or question has no state' },
          kind: { type: 'string', enum: ['question', 'option', 'pro', 'con', 'conclusion', 'task', 'appointment', 'problem', 'idea', 'fact', 'rule', 'note'], description: 'what the node is: question, option (pro and con under it), conclusion (a settled decision), task, appointment (with at), problem (what goes wrong; a decision resolves it), idea, fact, rule (a standing rule or preference), note. A topic has no kind' },
          detail: { type: 'string', description: 'what does not fit the short label: the full sentence, a decision\'s reason, a problem\'s symptoms. Omit when the label says it all' },
          memid: { type: 'string', description: 'the memory id of an existing node in the memory list that is about the same problem; omit for a new problem' },
          at: { type: 'string', description: 'when it happens: YYYY-MM-DD or YYYY-MM-DD HH:MM (an appointment)' },
          due: { type: 'string', description: 'by when: YYYY-MM-DD or YYYY-MM-DD HH:MM' },
          requires: { type: 'string', description: 'what must be done first: node ids of this map with ∧ (and), ∨ (or), ¬ (not), parentheses' },
          assumes: { type: 'string', description: 'what must hold for it to make sense: node ids with ∧ ∨ ¬' },
          reason: { type: 'string', description: 'one short line: why the conversation moved to this node; give it when a map style asks for it' },
          links: { type: 'array', items: { type: 'object', properties: { to: { type: 'string' }, rel: { type: 'string', enum: ['cause', 'leads', 'replaces', 'related', 'resolves'] } }, required: ['to', 'rel'] }, description: 'ties to nodes in other branches. resolves: this decision settles that problem (it closes, with what is open under it); replaces: this takes the place of that' },
          source: { type: 'object', properties: { file: { type: 'string' }, node: { type: 'string' }, title: { type: 'string' }, path: { type: 'array', items: { type: 'string' } } }, required: ['file', 'node'], description: 'for a node taken up from an imported map: that map file and node; give it when Whaybench asks' },
        },
        required: ['id'],
      },
    },
    remove: { type: 'array', items: { type: 'string' } },
    current: { type: 'string', description: 'id of the node the conversation is on now' },
    show: { type: 'boolean' },
    inherit: { type: 'string', description: 'a session id whose map this session takes over, replacing its own' },
  },
}

// /map and the map tool's calls.
export function mapToolHooks(on: On) {
  on('command.run', { command: 'map' }, async ($, e) => {
    const arg = String((e as { args?: string }).args ?? '').trim()
    const own = await $.session.id()
    if (arg === 'list') {
      const lines = (await storedMaps($)).map(
        ({ sessionId, map: m }) =>
          `${sessionId.slice(0, 8)}${sessionId === own ? ' (this session)' : ''}  ${new Date(m.updatedAt).toLocaleString()}  ${summary(m)}`,
      )
      return { text: lines.length ? lines.join('\n') : 'No saved maps.' }
    }
    if (arg === '' || own.startsWith(arg)) {
      await setOther($, null)
      await $.ui.open({ id: PANE, title: 'Conversation map' })
      return { text: 'Showing the map of this session.' }
    }
    const found = (await storedMaps($)).filter(m => m.sessionId.startsWith(arg))
    if (found.length !== 1) {
      return { text: found.length ? `Several sessions start with "${arg}". Give more characters.` : `No map starts with "${arg}". See /map list.` }
    }
    await setOther($, found[0] ?? null)
    await $.ui.open({ id: PANE, title: 'Conversation map' })
    return { text: `Showing the map of session ${found[0].sessionId.slice(0, 8)}. Back: /map` }
  })

  on('tool.call', { tool: TOOL }, async ($, e) => {
    // A registered tool's arguments arrive spread on the event itself.
    const input = e as unknown as { nodes?: MapNode[]; upsert?: Change[]; remove?: string[]; current?: string; show?: boolean; inherit?: string }
    if (input.inherit) {
      const taken = await inheritMap($, await $.session.id(), input.inherit)
      if (!taken) return { result: `No map found for ${input.inherit}.` }
    }
    const changes = input.nodes || input.upsert?.length || input.remove?.length || input.current
    if (changes) {
      try {
        await writeMap($, await $.session.id(), { nodes: input.nodes, upsert: input.upsert, remove: input.remove, current: input.current })
      } catch (err) {
        return { result: `Map not updated: ${(err as Error).message}. Send the change again, fixed.` }
      }
    }

    return { result: input.show ? asText(await mapNow($)) : 'Map updated.' }
  })
}

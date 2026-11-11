// context-map: the conversation map beside the chat, and the session's record for Whaybench.
// This file only assembles: each part lives in its own module; the session start and the
// prompt hook stay here because the engine takes one prompt.submit hook and they draw on every part.
import type { Register } from 'claude-code'

import type { MapNode, StoredMap } from '../types'
import { answersHooks } from './answers'
import { filesHooks, filesRecordHooks } from './files'
import { drainInbox } from './inbox'
import { logLine } from './log'
import { carriedSummary, changeMap, inheritMap, mapNow, refreshMap, writeMap } from './map-data'
import { memoryHooks, memoryNote } from './memory'
import { applyGraft, applyPatches } from './map-patches'
import { DESCRIPTION, mapToolHooks, SCHEMA } from './map-tool'
import { paneHooks } from './pane'
import { PANE, PREFIX, initPaths } from './paths'
import { changedFilesNote, safetyHooks } from './safety'
import { adoptFork, isShownElsewhere, record, REWIND_NOTE, sessionRecordHooks, setTab, takeRewound, touchSession } from './session-record'

const MAP_NOTE = 'Conversation map: if this turn brings a new topic or task, returns to an earlier one, or settles a question, call mcp__context-map__context_map in this turn before answering. An empty map gets its first topic in the first turn. A settled question is labelled "question → conclusion (reason)", closed, and given kind conclusion. Every item of a list you give becomes a child node. Put-off work has state plan; an idea or question thought about and paused stays open with no state. No call when nothing changed.'

// Told on a session's first prompt in Whaybench: without it a session opens things outside the window (xdg-open and the like).
const VIEW_NOTE = 'Whaybench: everything you show the person opens inside the view panel beside this conversation, and nowhere else. It follows this session: when they come back here, your views are there. Show a web page, a file, a document or anything you made only with `whaybench-view open URL|PATH|-`; a site they name is shown by opening its real address. A web address (http, https) opens as a tab of the browser inside Whaybench, a real Firefox in a panel here, where the person is signed in. Never open anything outside the panel, by any command: no xdg-open, gio open, kde-open, browsers, viewers or other desktop apps. `whaybench-view read` gives you what the panel shows as text; `whaybench-view --help` lists the rest.'
// Every later turn, in short: the long note can be lost when the conversation is compacted.
const VIEW_SHORT = 'Show things only in the view panel: whaybench-view open (a web address opens in the browser tab there).'
let viewNoteDue = false
let inWhaybench = false

// What a session started from another session's map is told on its first prompt: the map it
// took over, in short, since none of the conversation behind it comes along.
let carried: string | undefined
// The program's prompt for a session that keeps a map (WHAYBENCH_MAP_PROMPT): submitted once, with the summary.
let carryPrompt: string | undefined

export const register: Register = on => {
  // Hooks run in the order they are registered here.
  filesHooks(on)  // Bash run times

  // Before Claude reads the person's next message: carried map, /rewind note, files the person changed.
  on('prompt.submit', async ($, e, next) => {
    // Record the prompt for Whaybench's session list (one prompt.submit hook only: the engine refuses two).
    const text = String((e as { text?: string }).text ?? '').trim()
    if (text && !text.startsWith('/') && !text.startsWith('[')) await touchSession($, text.replace(/\s+/g, ' ').slice(0, 200)).catch(() => undefined)
    const carriedNote = carried && !e.command ? [carried] : []
    if (carriedNote.length) carried = undefined
    const rewindNote = [...carriedNote, ...(!e.command && takeRewound() ? [REWIND_NOTE] : [])]
    const changed = e.command ? [] : await changedFilesNote($).catch(() => [])
    // The person follows the conversation on the map, so it is kept up in the turn things happen.
    const mapNote = e.command ? [] : [MAP_NOTE]
    if (!e.command) await refreshMap($, await $.session.id()).catch(() => undefined)
    const memory = e.command ? [] : await memoryNote($).catch(() => [])
    if (inWhaybench && !e.command) { mapNote.unshift(viewNoteDue ? VIEW_NOTE : VIEW_SHORT); viewNoteDue = false }
    return next({ ...e, context: [...(e.context ?? []), ...rewindNote, ...changed, ...memory, ...mapNote] })
  })

  safetyHooks(on)
  memoryHooks(on)
  filesRecordHooks(on)
  sessionRecordHooks(on)

  on('session.start', async ($, e, next) => {
    await initPaths($)
    await $.tool.register({ name: 'context_map', description: DESCRIPTION, inputSchema: SCHEMA, isDeferred: false })
    await $.command.register({
      name: 'map',
      description: 'Conversation map: no argument for this session, list for saved maps, the start of a session id for that map',
    })
    const saved = (await $.store.get(PREFIX + (await $.session.id()))) as StoredMap | undefined
    if (saved?.nodes) await changeMap($, () => ({ nodes: saved.nodes, current: saved.current }))
    // Inside Whaybench the map is already a panel of the window.
    inWhaybench = (await $.process.run(['printenv', 'WHAYBENCH']).catch(() => null))?.stdout.trim() === '1'
    viewNoteDue = inWhaybench
    setTab((await $.process.run(['printenv', 'WHAYBENCH_TAB']).catch(() => null))?.stdout.trim() || undefined)
    if (!inWhaybench) void $.ui.open({ id: PANE, title: 'Conversation map' })
    // Started from another session's map: take it over; its summary rides on the first prompt.
    const keepFrom = (await $.process.run(['printenv', 'WHAYBENCH_MAP_FROM']).catch(() => null))?.stdout.trim()
    if (keepFrom?.startsWith('/') && !saved?.nodes) {
      // An imported map's file: it becomes this new session's own map.
      const body = (await $.fs.exists(keepFrom)) ? (JSON.parse(String(await $.fs.read(keepFrom))) as { nodes?: MapNode[]; title?: string }) : {}
      if (Array.isArray(body.nodes) && body.nodes.length) {
        // Its nodes start over in this session, stamped as opened now; the panel keeps following what it did.
        await writeMap($, await $.session.id(), { nodes: body.nodes }, '--start-from')
        carried = carriedSummary(await mapNow($), `imported map '${body.title ?? keepFrom}'`)
      }
    } else if (keepFrom && !saved?.nodes && (await inheritMap($, await $.session.id(), keepFrom).catch(() => false)))
      carried = carriedSummary(await mapNow($), keepFrom)
    const asked = (await $.process.run(['printenv', 'WHAYBENCH_MAP_PROMPT']).catch(() => null))?.stdout.trim()
    if (carried && asked) carryPrompt = `${asked}\n\n${carried}`
    let sessionId = await $.session.id()
    const startedAs = keepFrom?.startsWith('/') ? 'imported map' : keepFrom && carried ? 'map taken over' : saved?.nodes ? 'reopened' : 'new'
    const forked = await adoptFork($, sessionId).catch(() => false)
    await logLine($, sessionId, 'decide', 'session', 'start mode', undefined, { mode: forked ? 'went on from a standby process' : startedAs, taken_from: keepFrom || undefined })
    if (!forked) {
      void (async () => {
        for (let i = 0; i < 5; i++) {
          await $.process.run(['sleep', '2']).catch(() => undefined)
          if (await adoptFork($, await $.session.id()).catch(() => false)) {
            await logLine($, await $.session.id(), 'decide', 'session', 'number change', "took a standby process's late note", { mode: 'standby process', try: i + 1 })
            break
          }
        }
      })()
    }
    await touchSession($).catch(() => undefined)
    let busy = false
    let ticks = 0
    $.clock.every(1000, () => {
      if (busy) return
      busy = true
      void (async () => {
        // /resume inside a running session switches it to another id without a new start: the
        // heartbeat and inbox follow the id now in use, and the old one stops looking alive.
        const now = await $.session.id()
        if (now !== sessionId) {
          const old = sessionId
          await record($, sessionId, ['off']).catch(() => undefined)
          sessionId = now
          // Its map too: the atom still holds the old session's, and the next map call would
          // overwrite the resumed session's map with it.
          // A spare process taken up by /background gets its id now: it may carry a conversation on.
          const adopted = await adoptFork($, now).catch(() => false)
          if (!adopted) await inheritMap($, now, now).catch(() => undefined)
          const how = adopted ? 'standby process' : 'resume'
          await logLine($, old, 'decide', 'session', 'number change', 'this process moved to another number', { old, new: now, mode: how })
          await logLine($, now, 'decide', 'session', 'number change', 'this process moved to another number', { old, new: now, mode: how })
          await touchSession($).catch(() => undefined)
          ticks = 0
        }
        if (isShownElsewhere()) return  // the background session's own process keeps its heartbeat and inbox
        if (ticks++ % 10 === 0) await record($, sessionId, ['alive']).catch(() => undefined)
        if (carryPrompt) {
          const text = carryPrompt
          carryPrompt = carried = undefined
          // Our own submits skip our prompt hook, so the record is touched here.
          await touchSession($, text.replace(/\s+/g, ' ').slice(0, 200)).catch(() => undefined)
          await $.prompt.submit({ text, asUser: true }).catch(() => undefined)
        }
        await drainInbox($, sessionId).catch(() => undefined)
        await applyPatches($, sessionId).catch(() => undefined)
        await applyGraft($, sessionId).catch(() => undefined)
      })().finally(() => (busy = false))
    })

    return next(e)
  })

  answersHooks(on)
  mapToolHooks(on)
  paneHooks(on)
}

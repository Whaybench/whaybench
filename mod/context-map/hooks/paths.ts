// Where the mod keeps what it shares with Whaybench. Every path is written here only.
// The pane id and the tool's full name; map-tool.ts and pane.tsx repeat them as literals for their matchers.
export const PANE = 'context-map'
export const TOOL = 'mcp__context-map__context_map'
// $.store key of a session's map: map:<session id>
export const PREFIX = 'map:'
// The one user data directory, as the program finds it; the location is never stored.
// A mod has no process.env, so these are read from the environment when the session starts (initPaths),
// before anything else uses them. They are live bindings: every module sees the values then set.
export let HOME = ''
export let SHARE_DIR = ''
// What can be made again goes here: deleting it loses nothing of the person's.
export let CACHE = ''
// Where Claude Code keeps this account's conversations.
export let TRANSCRIPTS = ''
// Prompts Whaybench queues for a session arrive here as <session id>.json: [{ seq, text }, ...].
export let INBOX = ''
// One small record per session for the session list: what it began with, what it is on.
export let SESSIONS = ''
// The last reply of each session, for the answer panel, and a heartbeat saying the session runs.
export let ANSWERS = ''
export let ALIVE = ''
// Every answer of a session, one JSON line per turn: <session id>.jsonl = {at, prompt, text}.
export let ANSWER_LOG = ''
// /background carries a conversation on under a new session id; the old one leaves a note here
// and the new one, starting moments later, takes over its map and record.
export let FORKS = ''
// Files read or changed here, each with the map node the conversation was on, for the file list.
export let FILES = ''
// What each session is doing, for the session list: working, waiting or done.
export let STATE = ''
// Fields a side worker filled, branches brought in, deletions from the map menu.
export let PATCHES = ''
// The program's one command (linked here by Whaybench at start): every change to the map goes through it.
export let WHAYBENCH = ''
// Debug record: this session's lines, in the same line format as the program's run files.
export let LOGS_SESSIONS = ''
// Parts the person switched to detail in the log menu: { parts: [...] }.
export let LOG_DETAIL = ''

/** The first thing a session does: finds the data directory and makes sure its folders exist. */
export async function initPaths($: { process: { run(argv: string[]): Promise<{ stdout: string }> } }) {
  const read = async (name: string) => (await $.process.run(['printenv', name]).catch(() => null))?.stdout.trim() ?? ''
  HOME = await read('HOME')
  const given = await read('WHAYBENCH_DATA_DIR')
  const xdg = await read('XDG_DATA_HOME')
  SHARE_DIR = given || (xdg ? `${xdg}/whaybench` : `${HOME}/.local/share/whaybench`)
  CACHE = `${SHARE_DIR}/cache`
  WHAYBENCH = `${HOME}/.local/bin/whaybench`
  TRANSCRIPTS = `${HOME}/.claude/projects`
  INBOX = `${CACHE}/inbox`
  SESSIONS = `${SHARE_DIR}/sessions`
  ANSWERS = `${SHARE_DIR}/answers`
  ALIVE = `${CACHE}/alive`
  ANSWER_LOG = `${SHARE_DIR}/answer-log`
  FORKS = `${CACHE}/forks/pending.json`
  FILES = `${SHARE_DIR}/files`
  STATE = `${CACHE}/state`
  PATCHES = `${SHARE_DIR}/map-patches`
  LOGS_SESSIONS = `${SHARE_DIR}/logs/sessions`
  LOG_DETAIL = `${SHARE_DIR}/logs/detail.json`
  await $.process.run(['mkdir', '-p', INBOX, SESSIONS, ANSWERS, ALIVE, ANSWER_LOG, `${CACHE}/forks`, `${SHARE_DIR}/forks`, FILES, STATE, PATCHES, LOGS_SESSIONS, `${SHARE_DIR}/show`, `${SHARE_DIR}/node-files`]).catch(() => undefined)
}

export function clock(ms: number) {
  return new Date(ms).toTimeString().slice(0, 5)
}


// Keeping Claude's changes from clashing with the person's: files open in vim, files saved since read.
import type { EngineInterface, Register } from 'claude-code'

import { changedSince, toldChange } from './files'
import { clock } from './paths'

type On = Parameters<Register>[0]

// A file open in vim has a swap file beside it (.name.swp, .swo, .swn …) naming vim's pid and
// whether there are unsaved changes. Changing the file under vim makes one save overwrite the
// other, so the first change Claude tries is held back with a note to ask the person; trying the
// same file again (the person agreed) goes through.
const warnedVim = new Set<string>()

export async function vimHolding($: EngineInterface, path: string): Promise<{ pid: number; dirty: boolean } | null> {
  const slash = path.lastIndexOf('/')
  const dir = path.slice(0, slash + 1)
  const base = path.slice(slash + 1)
  for (const ext of ['swp', 'swo', 'swn']) {
    const swap = `${dir}.${base}.${ext}`
    if (!(await $.fs.exists(swap).catch(() => false))) continue
    // block 0: pid (4 bytes, little-endian) at 24, dirty flag 'U' at 1007
    const read = await $.process.run(['python3', '-c',
      'import sys,os;b=open(sys.argv[1],"rb").read(1008);p=int.from_bytes(b[24:28],"little");' +
      'alive=os.path.exists(f"/proc/{p}");print(p if alive else 0, 1 if len(b)>1007 and b[1007]==0x55 else 0)',
      swap]).catch(() => null)
    const [pid, dirty] = (read?.stdout.trim() ?? '0 0').split(' ').map(Number)
    if (pid) return { pid, dirty: dirty === 1 }
  }
  return null
}

// For the next prompt: files the person changed since Claude read or changed them.
export async function changedFilesNote($: EngineInterface): Promise<string[]> {
  const changes = await changedSince($).catch(() => [])
  if (changes.length === 0) return []
  const lines: string[] = []
  for (const c of changes) {
    toldChange.set(c.path, c.at)
    const vim = await vimHolding($, c.path).catch(() => null)
    lines.push(`- ${c.path} (changed at ${clock(c.at)}${vim ? ', still open in vim' + (vim.dirty ? ' with unsaved changes' : '') : ''})`)
  }
  return ['Files you read or changed earlier in this session were changed on disk since, most likely by the person ' +
    '(saved in an editor): read them again before relying on what you remember of them.\n' + lines.join('\n')]
}

// An edit is held back once when the file changed on disk or is open in vim.
export function safetyHooks(on: On) {
  on('tool.call', { tool: ['Edit', 'Write', 'NotebookEdit'] }, async ($, e, next) => {
    const path = 'notebook_path' in e ? e.notebook_path : e.file_path
    // Changed on disk while Claude was working: say who, so it does not wonder why.
    const changed = path ? (await changedSince($, path).catch(() => []))[0] : undefined
    if (path && changed) {
      toldChange.set(path, changed.at)
      return {
        deny: `${path} was changed on disk at ${clock(changed.at)}, after you last read or changed it: most likely the person ` +
          'saved it in an editor. Read it again and make your change on top of theirs.',
      }
    }
    const vim = path ? await vimHolding($, path).catch(() => null) : null
    if (path && vim && !warnedVim.has(`${path}:${vim.pid}`)) {
      warnedVim.add(`${path}:${vim.pid}`)
      return {
        deny: `${path} is open in vim right now (pid ${vim.pid}, ${vim.dirty ? 'with unsaved changes' : 'no unsaved changes'}). ` +
          'Changing it under vim makes one save overwrite the other. Tell the person and ask before changing it ' +
          '(they may save and close it, or reload it in vim with :e afterwards); if they agree, make the same change again and it goes through.',
      }
    }
    return next(e)
  })
}

// The map pane inside Claude Code (outside Whaybench), and hiding the map tool's own calls in the transcript.
import type { Register } from 'claude-code'

import { flipToggled, mapNow, otherNow, toggledNow } from './map-data'
import { foldedSet, glyph, rows } from './map-text'
// Literal here, not imported: the engine reads hook matchers from the hook's own file.
const PANE = 'context-map'
const TOOL = 'mcp__context-map__context_map'

type On = Parameters<Register>[0]

// Drawing.
export function paneHooks(on: On) {
  // The map's own calls stay out of the transcript; ctrl+o still shows them in full.
  on('ui.render', { component: 'ToolUse' }, async ($, e, next) => {
    if (e.props.tool !== TOOL || e.props.isExpanded || e.props.isErrored) return next(e)
    const { Box } = $.ui.resolve(e)

    return <Box />
  })

  on('ui.render', { component: 'ToolResult' }, async ($, e, next) => {
    if (e.props.tool !== TOOL || e.props.isExpanded || e.props.isErrored) return next(e)
    const { Box } = $.ui.resolve(e)

    return <Box />
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const viewing = await otherNow($)
    const { nodes, current } = viewing ? viewing.map : await mapNow($)
    const folded = foldedSet(nodes, current, await toggledNow($))
    const flip = (id: string) => () =>
      flipToggled($, id)

    return (
      <Box flexDirection="column">
        {viewing && <Text dimColor>Map of session {viewing.sessionId.slice(0, 8)} (read only)</Text>}
        {nodes.length === 0 && <Text dimColor>No map yet.</Text>}
        {rows(nodes, folded, true).map(({ node, prefix, hidden, hasKids }) => {
          const isHere = node.id === current
          const line = (
            <Text dimColor={!node.isOpen && !isHere} bold={isHere}>
              {prefix}
              {hasKids ? (hidden ? '+ ' : '- ') : '  '}
              {glyph(node, current) + ' '}
              {node.label}
              {hidden ? ` (${hidden})` : ''}
              {isHere ? '  ← Current' : ''}
            </Text>
          )
          return hasKids ? (
            <Button key={node.id} plain onPress={flip(node.id)}>
              {line}
            </Button>
          ) : (
            line
          )
        })}
      </Box>
    )
  })
}

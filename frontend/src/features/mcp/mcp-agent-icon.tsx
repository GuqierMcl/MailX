import claudeCode from '@/assets/agents/claude-code.svg'
import codexLight from '@/assets/agents/codex-light.svg'
import codexDark from '@/assets/agents/codex-dark.svg'
import cursorLight from '@/assets/agents/cursor-light.svg'
import cursorDark from '@/assets/agents/cursor-dark.svg'
import openCode from '@/assets/agents/opencode.svg'
import trae from '@/assets/agents/trae.svg'
import deepseek from '@/assets/agents/deepseek.svg'
import type { McpAgentId } from './types'

const LOGOS = {
  claudeCode: { light: claudeCode, dark: claudeCode },
  codex: { light: codexLight, dark: codexDark },
  cursor: { light: cursorLight, dark: cursorDark },
  trae: { light: trae, dark: trae },
  deepseekHarness: { light: deepseek, dark: deepseek },
} as const

/** Local brand assets; the adjacent client name provides the accessible label. */
export function McpAgentIcon({ id }: { id: McpAgentId }) {
  if (id === 'openCode') {
    // This original SVG uses currentColor; a mask follows the app's theme.
    const mask = `url("${openCode}")`
    return <span aria-hidden className="inline-block size-5 shrink-0 bg-current"
      style={{ maskImage: mask, WebkitMaskImage: mask, maskSize: 'contain', WebkitMaskSize: 'contain', maskRepeat: 'no-repeat', WebkitMaskRepeat: 'no-repeat', maskPosition: 'center', WebkitMaskPosition: 'center' }} />
  }
  const logo = LOGOS[id]
  return <span aria-hidden className="inline-flex size-5 shrink-0 items-center justify-center">
    <img src={logo.light} alt="" draggable={false} className="size-full object-contain dark:hidden" />
    <img src={logo.dark} alt="" draggable={false} className="hidden size-full object-contain dark:block" />
  </span>
}

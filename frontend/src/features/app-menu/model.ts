export const editCommands = ['undo', 'redo', 'cut', 'copy', 'paste', 'selectAll'] as const
export type EditCommand = typeof editCommands[number]

export const definitions = {
  mailx: ['about', 'update', 'settings', 'accounts', 'quit'],
  file: ['compose', 'attach', 'close'],
  edit: [...editCommands, 'search'],
  view: ['toggleSidebar', 'toggleReader', 'toggleAssistant', 'appearance', 'toggleFullscreen'],
  message: ['refresh', 'reply', 'replyAll', 'forward', 'toggleRead', 'toggleStar', 'archive', 'delete', 'send'],
  window: ['minimize', 'maximize', 'mainWindow', 'openWindows'],
  help: ['guide', 'shortcuts'],
} as const

export const themes = ['themeSystem', 'themeLight', 'themeDark'] as const
export type GroupId = keyof typeof definitions
export type CommandId = Exclude<typeof definitions[GroupId][number], 'appearance' | 'openWindows'> | typeof themes[number]

export const accelerators: Partial<Record<CommandId, string>> = {
  settings: 'CmdOrCtrl+Comma',
  quit: 'CmdOrCtrl+Q',
  compose: 'CmdOrCtrl+N',
  close: 'CmdOrCtrl+W',
  undo: 'CmdOrCtrl+Z',
  redo: 'CmdOrCtrl+Shift+Z',
  cut: 'CmdOrCtrl+X',
  copy: 'CmdOrCtrl+C',
  paste: 'CmdOrCtrl+V',
  selectAll: 'CmdOrCtrl+A',
  search: 'CmdOrCtrl+F',
  toggleSidebar: 'CmdOrCtrl+B',
  toggleAssistant: 'CmdOrCtrl+J',
  send: 'CmdOrCtrl+Enter',
  minimize: 'CmdOrCtrl+M',
}

export type MenuItem = {
  id: string
  label: string
  enabled: boolean
  checked?: boolean
  accelerator?: string
  items?: MenuItem[]
  separatorBefore?: boolean
  native?: boolean
}

export type MenuGroup = { id: GroupId; label: string; items: MenuItem[] }

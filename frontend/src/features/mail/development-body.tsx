// Synthetic browser-only preview. This module is eliminated from production
// and is never a fallback for desktop mail or protocol failures.
const STYLE = 'body{font:16px/1.8 sans-serif;color:#222;padding:24px}aside{padding:16px;background:#f3f4f6}p{margin:0 0 12px}h1{margin-top:0}'
const ORIGINAL = {
  title: '项目进展',
  lead: '这是用于界面开发的合成邮件。',
  aside: '会议纪要与下一步安排将在本周更新。',
  paragraph: (n: number) => `第 ${n} 段：用于撑高页面的合成文字，便于检查滚动位置是否保持。`,
}
const TRANSLATED = {
  title: 'Project progress',
  lead: 'This is a synthetic message used for interface development.',
  aside: 'Meeting notes and next steps will be updated this week.',
  paragraph: (n: number) => `Paragraph ${n}: synthetic filler text that makes the page tall enough to check that the scroll position is kept.`,
}

function sampleDocument(translated: boolean) {
  const text = translated ? TRANSLATED : ORIGINAL
  const accent = translated ? 'border-left:6px solid #2563eb;' : ''
  const paragraphs = Array.from({ length: 24 }, (_, index) => `<p>${text.paragraph(index + 1)}</p>`).join('')
  return `<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'"><meta charset="utf-8"><style>${STYLE}body{${accent}}</style><h1>${text.title}</h1><p>${text.lead}</p><aside>${text.aside}</aside>${paragraphs}`
}

export default function DevelopmentBody({ translated = false }: { translated?: boolean }) {
  return <div className="flex min-h-0 flex-1 flex-col gap-2">
    <p className="text-xs text-muted-foreground">开发预览 · 合成 HTML 样本{translated ? '（译文）' : ''}</p>
    <iframe title={translated ? '开发预览 HTML 正文（译文）' : '开发预览 HTML 正文'} sandbox="" referrerPolicy="no-referrer" className="min-h-0 w-full flex-1 border-0 bg-white"
      srcDoc={sampleDocument(translated)} />
  </div>
}

// Synthetic browser-only preview. This module is eliminated from production
// and is never a fallback for desktop mail or protocol failures.
export default function DevelopmentBody() {
  return <div className="flex min-h-0 flex-1 flex-col gap-2">
    <p className="text-xs text-muted-foreground">开发预览 · 合成 HTML 样本</p>
    <iframe title="开发预览 HTML 正文" sandbox="" referrerPolicy="no-referrer" className="min-h-0 w-full flex-1 border-0 bg-white"
      srcDoc={'<!doctype html><meta http-equiv="Content-Security-Policy" content="default-src \'none\'; style-src \'unsafe-inline\'"><meta charset="utf-8"><style>body{font:16px/1.8 sans-serif;color:#222;padding:24px}aside{padding:16px;background:#f3f4f6}</style><h1>项目进展</h1><p>这是用于界面开发的合成邮件。</p><aside>会议纪要与下一步安排将在本周更新。</aside>'} />
  </div>
}

import { expect, test } from 'bun:test'
import { formatSize, pickDownloads } from './release'

const asset = (name: string, size = 1) => ({ name, size, browser_download_url: `https://example.com/${name}` })

test('pickDownloads finds installers by platform and ignores update packages', () => {
  const latest = pickDownloads({
    tag_name: 'v0.5.0',
    published_at: '2026-11-21T08:43:46Z',
    html_url: 'https://github.com/GuqierMcl/MailX/releases/tag/v0.5.0',
    draft: false,
    prerelease: false,
    assets: [
      asset('MailX_0.5.0_macos-arm64.dmg', 19302213),
      asset('MailX_0.5.0_macos-arm64.app.tar.gz'),
      asset('MailX_0.5.0_macos-x64.dmg'),
      asset('MailX_0.5.0_windows-x64-setup.exe'),
      asset('MailX_0.5.0_windows-x64-setup.exe.sig'),
      asset('MailX_0.5.0_linux-x64.AppImage'),
      asset('MailX_0.5.0_linux-x64.deb'),
      asset('MailX_0.5.0_linux-x64.rpm'),
      asset('MailX_0.4.1_linux-x64.rpm'),
      asset('SHA256SUMS.txt'),
    ],
  })
  expect(latest.version).toBe('0.5.0')
  expect(latest.date).toBe('2026-11-21')
  expect(Object.keys(latest.downloads).sort()).toEqual(['linux-appimage', 'linux-deb', 'linux-rpm', 'macos-arm64', 'macos-x64', 'windows-x64'])
  expect(latest.downloads['macos-arm64']).toEqual({ url: 'https://example.com/MailX_0.5.0_macos-arm64.dmg', size: 19302213 })
  expect(latest.downloads['linux-rpm']?.url).toBe('https://example.com/MailX_0.5.0_linux-x64.rpm')
  expect(latest.checksums).toBe('https://example.com/SHA256SUMS.txt')
})

test('formatSize rounds to megabytes', () => {
  expect(formatSize(19302213)).toBe('18 MB')
})

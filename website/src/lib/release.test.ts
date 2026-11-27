import { expect, test } from 'bun:test'
import { formatSize, pickDownloads } from './release'

const asset = (name: string, size = 1) => ({ name, size, browser_download_url: `https://example.com/${name}` })
const release = {
  tag_name: 'v0.5.0',
  published_at: '2026-11-21T08:43:46Z',
  html_url: 'https://github.com/GuqierMcl/MailX/releases/tag/v0.5.0',
  draft: false,
  prerelease: false,
}

test('pickDownloads finds installers by platform and ignores update packages', () => {
  const latest = pickDownloads({
    ...release,
    assets: [
      asset('MailX_0.5.0_macos-arm64.dmg', 19302213),
      asset('MailX_0.5.0_macos-arm64.app.tar.gz'),
      asset('MailX_0.5.0_macos-x64.dmg'),
      asset('MailX_0.5.0_windows-x64-setup.exe'),
      asset('MailX_0.5.0_windows-x64-setup.exe.sig'),
      asset('MailX_0.5.0_linux-x64.AppImage'),
      asset('MailX_0.5.0_linux-x64.deb'),
      asset('MailX_0.5.0_linux-x64.rpm'),
      asset('MailX_0.5.0_linux-arm64.AppImage.sig'),
      asset('MailX_0.5.0_linux-arm64.AppImage'),
      asset('MailX_0.5.0_linux-arm64.deb'),
      asset('MailX_0.5.0_linux-arm64.rpm'),
      asset('MailX_0.4.1_linux-x64.rpm'),
      asset('SHA256SUMS.txt'),
    ],
  })
  expect(latest.version).toBe('0.5.0')
  expect(latest.date).toBe('2026-11-21')
  expect(Object.keys(latest.downloads).sort()).toEqual([
    'linux-arm64-appimage', 'linux-arm64-deb', 'linux-arm64-rpm',
    'linux-x64-appimage', 'linux-x64-deb', 'linux-x64-rpm',
    'macos-arm64', 'macos-x64', 'windows-x64',
  ])
  expect(latest.downloads['macos-arm64']).toEqual({ url: 'https://example.com/MailX_0.5.0_macos-arm64.dmg', size: 19302213 })
  expect(latest.downloads['linux-x64-rpm']?.url).toBe('https://example.com/MailX_0.5.0_linux-x64.rpm')
  expect(latest.downloads['linux-arm64-appimage']?.url).toBe('https://example.com/MailX_0.5.0_linux-arm64.AppImage')
  expect(latest.checksums).toBe('https://example.com/SHA256SUMS.txt')
})

test('pickDownloads omits ARM64 when a release only contains Linux x64 installers', () => {
  const latest = pickDownloads({
    ...release,
    assets: [
      asset('MailX_0.5.0_linux-x64.AppImage'),
      asset('MailX_0.5.0_linux-x64.deb'),
      asset('MailX_0.5.0_linux-x64.rpm'),
    ],
  })
  expect(Object.keys(latest.downloads).sort()).toEqual(['linux-x64-appimage', 'linux-x64-deb', 'linux-x64-rpm'])
})

test('formatSize rounds to megabytes', () => {
  expect(formatSize(19302213)).toBe('18 MB')
})

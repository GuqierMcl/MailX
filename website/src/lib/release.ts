/**
 * The download page is built from the latest GitHub release. Installers are
 * named `MailX_<version>_<platform><suffix>` by scripts/release-assets.mjs, so
 * each download is found by its platform and suffix, not by version.
 */

export const REPOSITORY = 'GuqierMcl/MailX'
export const RELEASES_URL = `https://github.com/${REPOSITORY}/releases`

export type LinuxArchitecture = 'x64' | 'arm64'
type LinuxFormat = 'appimage' | 'deb' | 'rpm'
export type DownloadId = 'macos-arm64' | 'macos-x64' | 'windows-x64' | 'windows-arm64' | `linux-${LinuxArchitecture}-${LinuxFormat}`

const SUFFIXES: Record<DownloadId, string> = {
  'macos-arm64': '_macos-arm64.dmg',
  'macos-x64': '_macos-x64.dmg',
  'windows-x64': '_windows-x64-setup.exe',
  'windows-arm64': '_windows-arm64-setup.exe',
  'linux-x64-appimage': '_linux-x64.AppImage',
  'linux-x64-deb': '_linux-x64.deb',
  'linux-x64-rpm': '_linux-x64.rpm',
  'linux-arm64-appimage': '_linux-arm64.AppImage',
  'linux-arm64-deb': '_linux-arm64.deb',
  'linux-arm64-rpm': '_linux-arm64.rpm',
}

export interface Download {
  url: string
  size: number
}

export interface LatestRelease {
  version: string
  date: string
  url: string
  checksums: string | null
  downloads: Partial<Record<DownloadId, Download>>
}

interface GitHubAsset {
  name: string
  size: number
  browser_download_url: string
}

interface GitHubRelease {
  tag_name: string
  published_at: string | null
  html_url: string
  draft: boolean
  prerelease: boolean
  assets: GitHubAsset[]
}

export function pickDownloads(release: GitHubRelease): LatestRelease {
  const version = release.tag_name.replace(/^v/, '')
  const downloads: LatestRelease['downloads'] = {}
  for (const [id, suffix] of Object.entries(SUFFIXES) as [DownloadId, string][]) {
    const asset = release.assets.find((item) => item.name === `MailX_${version}${suffix}`)
    if (asset) downloads[id] = { url: asset.browser_download_url, size: asset.size }
  }
  const checksums = release.assets.find((item) => item.name === 'SHA256SUMS.txt')?.browser_download_url ?? null
  return { version, date: (release.published_at ?? '').slice(0, 10), url: release.html_url, checksums, downloads }
}

/**
 * Asks GitHub once per build. A failed request is fatal in CI, so a deploy
 * never ships a download page without downloads; locally it falls back to the
 * Releases page so the site still builds offline.
 */
export async function latestRelease(): Promise<LatestRelease | null> {
  const headers: Record<string, string> = { Accept: 'application/vnd.github+json', 'User-Agent': 'mailx-website-build' }
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`
  try {
    const response = await fetch(`https://api.github.com/repos/${REPOSITORY}/releases/latest`, { headers })
    if (!response.ok) throw new Error(`GitHub answered ${response.status}`)
    const latest = pickDownloads(await response.json() as GitHubRelease)
    if (Object.keys(latest.downloads).length === 0) throw new Error(`release ${latest.version} has no installers`)
    return latest
  } catch (error) {
    if (process.env.CI) throw new Error(`Cannot read the latest release: ${(error as Error).message}`)
    console.warn(`[download] ${(error as Error).message}; linking to the Releases page instead.`)
    return null
  }
}

export function formatSize(bytes: number): string {
  return `${(bytes / 1024 / 1024).toFixed(0)} MB`
}

import type { Strings } from './zh-CN'

const strings: Strings = {
  siteName: 'MailX',
  description: 'A local-first, AI-native email client: fast, light, and every mailbox in one place. For macOS, Windows and Linux, free and open source.',
  nav: { download: 'Download', docs: 'Docs', changelog: 'Changelog', github: 'GitHub' },
  home: {
    title: 'MailX: a local-first, AI-native email client',
    headline: 'A local-first, AI-native email client',
    tagline: 'Fast, light, and every mailbox in one place.',
    download: 'Download for free',
    docs: 'Read the guide',
    platforms: 'For macOS, Windows and Linux · Free and open source',
    screenshotAlt: 'The MailX window: mailboxes on the left, the inbox in the middle, the AI assistant on the right',
    featuresTitle: 'Email, made a little simpler',
    features: [
      { title: 'Every mailbox, one inbox', body: 'Gmail, iCloud, Outlook and more sit side by side, so you stop switching between apps.' },
      { title: 'Your mail stays on your computer', body: 'No MailX account, and your mail never passes through our servers. It opens instantly, and mail you have downloaded is there even offline.' },
      { title: 'AI that reads and writes with you', body: 'Ask “what needs a reply today?”, have a long email summarized or an answer drafted. It uses the AI you choose, and asks before anything is sent.' },
      { title: 'Fast from the first click', body: 'Your mail is already there when MailX opens, and new mail shows up the moment it arrives.' },
      { title: 'Read with peace of mind', body: 'Pictures in mail wait for your OK, and messages open in a sealed-off view that cannot reach the app itself.' },
      { title: 'Translate in one click', body: 'Read mail in any language: translate it in place and keep its layout.' },
    ],
    closingTitle: 'Try MailX today',
    closingBody: 'Free and open source, with no ads and no usage tracking.',
  },
  download: {
    title: 'Download MailX',
    lead: 'Pick your system, open the file you download and follow the steps.',
    version: (version: string, date: string) => `Latest version ${version} · released ${date}`,
    releaseNotes: 'What’s new in this version',
    recommended: 'For this computer',
    macos: { name: 'macOS', note: 'Notarized by Apple, so it opens right away.', arm: 'Apple silicon (M series)', intel: 'Intel', which: 'Not sure which one? Open the Apple menu → About This Mac and look at “Chip” or “Processor”.' },
    windows: { name: 'Windows', note: 'For 64-bit Windows. If a blue warning appears the first time, choose “More info” → “Run anyway”.', setup: 'Download the installer' },
    linux: { name: 'Linux', note: 'The AppImage updates itself from inside the app; deb and rpm need each new version installed by hand.', appimage: 'AppImage', deb: 'deb (Debian / Ubuntu)', rpm: 'rpm (Fedora / openSUSE)' },
    fallback: 'The latest version could not be read right now. Download it from GitHub instead.',
    allReleases: 'All versions',
    checksums: 'Checksums (SHA-256)',
    updates: 'Once installed, MailX lets you know when a new version is ready.',
  },
  changelog: {
    title: 'Changelog',
    lead: 'What changed in each version of MailX.',
  },
  footer: { license: 'Open source under Apache-2.0', privacy: 'Privacy' },
}

export default strings

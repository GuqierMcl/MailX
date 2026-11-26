// @ts-check
import { defineConfig } from 'astro/config'
import starlight from '@astrojs/starlight'
import { DEFAULT_LOCALE, setLanguageCookie } from './src/lib/locale.mjs'
import { motionScript } from './src/lib/motion.mjs'
import { sidebar } from './src/lib/sidebar.mjs'

export default defineConfig({
  site: 'https://mailxapp.com',
  trailingSlash: 'always',
  // In production the Worker answers `/` first; this covers `astro dev`.
  redirects: { '/': `/${DEFAULT_LOCALE}/` },
  // The guides and screenshots live outside this package (docs/guides,
  // docs/assets/screenshots) and are read in place, never copied.
  vite: { server: { fs: { allow: ['..'] } } },
  integrations: [
    starlight({
      title: { 'zh-CN': 'MailX 文档', en: 'MailX Docs' },
      logo: {
        light: './src/assets/brand/logo-horizontal-black.svg',
        dark: './src/assets/brand/logo-horizontal-white.svg',
        replacesTitle: true,
      },
      favicon: '/favicon.svg',
      defaultLocale: DEFAULT_LOCALE,
      locales: {
        'zh-CN': { label: '简体中文', lang: 'zh-CN' },
        en: { label: 'English', lang: 'en' },
      },
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/GuqierMcl/MailX' }],
      sidebar,
      components: {
        SiteTitle: './src/components/SiteTitle.astro',
        SocialIcons: './src/components/DocsLinks.astro',
      },
      customCss: ['./src/styles/tokens.css', './src/styles/starlight.css'],
      head: [
        { tag: 'link', attrs: { rel: 'icon', href: '/favicon.ico', sizes: '32x32' } },
        { tag: 'script', content: setLanguageCookie },
        { tag: 'script', content: motionScript },
      ],
      // Guide paths are relative to this package (../docs/guides/…); the
      // browser resolves the dot segment, so the link opens docs/guides/….
      editLink: { baseUrl: 'https://github.com/GuqierMcl/MailX/edit/main/website/' },
      lastUpdated: false,
    }),
  ],
})

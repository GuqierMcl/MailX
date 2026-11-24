import { chooseLocale, localizedPath } from './language'

/**
 * Cloudflare serves the static pages directly. This Worker only answers
 * addresses without a language (`run_worker_first` in wrangler.jsonc), such as
 * `/` or `/docs/privacy/`, and sends the visitor to their language:
 *
 * - the `mailx-lang` cookie first: every page sets it to its own language;
 * - then the browser's preferred language: Chinese gets `/zh-CN/`, others `/en/`;
 * - neither (most crawlers) gets the default language.
 *
 * Addresses that name a language are never redirected, so a shared link always
 * opens the page it names. workerd accepts only handlers as exports of this
 * module, so the helpers live in language.ts.
 */
interface Env {
  ASSETS: { fetch(request: Request): Promise<Response> }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method !== 'GET' && request.method !== 'HEAD') return env.ASSETS.fetch(request)
    const url = new URL(request.url)
    return new Response(null, {
      status: 302,
      headers: {
        Location: `${localizedPath(chooseLocale(request), url.pathname)}${url.search}`,
        // The answer depends on the visitor, so no cache may share it.
        Vary: 'Accept-Language, Cookie',
        'Cache-Control': 'private, no-store',
      },
    })
  },
}

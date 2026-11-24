import { defineCollection } from 'astro:content'
import { glob } from 'astro/loaders'
import { docsSchema } from '@astrojs/starlight/schema'
import { guideId } from './lib/guides'

// The user guide is the repository's docs/guides, read in place:
// docs/guides/<locale>/<page>.md → /<locale>/docs/<page>/. The older README.md
// guides in that directory are left out.
export const collections = {
  docs: defineCollection({
    loader: glob({
      base: '../docs/guides',
      pattern: ['zh-CN/**/*.{md,mdx}', 'en/**/*.{md,mdx}', '!**/README.md'],
      generateId: ({ entry }) => guideId(entry),
    }),
    schema: docsSchema(),
  }),
}

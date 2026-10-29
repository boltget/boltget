// Content collections. Long-form copy lives here (Markdown/MDX and data
// files); short UI copy lives in the string catalog, src/i18n/en.json.
import { defineCollection } from "astro:content";
import { file, glob } from "astro/loaders";
import { z } from "astro/zod";
import { docsLoader, i18nLoader } from "@astrojs/starlight/loaders";
import { docsSchema, i18nSchema } from "@astrojs/starlight/schema";

export const collections = {
  // User docs, served by Starlight under /docs.
  docs: defineCollection({ loader: docsLoader(), schema: docsSchema() }),

  // Overrides for Starlight's own interface strings (src/content/i18n/en.json).
  i18n: defineCollection({ loader: i18nLoader(), schema: i18nSchema() }),

  // Long-form marketing pages (privacy, notes on the benchmark page, ...).
  pages: defineCollection({
    loader: glob({ pattern: "*.mdx", base: "./src/content/pages" }),
    schema: z.object({
      title: z.string(),
      description: z.string(),
    }),
  }),

  // The repository's CHANGELOG.md, rendered at build time for /changelog.
  changelog: defineCollection({
    loader: glob({ pattern: "CHANGELOG.md", base: "../.." }),
  }),

  // Rows of the BoltGet and IDM comparison, derived from docs/FEATURES.md.
  compare: defineCollection({
    loader: file("src/data/compare-idm.json"),
    schema: z.object({
      group: z.string(),
      feature: z.string(),
      boltget: z.string(),
      milestone: z.string(),
      idm: z.string(),
    }),
  }),
};

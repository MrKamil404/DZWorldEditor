import { defineConfig } from 'vitepress'
import { fileURLToPath } from 'node:url'
import { readFileSync } from 'node:fs'

const cargoToml = readFileSync(new URL('../../Cargo.toml', import.meta.url), 'utf8')
const appVersion = (cargoToml.match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1] ?? '0.4.2').replace('+', '.')

const SITE_URL = 'https://mrkamil404.github.io/DZWorldEditor'
const OG_IMAGE = `${SITE_URL}/og-preview.jpg`
const DEFAULT_DESC = 'A complete DayZ map editor in one program. Shape terrain, paint surfaces, place objects, draw roads, generate forests and rivers, edit missions, and build maps.'

const pageDescriptions: Record<string, string> = {
  'index.md': DEFAULT_DESC,
  'guide.md': 'Complete guide to DZ World Editor: interface layout, 3D and 2D views, tools, map config, map check, and one-click PBO build and play.',
  'real-places.md': 'Generate a playable DayZ map from any real place on Earth using satellite orthophotos, elevation data, and OpenStreetMap — no API key needed.',
  'objects.md': 'Place, snap, rotate, scale, and brush DayZ models from your P:\\ drive, build prefabs, and lay rows of walls and fences with the copy gizmo.',
  'terrain.md': 'Non-destructive DayZ terrain sculpting: brushes, terrain holes, procedural heightmaps, hydraulic and thermal erosion, terraces, and terrain stamps.',
  'surfaces.md': 'Paint DayZ surface masks within per-tile engine limits, derive masks from satellite imagery, and configure custom CfgSurfaces and layer materials.',
  'roads.md': 'Build DayZ roads and railways from vanilla Chernarus, Livonia, Sakhal, and Takistan parts with embankments, bridges, crossroads, and roundabouts.',
  'shapes.md': 'Draw and import polygons, polylines, points, and Terrain Builder compatible key points to drive generators and surface masks.',
  'satellite.md': 'Bake objects, roads, decals, and realistic water with depth shading and shoreline foam into the DayZ satellite image, plus clone brush tools.',
  'generators.md': 'Use built-in procedural generators for forests, rivers, ponds, coasts, fields, settlements, fences, and power lines, and share .dzwelib packages.',
  'team-work.md': 'Collaborate on a DayZ map with a team using visual diff previews, object ownership, and ordered patch merges through a shared folder.',
  'mission-editor.md': 'Edit DayZ server mission Central Economy files (types.xml, events.xml, spawns) and 3D loot points for mapgroupproto.xml directly on the map.',
  'changelog.md': 'Release history and changelog for DZ World Editor — new features, improvements, and bug fixes for DayZ map creators.',
}

const softwareJsonLd = JSON.stringify({
  '@context': 'https://schema.org',
  '@type': 'SoftwareApplication',
  name: 'DZ World Editor',
  applicationCategory: 'DesignApplication',
  operatingSystem: 'Windows 10, Windows 11',
  softwareVersion: appVersion,
  description: DEFAULT_DESC,
  url: `${SITE_URL}/`,
  downloadUrl: 'https://github.com/MrKamil404/DZWorldEditor/releases/latest',
  screenshot: OG_IMAGE,
  author: {
    '@type': 'Person',
    name: 'MrKamil404',
    url: 'https://github.com/MrKamil404',
  },
  offers: {
    '@type': 'Offer',
    price: '0',
    priceCurrency: 'USD',
  },
})

export default defineConfig({
  title: 'DZ World Editor',
  description: DEFAULT_DESC,
  base: '/DZWorldEditor/',
  srcDir: '../docs',
  outDir: './dist',
  cacheDir: './.cache',
  appearance: 'force-dark',
  cleanUrls: true,
  ignoreDeadLinks: [/README$/, /^\.\/docs\//],

  sitemap: {
    hostname: `${SITE_URL}/`,
  },

  transformPageData(pageData) {
    const desc = pageDescriptions[pageData.relativePath]
    if (desc && !pageData.frontmatter.description) {
      pageData.description = desc
    }
  },

  transformHead({ pageData }) {
    const slug = pageData.relativePath
      .replace(/(^|\/)index\.md$/, '$1')
      .replace(/\.md$/, '')
    const canonicalUrl = slug ? `${SITE_URL}/${slug}` : `${SITE_URL}/`
    const title = pageData.relativePath === 'index.md'
      ? 'DZ World Editor — DayZ Map Editor'
      : `${pageData.title} | DZ World Editor`
    const description = pageData.description || DEFAULT_DESC

    const head: Array<[string, Record<string, string>, string?]> = [
      ['link', { rel: 'canonical', href: canonicalUrl }],
      ['meta', { property: 'og:url', content: canonicalUrl }],
      ['meta', { property: 'og:title', content: title }],
      ['meta', { property: 'og:description', content: description }],
      ['meta', { property: 'og:image', content: OG_IMAGE }],
      ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
      ['meta', { name: 'twitter:title', content: title }],
      ['meta', { name: 'twitter:description', content: description }],
      ['meta', { name: 'twitter:image', content: OG_IMAGE }],
    ]

    if (pageData.relativePath === 'index.md') {
      head.push(['script', { type: 'application/ld+json' }, softwareJsonLd])
    }

    return head
  },

  vite: {
    define: {
      __APP_VERSION__: JSON.stringify(appVersion),
    },
    resolve: {
      alias: {
        vue: fileURLToPath(new URL('../node_modules/vue', import.meta.url)),
      },
    },
  },

  markdown: {
    config(md) {
      md.core.ruler.after('inline', 'dzwe-rewrite-links', (state) => {
        for (const token of state.tokens) {
          if (token.type !== 'inline' || !token.children) continue
          for (const child of token.children) {
            if (child.type !== 'link_open') continue
            const hrefIdx = child.attrIndex('href')
            if (hrefIdx < 0) continue
            const href = child.attrs![hrefIdx][1]
            if (href === '../README.md') {
              child.attrs![hrefIdx][1] = 'https://github.com/MrKamil404/DZWorldEditor#building-a-map'
            } else if (href.startsWith('docs/') && href.endsWith('.md')) {
              child.attrs![hrefIdx][1] = './' + href.slice('docs/'.length)
            }
          }
        }
      })
    },
  },

  head: [
    ['link', { rel: 'icon', href: '/DZWorldEditor/favicon.ico' }],
    ['meta', { name: 'theme-color', content: '#1a1a1a' }],
    ['meta', { name: 'google-site-verification', content: 'o9xwHng4X7ljkIfQuvXpu2GYxG9mPpwU2LKOptXAPJ0' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:site_name', content: 'DZ World Editor' }],
  ],

  themeConfig: {
    logo: '/favicon.ico',
    siteTitle: 'DZ World Editor',

    nav: [
      { text: 'Guide', link: '/guide' },
      { text: 'Real Places', link: '/real-places' },
      { text: 'Generators', link: '/generators' },
      { text: 'Changelog', link: '/changelog' },
      {
        text: 'Download',
        link: 'https://github.com/MrKamil404/DZWorldEditor/releases/latest',
      },
    ],

    sidebar: [
      {
        text: 'Getting Started',
        items: [
          { text: 'Editor Guide', link: '/guide' },
          { text: 'Maps of Real Places', link: '/real-places' },
        ],
      },
      {
        text: 'Map Tools',
        items: [
          { text: 'Objects & Placement', link: '/objects' },
          { text: 'Terrain & Sculpting', link: '/terrain' },
          { text: 'Surfaces & Painting', link: '/surfaces' },
          { text: 'Roads & Railways', link: '/roads' },
          { text: 'Shapes & Key Points', link: '/shapes' },
          { text: 'Satellite Image & Water', link: '/satellite' },
          { text: 'Generators & Library', link: '/generators' },
        ],
      },
      {
        text: 'Workflow & Server',
        items: [
          { text: 'Team Work', link: '/team-work' },
          { text: 'Mission Editor', link: '/mission-editor' },
        ],
      },
      {
        text: 'Project',
        items: [
          { text: 'Changelog', link: '/changelog' },
          {
            text: 'Latest Release',
            link: 'https://github.com/MrKamil404/DZWorldEditor/releases/latest',
          },
          {
            text: 'Support the Project',
            link: 'https://buymeacoffee.com/mrkamil404',
          },
        ],
      },
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/MrKamil404/DZWorldEditor' },
    ],

    search: {
      provider: 'local',
      options: {
        detailedView: true,
      },
    },

    outline: {
      level: [2, 3],
      label: 'Details',
    },

    editLink: {
      pattern: 'https://github.com/MrKamil404/DZWorldEditor/edit/main/docs/:path',
      text: 'Edit this page on GitHub',
    },

    docFooter: {
      prev: 'Previous page',
      next: 'Next page',
    },
  },
})

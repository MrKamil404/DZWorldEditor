import { defineConfig } from 'vitepress'
import { fileURLToPath } from 'node:url'
import { readFileSync } from 'node:fs'

const cargoToml = readFileSync(new URL('../../Cargo.toml', import.meta.url), 'utf8')
const appVersion = cargoToml.match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1] ?? '0.4.2'

export default defineConfig({
  title: 'DZ World Editor',
  description: 'A complete DayZ map editor in one program. Shape terrain, paint surfaces, place objects, draw roads, generate forests and rivers, edit missions, and build maps.',
  base: '/DZWorldEditor/',
  srcDir: '../docs',
  outDir: './dist',
  cacheDir: './.cache',
  appearance: 'force-dark',
  cleanUrls: true,
  ignoreDeadLinks: [/README$/, /^\.\/docs\//],

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

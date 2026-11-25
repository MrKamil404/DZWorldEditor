<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { withBase } from 'vitepress'

import townImg from '../../../docs/screenshots/town.jpg'
import splitImg from '../../../docs/screenshots/split.jpg'
import map2dImg from '../../../docs/screenshots/map2d.jpg'
import realplaceImg from '../../../docs/screenshots/realplace.jpg'
import realplaceMapImg from '../../../docs/screenshots/realplace-map.jpg'
import sculptImg from '../../../docs/screenshots/sculpt.jpg'
import paintImg from '../../../docs/screenshots/paint.jpg'
import roadsImg from '../../../docs/screenshots/roads.jpg'
import bridgeImg from '../../../docs/screenshots/bridge.jpg'
import rowsImg from '../../../docs/screenshots/rows.jpg'
import generatorsImg from '../../../docs/screenshots/generators.jpg'
import forestImg from '../../../docs/screenshots/forest.jpg'
import satelliteImg from '../../../docs/screenshots/satellite.jpg'
import keypointsImg from '../../../docs/screenshots/keypoints.jpg'
import missionImg from '../../../docs/screenshots/mission.jpg'

interface ShowcaseTab {
  id: string
  key: string
  label: string
  img: string
  caption: string
  statusLeft: string
  statusRight: string
  color: string
  doc: string
}

const tabs: ShowcaseTab[] = [
  {
    id: 'town',
    key: '1',
    label: '3D View',
    img: townImg,
    caption: 'Live 3D viewport with real DayZ models, textures, satellite image and water.',
    statusLeft: 'X 4594.2  Z 4069.1  height 8.84 m  slope 0.9°  surface grass',
    statusRight: 'Balticrus | 15.36 km | 172,131 objects',
    color: 'var(--dz-kind-model)',
    doc: '/guide',
  },
  {
    id: 'split',
    key: 'Ctrl+3',
    label: 'Split 3D | 2D',
    img: splitImg,
    caption: '3D and 2D views side by side, synchronized for fast navigation and editing.',
    statusLeft: 'Split view | 3D Perspective + 2D Map',
    statusRight: 'Ctrl+1 3D · Ctrl+2 2D · Ctrl+3 Split · F11 Fullscreen',
    color: 'var(--dz-kind-image)',
    doc: '/guide',
  },
  {
    id: 'realplace',
    key: 'New',
    label: 'Real Places',
    img: realplaceMapImg,
    caption: 'Generate a playable DayZ map from any place on Earth using satellite orthophotos, elevation and OpenStreetMap.',
    statusLeft: 'Zakopane | GUGiK orthophoto + OpenStreetMap shapes',
    statusRight: 'No API key required',
    color: 'var(--dz-kind-terrain)',
    doc: '/real-places',
  },
  {
    id: 'sculpt',
    key: '2',
    label: 'Sculpt',
    img: sculptImg,
    caption: 'Non-destructive terrain brushes, holes, erosion, terraces and terrain stamps.',
    statusLeft: 'Mode: Sculpt | Ctrl inverts · Shift smooths',
    statusRight: 'Non-destructive terrain modifiers',
    color: 'var(--dz-kind-terrain)',
    doc: '/terrain',
  },
  {
    id: 'paint',
    key: '3',
    label: 'Paint',
    img: paintImg,
    caption: 'Surface mask painting with per-tile game limits, CfgSurfaces editor and material preview.',
    statusLeft: 'Mode: Paint | Up to 6 surfaces per tile',
    statusRight: 'CfgSurfaces & layers.cfg ready',
    color: 'var(--dz-kind-layer)',
    doc: '/surfaces',
  },
  {
    id: 'roads',
    key: '5',
    label: 'Roads',
    img: roadsImg,
    caption: 'Roads and railways built from vanilla DayZ pieces with graded beds, embankments, bridges and roundabouts.',
    statusLeft: 'Mode: Roads | Chernarus · Livonia · Sakhal · Takistan',
    statusRight: 'Crossroads (kr_*) & bridges',
    color: 'var(--dz-kind-road)',
    doc: '/roads',
  },
  {
    id: 'generators',
    key: 'Gen',
    label: 'Generators',
    img: forestImg,
    caption: 'Rule-based generators for forests, rivers, ponds, coasts, fields, fences, power lines and settlements.',
    statusLeft: 'Content Browser | Drag onto a shape or road',
    statusRight: 'Shareable as .dzwelib',
    color: 'var(--dz-kind-generator)',
    doc: '/generators',
  },
  {
    id: 'satellite',
    key: '8',
    label: 'Satellite',
    img: satelliteImg,
    caption: 'Bake objects, roads, decals and realistic water into the satellite image, plus clone brush and mask conversion.',
    statusLeft: 'Mode: Satellite | Water depth shading & shoreline foam',
    statusRight: 'Non-destructive satellite modifiers',
    color: 'var(--dz-kind-image)',
    doc: '/satellite',
  },
  {
    id: 'mission',
    key: '9',
    label: 'Mission',
    img: missionImg,
    caption: 'Edit Central Economy loot tables, events, spawns and 3D loot points for mapgroupproto.xml.',
    statusLeft: 'Mode: Mission (experimental) | Central Economy',
    statusRight: 'Packs directly into ce.pbo',
    color: 'var(--dz-kind-point)',
    doc: '/mission-editor',
  },
]

const activeTabId = ref('town')
const currentTab = () => tabs.find((t) => t.id === activeTabId.value) || tabs[0]

interface FeatureCard {
  title: string
  category: string
  color: string
  desc: string
  link: string
  linkLabel: string
}

const features: FeatureCard[] = [
  {
    title: 'Live 3D & 2D Views',
    category: 'Viewport',
    color: 'var(--dz-kind-image)',
    desc: 'Inspect the entire map with real DayZ models, textures, water, clutter distance, slope & density heatmaps, split view, minimap and saved camera views.',
    link: '/guide',
    linkLabel: 'Editor Guide',
  },
  {
    title: 'Maps of Real Places',
    category: 'World Creation',
    color: 'var(--dz-kind-terrain)',
    desc: 'Pick any location on Earth and a map size. Downloads satellite imagery, elevation and OSM data, paints surfaces, places buildings, builds roads and adds forest & river generators.',
    link: '/real-places',
    linkLabel: 'Real Places',
  },
  {
    title: 'Terrain & Modifiers',
    category: 'Sculpt',
    color: 'var(--dz-kind-terrain)',
    desc: 'Sculpting brushes, terrain holes, procedural terrain, hydraulic & thermal erosion, terraces and terrain stamps — all stored as non-destructive modifiers.',
    link: '/terrain',
    linkLabel: 'Terrain Guide',
  },
  {
    title: 'Surfaces & CfgSurfaces',
    category: 'Paint',
    color: 'var(--dz-kind-layer)',
    desc: 'Paint up to six surfaces per tile matching DayZ engine limits, read masks from satellite imagery, and create custom map surfaces with grass, sounds and physics.',
    link: '/surfaces',
    linkLabel: 'Surfaces Guide',
  },
  {
    title: 'Objects, Prefabs & Rows',
    category: 'Place',
    color: 'var(--dz-kind-model)',
    desc: 'Searchable P:\\ model library, transform gizmos, angle & surface snapping, brush placement, prefabs, and a copy gizmo for laying rows of walls and fences.',
    link: '/objects',
    linkLabel: 'Objects Guide',
  },
  {
    title: 'Roads & Railways',
    category: 'Routes',
    color: 'var(--dz-kind-road)',
    desc: 'Fit roads and railways from vanilla Chernarus, Livonia, Sakhal and Takistan parts with embankments, bridges, crossroads (kr_*), level crossings and roundabouts.',
    link: '/roads',
    linkLabel: 'Roads Guide',
  },
  {
    title: 'Procedural Generators',
    category: 'Content Browser',
    color: 'var(--dz-kind-generator)',
    desc: 'Populate forests, rivers, ponds, coasts, cliffs, fields, road verges, settlements, fences and power lines. Share presets and prefabs as .dzwelib packages.',
    link: '/generators',
    linkLabel: 'Generators Guide',
  },
  {
    title: 'Satellite & Water Tools',
    category: 'Satellite',
    color: 'var(--dz-kind-image)',
    desc: 'Bake objects, roads and decals into the satellite image, paint realistic ocean, river and pond water with depth shading and shoreline foam, and use the clone brush.',
    link: '/satellite',
    linkLabel: 'Satellite Guide',
  },
  {
    title: 'One-Click Build & Play',
    category: 'Build (F7 / F5)',
    color: 'var(--dz-play)',
    desc: 'Run a full map check, export to P:\\, binarise with AddonBuilder into signed PBOs (including surfaces.pbo and ce.pbo), and launch a local DayZDiag server & client.',
    link: '/guide#build',
    linkLabel: 'Build Pipeline',
  },
  {
    title: 'Team Work & Mission Editor',
    category: 'Collaboration',
    color: 'var(--dz-kind-shape)',
    desc: 'Work with a team using visual diff previews and ordered patch merges through a shared folder, plus an experimental Central Economy & 3D loot point editor.',
    link: '/team-work',
    linkLabel: 'Team Work',
  },
]

interface GalleryItem {
  img: string
  title: string
  desc: string
}

const gallery: GalleryItem[] = [
  { img: townImg, title: '3D Viewport', desc: 'A town in the live 3D view with real DayZ assets and water' },
  { img: splitImg, title: 'Split View', desc: '3D and 2D views side by side' },
  { img: map2dImg, title: '2D Map View', desc: 'Top-down map view with roads and town plans' },
  { img: realplaceImg, title: 'Real Place Wizard', desc: 'Choosing Zakopane on the Polish orthophoto' },
  { img: realplaceMapImg, title: 'Generated Real Map', desc: 'DayZ buildings placed in real footprints with OSM forests' },
  { img: sculptImg, title: 'Terrain Sculpting', desc: 'Non-destructive sculpting brushes and modifiers' },
  { img: paintImg, title: 'Surface Painting', desc: 'Surface layers with the mask shown directly on the terrain' },
  { img: roadsImg, title: 'Roads & Roundabouts', desc: 'Graded beds, embankments and vanilla road parts' },
  { img: bridgeImg, title: 'Bridges & Rivers', desc: 'Stone bridge over a river from a Terrain Builder project' },
  { img: rowsImg, title: 'Rows of Walls & Fences', desc: 'Laid end to end around corners with the copy gizmo' },
  { img: generatorsImg, title: 'Generator Library', desc: 'Built-in generators and presets in the Content Browser' },
  { img: forestImg, title: 'Forest Generator', desc: 'Filling an area with settings live in the Details panel' },
  { img: satelliteImg, title: 'Satellite Image Tools', desc: 'Baking objects, realistic water and clone brush' },
  { img: keypointsImg, title: 'Key Points', desc: 'Named places compatible with Terrain Builder' },
  { img: missionImg, title: 'Mission Editor', desc: 'Central Economy loot tables and server mission tools' },
]

const shortcuts = [
  { keys: '1 – 9', action: 'Switch tools (Select, Sculpt, Paint, Place, Roads, Shapes, Key points, Satellite, Mission)' },
  { keys: 'RMB + WASD / QE', action: 'Fly 3D camera (Shift faster, mouse wheel changes speed)' },
  { keys: 'Ctrl+1 / Ctrl+2 / Ctrl+3', action: 'Switch between 3D view, 2D view, and Split view' },
  { keys: 'W / E / R / T', action: 'Move, rotate, scale and copy-row gizmos' },
  { keys: 'F7', action: 'Run map check, export and Build PBOs' },
  { keys: 'F5', action: 'Build and Play in a local DayZDiag server & client' },
  { keys: 'F1 / F2', action: 'Edit keyboard shortcuts / show current tool keys' },
  { keys: 'F11', action: 'Toggle fullscreen viewport' },
]

const lightboxIndex = ref<number | null>(null)

function openLightbox(idx: number) {
  lightboxIndex.value = idx
}

function closeLightbox() {
  lightboxIndex.value = null
}

function stepLightbox(delta: number) {
  if (lightboxIndex.value === null) return
  lightboxIndex.value = (lightboxIndex.value + delta + gallery.length) % gallery.length
}

function onKeyDown(e: KeyboardEvent) {
  if (lightboxIndex.value === null) return
  if (e.key === 'Escape') closeLightbox()
  else if (e.key === 'ArrowRight') stepLightbox(1)
  else if (e.key === 'ArrowLeft') stepLightbox(-1)
}

onMounted(() => window.addEventListener('keydown', onKeyDown))
onUnmounted(() => window.removeEventListener('keydown', onKeyDown))
</script>

<template>
  <div class="dz-home">
    <!-- HERO -->
    <section class="dz-hero">
      <div class="dz-container">
        <div class="dz-hero-top">
          <div class="dz-badges">
            <span class="dz-badge dz-badge-accent">v0.4.1</span>
            <span class="dz-badge">Windows 10 / 11</span>
            <span class="dz-badge">OpenGL 3.3</span>
            <span class="dz-badge">Written in Rust</span>
            <span class="dz-badge">EN · PL · DE</span>
          </div>

          <h1 class="dz-hero-title">DZ World Editor</h1>
          <p class="dz-hero-subtitle">
            A complete DayZ map editor in one program. Shape terrain, paint surfaces, place objects,
            draw roads and railways, generate forests and rivers, edit the server mission, then build
            the map and start it in the game — without leaving the editor.
          </p>

          <div class="dz-actions">
            <a
              class="dz-btn dz-btn-play"
              href="https://github.com/MrKamil404/DZWorldEditor/releases/latest"
              target="_blank"
              rel="noopener"
            >
              <span class="dz-btn-icon">▶</span>
              Download Latest Release
            </a>
            <a class="dz-btn dz-btn-accent" :href="withBase('/guide')">
              Editor Guide
            </a>
            <a class="dz-btn dz-btn-secondary" :href="withBase('/real-places')">
              Maps of Real Places
            </a>
            <a
              class="dz-btn dz-btn-coffee"
              href="https://buymeacoffee.com/mrkamil404"
              target="_blank"
              rel="noopener"
            >
              Buy me a coffee
            </a>
          </div>
        </div>

        <!-- INTERACTIVE EDITOR VIEWPORT SHOWCASE -->
        <div class="dz-editor-shell">
          <div class="dz-editor-toolbar">
            <div class="dz-tool-group" role="tablist" aria-label="Editor views">
              <button
                v-for="tab in tabs"
                :key="tab.id"
                type="button"
                role="tab"
                :aria-selected="activeTabId === tab.id"
                :class="['dz-tool-btn', { active: activeTabId === tab.id }]"
                @click="activeTabId = tab.id"
              >
                <span class="dz-tool-dot" :style="{ backgroundColor: tab.color }"></span>
                <span class="dz-tool-label">{{ tab.label }}</span>
                <kbd class="dz-tool-key">{{ tab.key }}</kbd>
              </button>
            </div>
          </div>

          <div class="dz-viewport-panel">
            <div class="dz-tab-strip">
              <div class="dz-tab-active">
                <span class="dz-tab-dot" :style="{ backgroundColor: currentTab().color }"></span>
                Viewport — {{ currentTab().label }}
              </div>
              <a class="dz-tab-doc-link" :href="withBase(currentTab().doc)">
                Open docs →
              </a>
            </div>
            <div class="dz-viewport-frame" @click="openLightbox(gallery.findIndex(g => g.img === currentTab().img))">
              <img :src="currentTab().img" :alt="currentTab().label" />
              <div class="dz-viewport-overlay">
                <span>{{ currentTab().caption }}</span>
                <span class="dz-zoom-hint">Click to enlarge</span>
              </div>
            </div>
          </div>

          <div class="dz-status-bar">
            <span class="dz-status-left">{{ currentTab().statusLeft }}</span>
            <span class="dz-status-right">{{ currentTab().statusRight }}</span>
          </div>
        </div>
      </div>
    </section>

    <!-- FEATURES -->
    <section class="dz-section-block">
      <div class="dz-container">
        <div class="dz-section-heading">
          <h2>Everything you need to build a DayZ map</h2>
          <p>
            From importing Terrain Builder projects or real-world places to binarising signed PBOs
            and launching a local server.
          </p>
        </div>

        <div class="dz-feature-grid">
          <a
            v-for="feat in features"
            :key="feat.title"
            class="dz-panel-card"
            :href="withBase(feat.link)"
          >
            <div class="dz-panel-header">
              <span class="dz-arrow">▾</span>
              <span class="dz-kind-dot" :style="{ backgroundColor: feat.color }"></span>
              <span class="dz-panel-title">{{ feat.title }}</span>
              <span class="dz-panel-cat">{{ feat.category }}</span>
            </div>
            <div class="dz-panel-body">
              <p>{{ feat.desc }}</p>
              <span class="dz-panel-link">{{ feat.linkLabel }} →</span>
            </div>
          </a>
        </div>
      </div>
    </section>

    <!-- GETTING STARTED + SHORTCUTS -->
    <section class="dz-section-block dz-section-alt">
      <div class="dz-container">
        <div class="dz-split-panels">
          <!-- Left panel: Getting Started -->
          <div class="dz-dock-panel">
            <div class="dz-dock-tabbar">
              <span class="dz-dock-tab">Getting Started</span>
            </div>
            <div class="dz-dock-body">
              <ol class="dz-steps">
                <li>
                  <strong>Install prerequisites</strong>
                  <span>Install DayZ and DayZ Tools via Steam, and mount the <code>P:\</code> work drive.</span>
                </li>
                <li>
                  <strong>Download &amp; run</strong>
                  <span>
                    Grab <code>DZWorldEditor.exe</code> from the
                    <a href="https://github.com/MrKamil404/DZWorldEditor/releases/latest" target="_blank" rel="noopener">latest release</a>.
                    The editor includes a built-in updater.
                  </span>
                </li>
                <li>
                  <strong>Verify paths</strong>
                  <span>Check your DayZ and DayZ Tools folders in <code>Edit &gt; Settings</code>.</span>
                </li>
                <li>
                  <strong>Create or import</strong>
                  <span>Start a flat or real-world map with <code>File &gt; New project</code>, or import a Terrain Builder project, WRP, <code>.dze</code> or <code>cfggameplay.json</code>.</span>
                </li>
              </ol>

              <div class="dz-req-box">
                <div class="dz-req-title">Requirements &amp; Compatibility</div>
                <p>
                  Windows 10/11 · OpenGL 3.3 GPU · DayZ Tools &amp; <code>P:\</code> drive for models and building.
                  Supports Terrain Builder import/export, automatic space-saving backups, and English, Polish and German UI.
                </p>
              </div>
            </div>
          </div>

          <!-- Right panel: Shortcuts -->
          <div class="dz-dock-panel">
            <div class="dz-dock-tabbar">
              <span class="dz-dock-tab">Quick Shortcuts</span>
            </div>
            <div class="dz-dock-body dz-list-body">
              <div v-for="(sc, i) in shortcuts" :key="sc.keys" :class="['dz-list-row', { alt: i % 2 === 1 }]">
                <kbd class="dz-kbd">{{ sc.keys }}</kbd>
                <span class="dz-list-desc">{{ sc.action }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- SCREENSHOT GALLERY -->
    <section class="dz-section-block">
      <div class="dz-container">
        <div class="dz-section-heading">
          <h2>Screenshots</h2>
          <p>
            Balticrus imported from Terrain Builder, and Zakopane generated from open GUGiK orthophotos and OpenStreetMap.
            Click any screenshot to inspect it full-size.
          </p>
        </div>

        <div class="dz-gallery-grid">
          <button
            v-for="(item, idx) in gallery"
            :key="item.title"
            type="button"
            class="dz-gallery-card"
            @click="openLightbox(idx)"
          >
            <div class="dz-gallery-thumb">
              <img :src="item.img" :alt="item.title" loading="lazy" />
            </div>
            <div class="dz-gallery-meta">
              <div class="dz-gallery-title">{{ item.title }}</div>
              <div class="dz-gallery-desc">{{ item.desc }}</div>
            </div>
          </button>
        </div>
      </div>
    </section>

    <!-- FOOTER STATUS BAR -->
    <footer class="dz-footer">
      <div class="dz-container dz-footer-inner">
        <div class="dz-footer-left">
          <strong>DZ World Editor</strong> · Free &amp; open source · Built with Rust, egui &amp; OpenGL 3.3
        </div>
        <div class="dz-footer-links">
          <a href="https://github.com/MrKamil404/DZWorldEditor" target="_blank" rel="noopener">GitHub</a>
          <a href="https://github.com/MrKamil404/DZWorldEditor/releases/latest" target="_blank" rel="noopener">Releases</a>
          <a :href="withBase('/changelog')">Changelog</a>
          <a href="https://github.com/MrKamil404/DZWorldEditor/issues" target="_blank" rel="noopener">Issues</a>
          <a href="https://buymeacoffee.com/mrkamil404" target="_blank" rel="noopener">Support</a>
        </div>
      </div>
    </footer>

    <!-- LIGHTBOX MODAL -->
    <Teleport to="body">
      <div v-if="lightboxIndex !== null" class="dz-lightbox" @click.self="closeLightbox">
        <div class="dz-lightbox-window">
          <div class="dz-lightbox-bar">
            <span class="dz-lightbox-title">
              {{ gallery[lightboxIndex].title }}
              <span class="dz-lightbox-count">{{ lightboxIndex + 1 }} / {{ gallery.length }}</span>
            </span>
            <div class="dz-lightbox-controls">
              <button type="button" class="dz-lb-btn" @click="stepLightbox(-1)" aria-label="Previous">←</button>
              <button type="button" class="dz-lb-btn" @click="stepLightbox(1)" aria-label="Next">→</button>
              <button type="button" class="dz-lb-btn" @click="closeLightbox" aria-label="Close">✕</button>
            </div>
          </div>
          <div class="dz-lightbox-img-wrap">
            <img :src="gallery[lightboxIndex].img" :alt="gallery[lightboxIndex].title" />
          </div>
          <div class="dz-lightbox-caption">
            {{ gallery[lightboxIndex].desc }}
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.dz-home {
  background-color: var(--dz-gap);
  color: var(--dz-text);
  min-height: calc(100vh - var(--vp-nav-height));
}

.dz-container {
  max-width: 1220px;
  margin: 0 auto;
  padding: 0 24px;
}

/* HERO */
.dz-hero {
  background: linear-gradient(180deg, var(--dz-panel) 0%, var(--dz-gap) 100%);
  padding: 40px 0 48px;
  border-bottom: 1px solid var(--dz-border);
}

.dz-hero-top {
  text-align: center;
  max-width: 820px;
  margin: 0 auto 32px;
}

.dz-badges {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 6px;
  margin-bottom: 16px;
}

.dz-badge {
  background: var(--dz-section);
  border: 1px solid var(--dz-border);
  border-radius: 3px;
  padding: 2px 8px;
  font-size: 11.5px;
  font-family: var(--vp-font-family-mono);
  color: var(--dz-text);
}

.dz-badge-accent {
  background: var(--dz-toggle-on);
  border-color: var(--dz-accent);
  color: var(--dz-toggle-on-text);
  font-weight: 600;
}

.dz-hero-title {
  font-size: clamp(32px, 5vw, 46px);
  font-weight: 700;
  color: var(--dz-text-bright);
  letter-spacing: -0.02em;
  line-height: 1.15;
  margin: 0 0 14px;
}

.dz-hero-subtitle {
  font-size: 15.5px;
  line-height: 1.6;
  color: var(--dz-text);
  margin: 0 0 24px;
}

.dz-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 8px;
}

.dz-btn {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 36px;
  padding: 0 16px;
  border-radius: 3px;
  font-size: 13.5px;
  font-weight: 600;
  text-decoration: none;
  border: 1px solid transparent;
  transition: background-color 0.12s, border-color 0.12s, color 0.12s;
  cursor: pointer;
}

.dz-btn-icon {
  font-size: 11px;
}

.dz-btn-play {
  background: var(--dz-play);
  border-color: #3ca84d;
  color: #ffffff;
}

.dz-btn-play:hover {
  background: var(--dz-play-hover);
}

.dz-btn-accent {
  background: var(--dz-accent);
  border-color: #2188f0;
  color: #ffffff;
}

.dz-btn-accent:hover {
  background: var(--dz-accent-hover);
}

.dz-btn-secondary {
  background: var(--dz-section);
  border-color: var(--dz-border);
  color: var(--dz-text-bright);
}

.dz-btn-secondary:hover {
  background: var(--dz-hover);
  border-color: #505050;
}

.dz-btn-coffee {
  background: var(--dz-strip);
  border-color: rgba(226, 179, 60, 0.5);
  color: var(--dz-focus);
}

.dz-btn-coffee:hover {
  background: rgba(226, 179, 60, 0.12);
  border-color: var(--dz-focus);
}

/* EDITOR SHELL SHOWCASE */
.dz-editor-shell {
  background: var(--dz-gap);
  border: 1px solid #0a0a0a;
  border-radius: 4px;
  overflow: hidden;
  box-shadow: 0 14px 40px rgba(0, 0, 0, 0.6);
}

.dz-editor-toolbar {
  background: var(--dz-panel);
  padding: 6px 8px;
  border-bottom: 3px solid var(--dz-gap);
  overflow-x: auto;
}

.dz-tool-group {
  display: inline-flex;
  gap: 2px;
  background: #121212;
  border: 1px solid #2c2c2c;
  border-radius: 5px;
  padding: 2px;
  min-width: max-content;
}

.dz-tool-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border: 1px solid transparent;
  border-radius: 3px;
  background: transparent;
  color: var(--dz-text);
  font-size: 12.5px;
  font-weight: 500;
  cursor: pointer;
  transition: background-color 0.1s, color 0.1s;
}

.dz-tool-btn:hover {
  background: var(--dz-section);
  color: var(--dz-text-bright);
}

.dz-tool-btn.active {
  background: var(--dz-accent);
  color: #ffffff;
}

.dz-tool-dot {
  width: 7px;
  height: 7px;
  border-radius: 2px;
  flex-shrink: 0;
}

.dz-tool-btn.active .dz-tool-dot {
  background-color: #ffffff !important;
}

.dz-tool-key {
  font-family: var(--vp-font-family-mono);
  font-size: 10px;
  padding: 1px 4px;
  border-radius: 2px;
  background: rgba(0, 0, 0, 0.35);
  color: var(--dz-text-dim);
}

.dz-tool-btn.active .dz-tool-key {
  background: rgba(255, 255, 255, 0.2);
  color: #ffffff;
}

.dz-viewport-panel {
  background: var(--dz-bg);
}

.dz-tab-strip {
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: var(--dz-strip);
  height: 30px;
  padding: 0 10px 0 0;
}

.dz-tab-active {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 100%;
  padding: 0 14px;
  background: var(--dz-bg);
  color: var(--dz-text-bright);
  font-size: 12.5px;
  font-weight: 600;
  border-radius: 3px 3px 0 0;
}

.dz-tab-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
}

.dz-tab-doc-link {
  font-size: 12px;
  color: var(--dz-link);
  text-decoration: none;
}

.dz-tab-doc-link:hover {
  text-decoration: underline;
}

.dz-viewport-frame {
  position: relative;
  border: 2px solid var(--dz-focus);
  background: #0d0d0d;
  cursor: zoom-in;
  overflow: hidden;
}

.dz-viewport-frame img {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: cover;
}

.dz-viewport-overlay {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 10px 14px;
  background: linear-gradient(180deg, rgba(16, 16, 16, 0) 0%, rgba(16, 16, 16, 0.88) 45%);
  color: var(--dz-text-bright);
  font-size: 13px;
}

.dz-zoom-hint {
  font-size: 11px;
  font-family: var(--vp-font-family-mono);
  color: var(--dz-focus);
  white-space: nowrap;
}

.dz-status-bar {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 8px;
  background: var(--dz-strip);
  border-top: 1px solid var(--dz-gap);
  padding: 5px 12px;
  font-family: var(--vp-font-family-mono);
  font-size: 11px;
  color: var(--dz-text-dim);
}

.dz-status-right {
  color: var(--dz-text);
}

/* SECTIONS */
.dz-section-block {
  padding: 56px 0;
  border-bottom: 1px solid var(--dz-border);
}

.dz-section-alt {
  background: var(--dz-panel);
}

.dz-section-heading {
  margin-bottom: 28px;
}

.dz-section-heading h2 {
  font-size: 24px;
  font-weight: 600;
  color: var(--dz-text-bright);
  margin: 0 0 6px;
}

.dz-section-heading p {
  font-size: 14.5px;
  color: var(--dz-text-dim);
  margin: 0;
}

/* FEATURE CARDS — styled like Details sections */
.dz-feature-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 12px;
}

.dz-panel-card {
  display: flex;
  flex-direction: column;
  background: var(--dz-bg);
  border: 1px solid var(--dz-border);
  border-radius: 4px;
  text-decoration: none;
  overflow: hidden;
  transition: border-color 0.14s, transform 0.14s;
}

.dz-panel-card:hover {
  border-color: var(--dz-accent);
  transform: translateY(-1px);
}

.dz-panel-header {
  display: flex;
  align-items: center;
  gap: 7px;
  height: 30px;
  padding: 0 10px;
  background: var(--dz-section);
  border-bottom: 1px solid var(--dz-strip);
}

.dz-arrow {
  color: var(--dz-text-dim);
  font-size: 11px;
}

.dz-kind-dot {
  width: 8px;
  height: 8px;
  border-radius: 2px;
  flex-shrink: 0;
}

.dz-panel-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--dz-text-bright);
}

.dz-panel-cat {
  margin-left: auto;
  font-size: 11px;
  font-family: var(--vp-font-family-mono);
  color: var(--dz-text-dim);
}

.dz-panel-body {
  display: flex;
  flex-direction: column;
  flex: 1;
  padding: 12px 14px;
  gap: 10px;
}

.dz-panel-body p {
  margin: 0;
  font-size: 13.5px;
  line-height: 1.55;
  color: var(--dz-text);
}

.dz-panel-link {
  margin-top: auto;
  font-size: 12.5px;
  font-weight: 500;
  color: var(--dz-link);
}

/* SPLIT PANELS (Getting Started + Shortcuts) */
.dz-split-panels {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
  gap: 14px;
}

.dz-dock-panel {
  background: var(--dz-bg);
  border: 1px solid var(--dz-border);
  border-radius: 4px;
  overflow: hidden;
}

.dz-dock-tabbar {
  background: var(--dz-strip);
  height: 30px;
  display: flex;
  align-items: flex-end;
  padding: 0 8px;
}

.dz-dock-tab {
  background: var(--dz-bg);
  color: var(--dz-text-bright);
  font-size: 12.5px;
  font-weight: 600;
  padding: 5px 12px;
  border-radius: 3px 3px 0 0;
}

.dz-dock-body {
  padding: 16px;
}

.dz-steps {
  list-style: none;
  counter-reset: dz-step;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.dz-steps li {
  counter-increment: dz-step;
  position: relative;
  padding-left: 32px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-size: 13.5px;
  line-height: 1.5;
}

.dz-steps li::before {
  content: counter(dz-step);
  position: absolute;
  left: 0;
  top: 2px;
  width: 22px;
  height: 22px;
  border-radius: 3px;
  background: var(--dz-accent-soft);
  border: 1px solid var(--dz-accent);
  color: var(--dz-toggle-on-text);
  font-family: var(--vp-font-family-mono);
  font-size: 11.5px;
  font-weight: 600;
  display: flex;
  align-items: center;
  justify-content: center;
}

.dz-steps strong {
  color: var(--dz-text-bright);
}

.dz-steps code {
  background: var(--dz-dark);
  border: 1px solid var(--dz-border);
  border-radius: 3px;
  padding: 0 4px;
  font-size: 12px;
  font-family: var(--vp-font-family-mono);
  color: var(--dz-toggle-on-text);
}

.dz-steps a {
  color: var(--dz-link);
}

.dz-req-box {
  margin-top: 16px;
  padding: 10px 12px;
  background: var(--dz-dark);
  border: 1px solid var(--dz-border);
  border-left: 3px solid var(--dz-focus);
  border-radius: 3px;
}

.dz-req-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--dz-text-bright);
  margin-bottom: 4px;
}

.dz-req-box p {
  margin: 0;
  font-size: 12.5px;
  line-height: 1.5;
  color: var(--dz-text-dim);
}

.dz-list-body {
  padding: 6px;
}

.dz-list-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 7px 10px;
  border-radius: 2px;
  font-size: 13px;
}

.dz-list-row.alt {
  background: var(--dz-row-alt);
}

.dz-kbd {
  font-family: var(--vp-font-family-mono);
  font-size: 11.5px;
  padding: 2px 7px;
  border-radius: 3px;
  background: var(--dz-field);
  border: 1px solid var(--dz-border);
  color: var(--dz-toggle-on-text);
  white-space: nowrap;
  flex-shrink: 0;
}

.dz-list-desc {
  color: var(--dz-text);
  line-height: 1.4;
}

/* GALLERY */
.dz-gallery-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 12px;
}

.dz-gallery-card {
  text-align: left;
  background: var(--dz-bg);
  border: 1px solid var(--dz-border);
  border-radius: 4px;
  overflow: hidden;
  padding: 0;
  cursor: zoom-in;
  transition: border-color 0.14s, transform 0.14s;
}

.dz-gallery-card:hover {
  border-color: var(--dz-focus);
  transform: translateY(-1px);
}

.dz-gallery-thumb {
  background: var(--dz-dark);
  overflow: hidden;
}

.dz-gallery-thumb img {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 10;
  object-fit: cover;
  transition: transform 0.2s;
}

.dz-gallery-card:hover .dz-gallery-thumb img {
  transform: scale(1.02);
}

.dz-gallery-meta {
  padding: 10px 12px;
  border-top: 1px solid var(--dz-strip);
}

.dz-gallery-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--dz-text-bright);
  margin-bottom: 2px;
}

.dz-gallery-desc {
  font-size: 12px;
  color: var(--dz-text-dim);
  line-height: 1.4;
}

/* FOOTER */
.dz-footer {
  background: var(--dz-strip);
  padding: 18px 0;
  font-size: 12.5px;
  color: var(--dz-text-dim);
}

.dz-footer-inner {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}

.dz-footer-left strong {
  color: var(--dz-text-bright);
}

.dz-footer-links {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
}

.dz-footer-links a {
  color: var(--dz-text);
  text-decoration: none;
}

.dz-footer-links a:hover {
  color: var(--dz-link);
}

/* LIGHTBOX */
.dz-lightbox {
  position: fixed;
  inset: 0;
  z-index: 200;
  background: rgba(10, 10, 10, 0.88);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
}

.dz-lightbox-window {
  background: var(--dz-bg);
  border: 1px solid var(--dz-focus);
  border-radius: 4px;
  max-width: 1280px;
  width: 100%;
  max-height: 92vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.8);
}

.dz-lightbox-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  background: var(--dz-strip);
  padding: 8px 12px;
  border-bottom: 1px solid var(--dz-gap);
}

.dz-lightbox-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--dz-text-bright);
}

.dz-lightbox-count {
  margin-left: 8px;
  font-family: var(--vp-font-family-mono);
  font-size: 11.5px;
  color: var(--dz-text-dim);
  font-weight: 400;
}

.dz-lightbox-controls {
  display: flex;
  gap: 4px;
}

.dz-lb-btn {
  width: 28px;
  height: 26px;
  background: var(--dz-section);
  border: 1px solid var(--dz-border);
  border-radius: 3px;
  color: var(--dz-text-bright);
  cursor: pointer;
  font-size: 13px;
}

.dz-lb-btn:hover {
  background: var(--dz-hover);
  border-color: var(--dz-accent);
}

.dz-lightbox-img-wrap {
  background: #0d0d0d;
  overflow: auto;
  display: flex;
  justify-content: center;
  align-items: center;
  max-height: calc(92vh - 80px);
}

.dz-lightbox-img-wrap img {
  max-width: 100%;
  max-height: calc(92vh - 80px);
  display: block;
}

.dz-lightbox-caption {
  padding: 8px 14px;
  background: var(--dz-strip);
  border-top: 1px solid var(--dz-gap);
  font-size: 12.5px;
  color: var(--dz-text);
}

@media (max-width: 640px) {
  .dz-viewport-overlay {
    flex-direction: column;
    align-items: flex-start;
  }
  .dz-list-row {
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
  }
}
</style>

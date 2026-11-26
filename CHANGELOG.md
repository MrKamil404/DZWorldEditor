# Changelog

## 0.4.3

### New

- **Tile surface limit controls in Project settings.**
  - Changing **Surfaces per tile** no longer alters your mask automatically; instead, the editor
    prompts you to run **Check tiles**.
  - **Highlight excess pixels**: checking tiles can now mark the exact over-limit pixels directly in
    the 2D and 3D views alongside the tile outlines, making stray pixels easy to find.
  - **Configurable Fix options**: choose how excess pixels are replaced (**Nearest kept surface**,
    **Most used surface on the tile**, or a specific **Layer**), set per-surface priority
    (**Auto**, **Protect**, or **Remove first**) globally or per tile, and fix either **All tiles
    over the limit** or only the selected tile.
- **3D view distance for mission points and faster 2D/3D mission view.**
  - Added a **3D view distance** slider (`50 m – 15000 m`, default `800 m`, saved in settings) in
    **Mission > Show on the map** to control how far from the camera mission markers and event-group
    model previews are drawn in the 3D view.
  - Distant points beyond the chosen distance are skipped before sampling terrain or projecting
    rings, and the 2D view now culls off-screen markers and draws zone rings directly, keeping large
    missions with hundreds of points smooth.
- **Unified editor UI across windows and left-panel tool tabs.**
  - **Project settings**, **Settings**, and **Keyboard shortcuts**: redesigned with native sections,
    dark cards, segmented controls, and `PopupWindow` chrome (pop-out, minimize, maximize).
    **Shortcuts** is now also accessible as a tab inside **Settings**, with live search, a custom
    shortcut counter, a conflict banner, keycap buttons, per-row reset, and segmented brush
    modifiers.
  - **Mission editor & Mission panel**: switched **Mission editor** from two top rows of tabs to a
    grouped left navigation sidebar (like **Settings**), added an **Overview** dashboard with 8
    clickable metric cards and a file inventory table, and unified the **Mission** side panel with
    aligned map visibility toggles, colored tool icons, and a **Selected** inspector.
  - **Select, Sculpt, Paint, Place, Roads, and Shapes tabs**: unified status cards, segmented icon
    and glyph switches, `ValueField` sliders (including `Angle step`, `Scale`, `Copied height`,
    `Diameter`, and `Deck at m`), full-width dropdowns, responsive **Surface layers** and **Terrain
    stamps** tile grids (with search and bounded scrolling for **Your stamps**), and consistent
    icon buttons across **Surfaces in the game**, **Surfaces from the satellite**, **Surface under
    objects**, **Selected shape**, and **Piece by piece** roads.

### Fixed

- **Exported terrain layer masks and satellite tiles match Terrain Builder.**
  - Fixed exported `M_*_lca.paa` / `S_*_lco.paa` tiles to include proper PAA `CXAM` and `GALF`
    metadata and mipmaps, keep full satellite brightness, follow Terrain Builder's 5/6-surface slot
    order, and spread blue/alpha mask channels smoothly across surface edges so terrain transitions
    in the game match the editor.
  - Matched relief-based surface blending in the editor viewport to how the game reads `_nopx`
    height data.
- **Sculpt slider clipping and window auto-resize loops.**
  - Fixed right-edge clipping of brush slider values (`Radius`, `Strength`, `Falloff`, `Edge noise`)
    in the **Sculpt** tab caused by an oversized **Terrain stamps** grid.
  - Fixed vertical auto-growth in **Mission editor > Files** and horizontal auto-growth in
    **Settings**, plus consistent left text alignment on full-width menu buttons.

### Good to know

- Projects from 0.4.2 and earlier open in 0.4.3, and projects saved in 0.4.3 open in 0.4.2 as well.
  The new **3D view distance** setting is stored in your editor settings (`settings.json`) and
  defaults to `800 m`.

## 0.4.2

### New

- **Outliner layer & modifier management upgrades.**
  - **Drag-and-drop reordering with real stacking order**: drag terrain, mask, and satellite
    modifiers, object layers, routes, and generators up or down in the Outliner. Reordering
    modifiers immediately changes how they stack on top of each other (higher in the list draws on
    top) and supports Undo/Redo.
  - **Inline renaming**: double-click or right-click (**Rename**) any terrain, mask, or satellite
    modifier, generator, or route in the Outliner to give it a custom name. Renamed items are saved
    in the project and shared in Team Work.
  - **Convert mask & satellite modifiers to editable brush**: right-click a generated modifier
    (such as surface under objects, baked objects, or painted water) or click **Convert to brush**
    in its details panel to bake it into a hand-editable brush layer, with a clear confirmation
    warning that its generator settings will no longer be editable afterward.
  - **Cleaner Outliner rows**: quick eye visibility toggles, type badges, and consistent hover and
    selection styling across all Outliner sections.
- **3D `<dispatch>` proxy editor in Loot points (`mapgroupproto.xml`).**
  - Edit `<dispatch>` groups (`DE chance`) and `<proxy>` items (car wheels, doors, hoods, trunks,
    etc.) directly inside the 3D Loot points window, with live 3D GPU preview of each proxy model
    resolved from `CfgVehicles` (toggleable via **Dispatch** in the toolbar).
  - **3D Move & Rotate gizmos**: click a proxy in 3D or in the list to move it along **X / Y / Z**
    or switch to **Rotate** (`R` or toolbar button) to turn it with **Pitch / Yaw / Roll** rings
    (with **Shift** 5° angle snapping and live degree readout).
  - **Alt precision mode**: hold **Alt** while dragging any gizmo handle (move, rotate, range, or
    height) to slow movement 5× for fine placement, matching the main 3D map viewport.
- **Minimize, Maximize, and Pop-out buttons on popup windows.**
  - Floating editor windows (**Loot points**, **Mission editor**, **Map config**, **Project
    settings**, **Map check**, **Road catalog**, **Material**, **Surface**, **Team work**, **Merge**,
    and **Keyboard shortcuts**) now have **Pop-out** (detach into an independent OS window for
    multi-monitor setups), **Minimize** (collapse to title bar), and **Maximize / Restore** buttons
    right next to Close (`X`), plus double-click on the title bar to maximize or restore.

### Fixed

- **Team Work visual diff legend placement.** Moved the Team Work change-summary pill to the
  top-center of the 3D viewport so it no longer hides behind the bottom-left minimap.

### Good to know

- Projects from 0.4.1 and earlier open in 0.4.2, and projects saved in 0.4.2 open in 0.4.1 as well.
  Custom modifier names and converted brush layers are stored in the standard project fields and
  sync across Team Work.

## 0.4.1

### New

- **Realistic water painting in Satellite (`Water`).** Paint depth-graded ocean, river, and pond
  water directly into the satellite image as a modifier (with Undo, Outliner visibility, blending,
  export, and Team Work support):
  - **Ocean / sea**: paints below sea level with shallow-to-deep color transitions, subtle wind
    ripples, and optional **Shoreline foam**.
  - **Rivers and ponds**: rasterizes placed water models with real depth-to-bed shading and ripples.
  - **Terrain & above-water awareness**: clips underground water geometry (**Only what is above the
    ground**) and leaves piers, bridges, rocks, boats, and buildings standing above the water
    untouched (**Leave out objects above water**).
  - Choose between **Natural**, **Coastal**, and **Murky** styles plus a **Blending** slider.
- **2D player map export upgrades (`File > Export > 2D player map (PNG)...`).**
  - **Full map resolution**: choose **Full map** (native satellite width or 1 px/m) and presets up
    to `20480 px` (supported up to `32768 px`).
  - **Place name font and size**: pick any common Windows font, the built-in pixel font, or browse
    for a custom `.ttf` / `.otf` / `.ttc` font file (with full Unicode/Polish character support),
    and choose **Small**, **Medium**, or **Large** label size.
  - **Separate buildings and vegetation**: **Building footprints** now draws only buildings, while
    the new **Trees and bushes** option draws all placed trees and bushes as forest canopy on the
    map (so tree lines, groves, and forests on non-forest mask layers appear on topographic maps).
  - **With satellite modifiers**: in Satellite style, toggle whether baked satellite modifiers are
    included — turn off **Building footprints** to keep only what is baked into the satellite.
- **Unified UI in Export, Key points, and Satellite Bake objects.**
  - **Export files** window now uses the editor's native sections, aligned rows, value fields,
    segmented controls, and accent footer bar.
  - **Key points** tab now shows a count in the list header, full-height scrollable list with type
    labels and double-click camera jump, and a dedicated **Selected key point** section with
    `ValueField` controls, `Position` vector row, and **Go to** / **Delete** buttons.
  - **Bake objects** in Satellite is organized into collapsible **What to bake**, **Left out when
    baking the whole map** (with compact removable chips), and **Appearance** subsections, plus a
    primary **Bake** button.

### Fixed

- **Vector row alignment (`Position`, `Rotation`, etc.).** Fixed a 3 px left shift in `vector_row`
  so X/Y/Z coordinate fields line up cleanly with the value column of surrounding rows.
- **Tree and bush detection for bare model names.** Vegetation models stored by filename only (such
  as `t_betulapendula_2f.p3d` or `b_corylusavellana_2s.p3d`) are now consistently recognized as
  plants across the 2D map exporter and editor.

### Good to know

- Projects from 0.4.0 and earlier open in 0.4.1, and projects saved in 0.4.1 open in 0.4.0 as well;
  painted satellite water is stored as a standard satellite modifier.

## 0.4.0

### New

- **3D loot point editor for `mapgroupproto.xml`.** Right-click any building in the Content Browser
  and choose **Loot points...** to inspect and edit its loot containers and spawn points in a
  dedicated 3D GPU preview window (requires a mission folder):
  - **Fly camera** with the same controls as the main 3D view (`RMB + WASD/QE`, wheel to zoom,
    `MMB` to pan) plus quick camera presets (**Iso**, **Top**, **Front**, **Side**, **Inside**).
  - **Visual proxies** (furniture, tables, shelves) are shown inside the building and can be
    toggled on/off, along with a horizontal **Cut roof** slice slider to see inside rooms and floors.
  - **Interactive 3D gizmo** for the selected loot point: move along **X / Y / Z** axes, resize
    **range** by dragging the horizontal ring, and adjust **height** clearance by dragging the top
    **H** handle.
  - **Click 3D to add** places a new loot point directly onto the building floor, shelf, or table
    surface under the cursor.
  - Edit prototype class, max loot, dynamic event loot (`deloot`), usage/value flags, containers,
    categories, tags, and individual point settings, then save directly to `mapgroupproto.xml`.
- **Place on top of objects and Drop onto surface.**
  - **Place on top of objects** snaps newly placed objects onto roofs, floors, tables, and other
    objects instead of only the terrain. Works in **Place one** mode and when dragging assets from
    the Content Browser. Toggle it from the new **Surface** button next to **Ground** in the
    toolbar, the Place panel, or its shortcut (`Shift+End`).
  - **Drop onto surface** (`Edit > Drop onto surface`, default `End`) drops already selected
    objects onto the highest object surface or terrain underneath.
- **3D ghost preview when placing or dragging objects.** Hovering in **Place one** mode or dragging
  models, presets, or prefabs from the Content Browser onto the map now shows a live translucent 3D
  preview of the object (respecting surface snapping, rotation, and scale) instead of a flat
  circle.
- **Hide and isolate objects.** Temporarily hide selected objects (`Ctrl+H`), isolate the current
  selection (`/`), or show all hidden objects again (`Alt+H`).
- **DayZ Editor (`.dze`) and Object Spawner JSON (`cfggameplay.json`) import and export.** Import
  and export object layouts directly in DayZ Editor `.dze` format and vanilla Object Spawner JSON
  from `File > Import` and `File > Export`.
- **2D player map PNG export.** Export a clean top-down 2D map image with hillshade, elevation
  contours, roads, railways, buildings, DayZ coordinate grid, and key point labels.
- **DayZ slope thresholds and object density heatmap.**
  - The slope overlay highlights exact DayZ walkability and climb thresholds.
  - New **Object density** shading mode visualizes crowded areas on the map, paired with a dense
    area warning in **Map Check**.
- **Expanded `CfgWorlds` settings (`File > Map config...`).** Configure sea waves, atmosphere,
  sound map, and navmesh settings directly in the GUI, written to `config.cpp` on export.
- **Config-driven clutter (grass) draw distance.** In `3D View > Camera`, switch grass draw
  distance between the editor setting and your map's `Map config` (`CfgWorlds`) value.
- **Team Work visual diff overlay.** Preview incoming Team Work patches directly on the 2D and 3D
  map (added, moved, and removed objects highlighted in color) before applying them.
- **Manage recent projects.** Remove individual entries (`×`) or clear the whole recent projects
  list from `File > Recent projects` and from the welcome screen.

### Fixed

- **Content Browser deselection.** Clicking an already-selected item in the Content Browser again
  (or clearing the selection) deselects it and clears the active object in the Place tool, leaving a
  normal cursor with no ghost preview or brush circle.
- **Welcome screen recent projects** are centered neatly under the action buttons.
- **Loot point coordinates** match DayZ Central Economy's 90-degree building frame when mapping
  `mapgroupproto.xml` points to P3D model space.
- **Loot point editor window** keeps a stable height and resizes cleanly in both axes without
  drifting or clipping.

### Good to know

- Projects from 0.3.0 and earlier open in 0.4.0. A project saved in 0.4.0 also opens in 0.3.0; any
  new `CfgWorlds` settings default gracefully on older versions.
- All new `Map config` fields and `mapgroupproto.xml` edits are saved, diffed, and merged in Team
  Work.
- The **Drop onto surface** (`End`) shortcut drops currently selected objects; **Place on top of
  objects** (`Shift+End`) toggles snapping for newly placed and dragged objects.

## 0.3.0

### New

- **Maps of real places.** `File > New project from a real place` (or **Real place** in the New
  project window) makes a map of a real square of the Earth, of one of the DayZ map sizes. Find the
  place by name or by its latitude and longitude and choose it on a satellite picture. The editor
  downloads:
  - the **satellite picture**: Esri World Imagery or Sentinel-2 cloudless for the whole world,
    the open orthophotos of Poland (Geoportal), Czechia (ČÚZK), Austria (basemap.at) and the USA
    (USGS), or your own tile server. The window shows the licence of each source;
  - the **heights** (AWS Terrain Tiles);
  - the **OpenStreetMap shapes**: roads, railways, rivers and streams, lakes, forests (leafy and
    conifer apart), fields, meadows, scrub, built-up land, sand, rock and buildings, each kind in
    a shape layer of its own, with clearings as holes of the forests.

  Then it can paint the **surface mask** from the picture and from the shapes, place the game's
  **buildings** in the outlines of the real ones, build the **roads** from the game's road pieces,
  and add **forest, field, river and stream generators**, run one after another when the map
  opens. The map's latitude and longitude are set to the place. No account or API key is needed.
  See [docs/real-places.md](docs/real-places.md).
- **Surfaces from the satellite has two new ways**, and is now in the **Paint** tab:
  - **Natural colours** tells forest, grass, fields, soil, rock and roads apart in a real photo;
  - **Picked colours** is your own list of colours, each with its surface: four colours make a
    mask of four surfaces. **Find colours** finds them, a click on the preview adds one.
- **Decals can be baked into the satellite** (**Decals too** in Bake objects): road damage,
  manholes, crossings and the marks under wrecks, with their see-through parts.
- **Merge brush modifiers.** Right click a modifier of a brush in the outliner: **Merge down**, or
  **Merge all brush modifiers**. The image and the mask stay as they were. In team work only your
  own are merged.
- **Rows of walls and fences from a preset.** **Copies of** takes the copies from an object preset,
  each a model drawn by its weights, so a broken fence comes now and then.
- **Fields of models and presets look alike everywhere**, as in Unreal: a picture (larger under the
  pointer) that takes what is dragged from the Content Browser, a list with a search and pictures,
  and buttons to take the one selected in the Content Browser, show it there, or clear the field.
  Lists of surfaces show the texture of each.
- **Surface under objects: only what is above the ground.** With the outline of the model, a rock
  sunk half into the ground paints only round what shows of it.
- **Shapes can have holes**, which the mask and the generators leave out.
- **The guide has a page for each part** of the editor, see [docs/guide.md](docs/guide.md).

### Fixed

- **The editor's own icon** shows on the taskbar and in Alt+Tab, not the default one.
- **Gates and wickets** take only the gap they fill in a fence, not the whole swing of their leaf.
- **Surface under objects** follows objects that are tilted or turned over, instead of an outline
  mirrored and set aside.
- **A forest generator** leaves out shapes whose outline crosses itself instead of planting nothing.
- **Errors of the road tools** are shown in the language of the interface, not in Polish.
- **Changing the work drive** in the settings reads models, thumbnails, the list of models and road
  pieces from the new one at once, without a restart.
- **Rows in lists of presets** take the whole width of the list.

### Good to know

- Projects of 0.2.9.1 and earlier open in 0.3.0. A project saved in 0.3.0 opens in 0.2.9.1 too,
  without what is new: holes of shapes and the place a map of a real place was made from.
- Esri's satellite pictures may not be published in a map: use them to trace and for tests.
- Roads built from OpenStreetMap get no crossroad pieces where they cross: fix those with the
  Roads tab. A road that cannot be fitted stays a shape, and the Output Log says how many.
- Holes of shapes are not yet written to shape files for Terrain Builder, nor used by the
  satellite tools.

## 0.2.9.1

### New

- **Select the objects inside a shape.** Select one or more polygons in Shapes and click **Select
  objects inside** in Details (or in the right-click menu of the map). The objects inside are
  selected and the Select tool opens, ready to move or delete them. Hold Shift to add them to what
  is already selected.
- **Shift+Alt with a drag adds to the selection** with a box or a lasso. Shift alone still turns the
  selection while something is selected, so it could not add before.
- **Models that stand on the ground say so.** Walls, fences, rail tracks, sidewalks, timber piles
  and about 200 other vanilla models have `placement = slopelandcontact`: the game puts them on the
  ground and does not use their elevation, so they cannot be raised. Details now shows
  **Stands on the ground** for them. Where BI made a copy that can be raised (`_nolc`, for example
  `Sidewalk3_Border_nolc`), a button puts that copy in place of the selected objects, right where
  they were shown.

### Fixed

- **Ctrl with a box or a lasso takes objects out of the selection**, the same as Ctrl with a click.
  Before, it added them. The box turns red while it takes objects out.
- **Place: the scale range works with object presets.** A preset places its models with its own
  scale range, and the panel showed the range of the tool instead, which was not used. While a
  preset is chosen, **Rotation and size** now shows and changes the preset's settings. Without a
  preset, each placed object gets a random size in the range, as before.

### Good to know

- Projects open in this version and in 0.2.9 alike; the project format did not change.
- Swapping a model for its `_nolc` copy is one undo step (Ctrl+Z).

## 0.2.9

### New

- **A new look.** Darker panels on a darker ground, one blue for what is active, the Segoe UI
  font of Windows, and a thin yellow frame round the view.
- **Every panel is a tab.** Drag a tab by its title onto another panel to put them together, onto
  the edge of a panel to split it, or out of the window to let it float. Right click a tab to close
  or float it. **View > Window layout** opens closed panels again and has **Default layout**. Your
  layout is kept for the next start.
- **The editor draws its own title bar**, with the menus, the name of the project (a yellow dot
  while it has unsaved changes) and the buttons of the window. Drag it to move the window, double
  click it to fill the screen; the edges of the window resize it.
  `Edit > Settings > Display > Own title bar` brings back the usual Windows title bar.
- **A new toolbar:** the modes as icons over their names, the gizmo, the views as 3D / 2D / 3D | 2D,
  and **Build** (with its key) and **Play** at the right.
- **Snap to the grid.** A new switch in the toolbar: while it is on, moving along an arrow of the
  gizmo or over the ground goes by whole steps of the grid (1 m at first; the step can be changed
  next to the switch). Without it, Ctrl does the same while held, as before. The angle step and
  **Ground** (objects stick to the ground) sit next to it.
- **Tidier panels:**
  - Numbers are dark fields that fill with blue up to their value: drag across a field to change
    it, or click it to type a value.
  - Names sit in one column and values in the next; every other row is a shade lighter.
    Switches are in the column of values.
  - The tools of Sculpt, Shapes, Roads and Key points are tiles with their picture over their name.
  - The line telling what a tool does is one quiet line; the whole text shows under the pointer
    (or in full with *Show key help* on).
- **Outliner:** rows as wide as the panel with an icon in the colour of their kind (roads,
  generators, layers, models...), the number of objects at the right, and icons to show and edit
  layers. The team filter sits in the bar over it.
- **Details:** the model with its folder at the top, then **Transform** with Location, Rotation and
  Scale as X, Y and Z fields in the colours of the axes, then **Placement** (layer, ground).
- **Paint:** the brush shows the colour, name and number of the layer it paints, with the
  eyedropper beside it. Shape, Spray and where the strokes go are rows of their own.
- **Content Browser:** quieter blue-grey folders, names cut to the width of their tile, the tile
  size next to the search, and the bar over it all on one line.
- The status bar tells when the project was last saved.

### Fixed

- **The mask colour of a surface layer can be changed again.** The colour button in the menu of a
  layer closed the menu as soon as you clicked the picker. The picker is now in a submenu of its
  own, and the colour square at the top of the Paint panel opens it too.
- **A changed mask colour can be undone** with Ctrl+Z.
- A warning shows when two surface layers have the same colour, which the mask of the map cannot
  tell apart.
- "1 objects selected" reads "1 object selected", and values that round to zero no longer show a
  minus ("-0.0", "-0.000").
- A long message keeps to one line of the status bar; all of it shows under the pointer.
- The README and the guide show the new look, with a new picture of the forest generator.

### Good to know

- Projects are saved as before: projects of 0.2.9 open in 0.2.8 and the other way round.
- The window layout of older versions is read once: panels you had on another side start there.
  An older version still opens with its own layout, ignoring the tabs.
- In full screen (F11) the panels still slide in from the edges as before; tabs floated out of the
  window are not shown there.

## 0.2.8

### New

- **Team work is no longer experimental.** The **Team** menu is always there; you no longer need
  *Show experimental features*. Only the mission editor stays experimental.
- **Team work warns you about the work of others.** Deleting a modifier, road or generator that
  someone else made first asks whether to delete it anyway, and names who made it. Changing their
  things (moving their road, running their generator again, editing objects on a layer of theirs
  that you unlocked) shows a message once, with a reminder that Ctrl+Z undoes it.
- **Rows of walls and fences:**
  - **Row direction** turns the way the row goes, and its arrows, by any angle from the long side
    of the first object: a slider, a typed value, or 0°, 90°, 180° and -90°. For models whose
    copies belong another way, such as posts or gates.
  - **Turn copies by 180° at random** and **Random turn ±°** turn the new copies about their own
    middle, so a row shows both sides of a wall or looks less straight. The outlines while
    dragging show these turns.
- **Wrecks and road decals:**
  - **Turn of the wrecks** turns every wreck by a chosen angle, for models that face the wrong way.
    The random turn of cars in a jam and on the verge can be set too.
  - **Most metres from the middle** limits how far decals lie from the middle of the road, and
    **Keep decals inside the road** (on by default) keeps wide decals nearer the middle so their
    edges stay on the road.

### Fixed

- **Camera below the terrain** moved from the Show menu to the Camera menu of the 3D view. It is
  still in the View menu and on the B key.
- Messages of the forest generator and the updater, and errors when reading files (Terrain
  Builder exports, SHP, GeoJSON, models, textures), were partly in Polish in the English
  interface. They are in English now.
- The README, the guide and the screenshots show the editor as it is now.

### Good to know

- Projects saved in 0.2.8 still open in 0.2.7.2. The older version leaves out the new settings
  of the wrecks generator.
- When you run a wrecks generator again, its decals are laid out differently than before: they
  now keep inside the road.

## 0.2.7.2

### New

- **Rows of walls and fences.** A new section of the Select panel and a fourth gizmo, **Copy in a
  row** (T), lay copies of the selection end to end along its long side:
  - Drag one of the red arrows at the ends of the selection. Outlines show each copy that fits up to
    the cursor. Pull aside to turn the row, or hold Ctrl to turn it by the angle step. When you let
    go, the copies are laid and the last one is selected, so you can go on from there.
  - The green ring turns the selection about the point where it joins the wall next to it. The
    selection first moves back to meet that wall, so it stays joined.
  - **Join to the neighbour** (J) moves the selection so its end meets the end of the object next
    to it.
  - The arrow buttons, or Ctrl+Shift+D and Ctrl+Shift+A, lay one copy ahead or behind.
  - **Gap** sets the space between copies and between joined neighbours. Negative values overlap
    them.
- **Copy direction** (Select panel): click it, then click another object, and the selection turns
  the way that object faces. Useful to line up a broken wall with the wall next to it.
- **Reset scale** (Select panel) sets the selected objects back to their own size.
- **Fence corners.** With *Join the pieces at corners* the fence generator has two more options:
  - **Close the corners of thick walls**: the pieces reach past each corner, so the outer edge
    closes. Where a wall has a pillar at one end, the pillar stands centred on the corner and the
    other wall ends inside it. The panel shows how thick the pieces are at their ends, and you can
    set the reach at corners by hand.
  - **Corner object**: a post or corner piece (for example `Wall_IndVar1_Pole`) stands on every
    corner, and the pieces stop at its sides.
  - The start and end point of a closed line, such as the outline of a polygon, now counts as a
    corner too.

### Good to know

- Projects saved in 0.2.7.2 still open in 0.2.7.1. The older version builds fences without the
  new corner options.

## 0.2.7.1

### New

- **Share your library.** Right-click generators, presets or prefabs in the library and choose
  **Export to a file** to save them as a `.dzwelib` file. Do the same on a folder, on one of the
  groups, or on the Library itself to export everything in it. **Import from a file** adds the
  contents of such a file as new items, next to what you already have, so nothing is overwritten.
  A generator takes the object presets it uses along with it. Models are not in the file, only
  their paths, so the other person needs the same models on their work drive.

### Fixed

- Fence generator: the model fields no longer push their buttons out of view in a narrow panel.
  The model box now sits under its label, across the full width, and still takes dragged models.
  The label line holds only **Use selected** and the clear button. The gate and wicket header
  shrinks to keep its buttons visible.

## 0.2.7

### New

- **Terrain stamps.** Lay craters, hills, ridges, quarries, embankments, valleys, volcanoes and dunes
  on the map from the Sculpt tab or the Content Browser. You can also make your own stamps from grey
  pictures, or keep a piece of your map as a stamp. Stamps shape the ground that roads, rivers, ponds
  and pads are laid on, and graded roads over a stamp follow it again when the stamp changes.
- Terrain modifiers can now be selected on the map with the Select tool.
- **Keep the terrain of a deleted modifier.** When you delete a terrain modifier, a road's levelling or a
  generator's terrain change, you can keep the shape it gave the ground. It is written into the sculpted
  terrain that is exported as ASC. One step to undo.
- **Set height brush** in the Sculpt tab. The sculpt, shape, road and place tools now have drawn icons.
- **Go to** a key point, a position or a map grid square. You can also move the camera to things
  from the outliner.
- **Gizmo placement.** Choose where the gizmo stands and what a selection turns about. The Select
  panel has icon buttons.
- **Placing brush** paints a whole zone with a density, and can skip spots that other objects already use.
- **Fences:** wickets and gate leaves turn up to 180°, and the fence closes round them. End objects can
  stand inside the line.
- **Satellite bake:** leave objects out by layer, model or path.
- The interface starts in your system language when it is available, otherwise in English.

### Fixed

- Map tiles always use the TerrainX shader, so the game no longer swaps in gravel for them.
- Mask edges keep their two surfaces when the mask is compressed.
- Roads, tracks, plants and model paths now follow the work drive you chose, not only `P:`.
- The wrecks generator no longer puts tyre marks among its road decals.
- The view can show surfaces the way the game does, and it can show hard mask pixels.

### Good to know

- **Projects stay compatible.** Projects from older versions open in 0.2.7.
- Projects saved with terrain stamps also open in 0.2.6 and older. Those versions simply leave the
  stamps out instead of refusing the project.

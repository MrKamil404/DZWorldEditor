# Changelog

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

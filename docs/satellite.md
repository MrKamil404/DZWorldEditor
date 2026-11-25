# Satellite image

![The satellite tools](screenshots/satellite.jpg)

* **Bake objects** draws the objects as seen from above into the satellite image, organized into
  **What to bake** (selected objects or all, trees and bushes, road parts, water parts, and decals),
  **Left out when baking the whole map** (quick toggles and removable path chips such as `\wrecks\`),
  and **Appearance** (**Only what is above the ground** and **Blending**). **Decals too** paints road
  damage, manholes, crossings and the marks under wrecks with the see-through parts of their
  pictures, over the roads baked with them; only what stands clearly above a decal (a car, a roof, a
  tree) hides it. Baking the whole map can leave things out, so wrecks, rubbish or fences do
  not end up in the image: turn off **Bake into the satellite** in the menu of an object layer (the
  layers of a built map are sorted by kind), or right click a model in the Outliner or the Content
  Browser and choose **Leave out of the satellite bake**. Objects you select by hand are always baked.
* **Water** paints realistic ocean, river and pond water into the satellite image as a modifier:
  **Ocean / sea** shades water below sea level by depth with subtle ripples and optional
  **Shoreline foam**; **Rivers and ponds** paints placed water models by depth to the terrain bed.
  **Only what is above the ground** clips water meshes buried under hills, and **Leave out objects
  above water** keeps piers, bridges, rocks, boats and buildings standing above the water untouched.
  Choose **Natural**, **Coastal** or **Murky** style and **Blending**.
* **Clone brush** copies from another place of the satellite image or from a picture you load.
  Choose the source in the preview, then paint on the map.

* **Colour correction** changes the colours of the whole image or of shapes, and can be changed
  later. **Use legacy** makes brightness and contrast work as Photoshop's legacy mode.
* **Satellite from the mask** paints the image in the colours of the surfaces of the mask, with the
  pattern of their textures, ragged soft edges between them, lighter and darker patches, the
  shading of the hills and water under the sea level. Each surface gets its colour in **Colours of
  the surfaces**: guessed from its name, or taken from the satellite image the map already has. A
  project without a satellite image gets one. It works on the whole map or on shapes.

Every bake, clone stroke, correction and generated image is a **satellite modifier** in the
outliner: it can be hidden or deleted, and what it covered comes back. Everything the satellite and
mask modifiers make is part of the map when it is built.

Brushes can leave many modifiers behind. Right click a modifier of a brush (clone, adjust or colour
brush on the satellite, brush strokes on the mask) in the outliner and choose **Merge down** to
merge it into the one right under it, if that one is of the same brush and both are shown, or
**Merge all brush modifiers** to merge each run of neighbouring modifiers of one brush into one.
The image and the mask stay as they were; the merged modifier keeps the name of the lowest one, and
undo splits them again. In team work only your own modifiers are merged: merging takes the upper
one away, and a modifier of someone else would then go from their copy too.

[Back to the guide](guide.md)

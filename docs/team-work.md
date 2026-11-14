# Team work (experimental)

Several people can work on one map at the same time, each on their own copy, and merge their
work at the end of the day. No server is needed: the changes travel as files through a shared
folder that Dropbox, Google Drive, OneDrive or Syncthing keeps the same on every computer (or a
pen drive). Turn it on with *Edit > Settings > General > Show experimental features*; it is in
the **Team** menu.

## Setting up

1. One person opens the project, *Team > Team work...*, enters a name, chooses the team folder
   and clicks **Start team work**. The current state of the map becomes the shared version.
2. **Send a full copy** writes the project to `<team folder>\full\`.
3. Everyone else clicks **Take a full copy...**, picks that folder and a place on their own
   disk. The team folder the copy came from becomes the team folder of the new copy. Keep the
   project itself outside the synced folder: only the changes go through it.

## A day of work

* Everyone works on their own copy as usual; nothing is locked.
* In the evening one person clicks **Send my changes**. A small `.dzpatch` file with what changed
  since the shared version goes to the team folder.
* The others see *Team (1)* in the menu and click **Merge**. What only the other person changed
  is taken over, what only you changed stays. Where both changed the same thing, the merge
  window lists it with a button that shows the place on the map, and you choose *Keep mine*,
  *Take theirs* or, for the terrain, *Add both* (the two height changes added together).
* The one who merged sends their changes back; the first person merges them. Both now have the
  same map and count tomorrow's changes from it.

Two people may also send at the same time: each then merges the other's file, and the next
changes they send settle any difference.

## Who works on what

Everyone can keep their own layers of objects. A layer made during team work belongs to the one
who made it; *Owner* in the right-click menu of a layer gives it to anyone in the team, to no
one, or to someone new whose name you type. The layers of others show their owner and a lock:
their objects cannot be picked, moved or added to until you click the lock, which unlocks the
layer for you until you lock it again or close the editor. Layers of no one, like *Default*, stay
open to everyone.

Terrain, mask and satellite modifiers, roads and generators remember who made them; *Author* in
their right-click menu in the outliner gives them to someone else. The outliner shows the name
next to them, and the choice above it shows only the work of one person: layers by their owner,
the rest by who made it.

To split a map, for example one just imported from Terrain Builder, select an area of it and use
*Team > Give the selected objects*: the objects move to a layer of the one you choose, made when
they have none. Every change of owner or author can be undone and travels with the next patch,
also when nothing else of the thing changed.

When both changed something that belongs to one of you, the merge suggests that person's
version: Ania's tree on Ania's layer keeps her position unless you choose otherwise. Without an
owner your own version is suggested, as before.

## What is merged

| Part | How |
|---|---|
| Terrain (as sculpted) | Cell by cell, as changes in height; changes of both in one cell are added unless chosen otherwise |
| Surface mask (as painted) | Pixel by pixel |
| Satellite image (under its modifiers) | Pixel by pixel, a band at a time, so a large image is never held twice in memory |
| Mask and satellite modifiers | By id, tile by tile (128 px): both can work in one modifier, as with strokes of the clone brush, and only a tile both changed is asked. Their name, being shown and opacity merge apart from the pixels. Laid again over the merged pixels, so hiding one still shows what is under it; a patch carries only the tiles that changed |
| Objects | By a lasting identity, field by field: a move by one and a turn or new layer by the other both stay |
| Object layers | By name, with their owners |
| Terrain modifiers, roads, generators | By id, whole |
| Mission | File by file; text files (XML, JSON, scripts) line by line, as `diff3` does, and lines changed in place one by one. Other files whole. The server's persistence is left out |
| Map settings | Setting by setting |
| Surface layers, shapes, key points, holes, road catalog | Whole: taken over when only the other person changed them |

A new satellite image of another size, a new terrain size or a new class name of the map cannot
travel in a patch: the patch says so, and a full copy is sent instead.

## Files

* `.team\state.json` in the project: the team folder, the shared version and the patches merged.
* `.team\versions\`: the last shared versions (four besides the base). They share the satellite
  image with the project through hard links while it does not change; each version with a
  changed image takes as much space as the image.
* `.team\before-merge\`: a copy of the project from before each merge (the newest three). To go
  back, open the `project.dzwe` in it and use *Save as*.

Undo starts anew after a merge, as after opening a project. While the merge writes the project,
editing waits. Objects keep a lasting identity in `objects.dzob` (format 2); older projects get
one when they are opened, and older versions of the editor cannot read format 2. New modifiers get
random ids, so that two people never make the same one.

//! Mask and satellite modifiers merged tile by tile: two people can both work in one modifier,
//! as with strokes of the clone brush, and only a tile both changed is a conflict. The settings
//! of a modifier (name, shown, opacity) are merged apart from its pixels.
use super::{choice, Choice, ConflictKey, ConflictKind, Merged};
use crate::maskmod::MaskModifier;
use crate::satmod::SatModifier;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

pub type Key = (u16, u16);
pub type Tile = Arc<Vec<u8>>;

/// A modifier of tiles laid over the mask or the satellite image.
pub trait LayerMod: Clone + PartialEq {
    fn id(&self) -> u32;
    fn name(&self) -> &str;
    fn tiles(&self) -> &HashMap<Key, Tile>;
    fn tiles_mut(&mut self) -> &mut HashMap<Key, Tile>;
    fn same_settings(&self, o: &Self) -> bool;
    fn take_settings(&mut self, from: &Self);
}

impl LayerMod for SatModifier {
    fn id(&self) -> u32 {
        self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn tiles(&self) -> &HashMap<Key, Tile> {
        &self.tiles
    }
    fn tiles_mut(&mut self) -> &mut HashMap<Key, Tile> {
        &mut self.tiles
    }
    fn same_settings(&self, o: &Self) -> bool {
        (&self.name, self.kind, self.enabled, self.opacity) == (&o.name, o.kind, o.enabled, o.opacity)
    }
    fn take_settings(&mut self, from: &Self) {
        (self.name, self.kind, self.enabled, self.opacity) = (from.name.clone(), from.kind, from.enabled, from.opacity);
    }
}

impl LayerMod for MaskModifier {
    fn id(&self) -> u32 {
        self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn tiles(&self) -> &HashMap<Key, Tile> {
        &self.tiles
    }
    fn tiles_mut(&mut self) -> &mut HashMap<Key, Tile> {
        &mut self.tiles
    }
    fn same_settings(&self, o: &Self) -> bool {
        (&self.name, self.kind, self.enabled) == (&o.name, o.kind, o.enabled)
    }
    fn take_settings(&mut self, from: &Self) {
        (self.name, self.kind, self.enabled) = (from.name.clone(), from.kind, from.enabled);
    }
}

/// What a patch says about one changed modifier, beside the modifier itself, which carries its
/// settings and only the tiles that changed.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ModDelta {
    pub id: u32,
    /// Not in the shared version: the modifier comes whole.
    pub new: bool,
    /// Tiles the modifier no longer has.
    pub removed_tiles: Vec<[u16; 2]>,
    /// Who made it, when known.
    pub author: String,
}

/// The changed and new modifiers with only their changed tiles, what else changed in them, and
/// the ids of the removed ones.
pub fn diff<M: LayerMod>(base: &[M], now: &[M], author: impl Fn(u32) -> String) -> (Vec<M>, Vec<ModDelta>, Vec<u64>) {
    let before: HashMap<u32, &M> = base.iter().map(|m| (m.id(), m)).collect();
    let (mut set, mut deltas) = (Vec::new(), Vec::new());
    for m in now {
        let Some(b) = before.get(&m.id()) else {
            set.push(m.clone());
            deltas.push(ModDelta { id: m.id(), new: true, removed_tiles: Vec::new(), author: author(m.id()) });
            continue;
        };
        if *b == m {
            continue;
        }
        let mut part = m.clone();
        part.tiles_mut().retain(|k, t| b.tiles().get(k) != Some(t));
        let mut removed: Vec<[u16; 2]> = b.tiles().keys().filter(|k| !m.tiles().contains_key(k)).map(|k| [k.0, k.1]).collect();
        removed.sort();
        set.push(part);
        deltas.push(ModDelta { id: m.id(), new: false, removed_tiles: removed, author: author(m.id()) });
    }
    let ids: BTreeSet<u32> = now.iter().map(|m| m.id()).collect();
    let mut removed: Vec<u64> = base.iter().filter(|m| !ids.contains(&m.id())).map(|m| m.id() as u64).collect();
    removed.sort();
    (set, deltas, removed)
}

/// Applies the changed modifiers of a patch to `mine`. `default` gives what stays in a conflict
/// about a modifier when nothing is chosen, `place` where a tile is on the map. Gives the modifiers
/// taken from the other person, with their authors.
#[allow(clippy::too_many_arguments)]
pub fn merge<M: LayerMod>(
    list: &'static str, base: &[M], mine: &mut Vec<M>, set: Vec<M>, deltas: &[ModDelta], removed: &[u64], default: impl Fn(u32, &ModDelta) -> Choice,
    place: impl Fn(Key) -> Option<[f32; 2]>, choices: &HashMap<ConflictKey, Choice>, out: &mut Merged,
) -> Vec<(u32, String)> {
    let before: HashMap<u32, &M> = base.iter().map(|m| (m.id(), m)).collect();
    let mut taken = Vec::new();
    for (theirs, d) in set.into_iter().zip(deltas) {
        let id = theirs.id();
        let key = ConflictKey::Element(list, id as u64);
        let def = default(id, d);
        let pos = mine.iter().position(|m| m.id() == id);
        // The other person's whole modifier: what they sent laid over the shared version.
        let whole = |b: Option<&&M>| -> M {
            let mut w = match b {
                Some(b) if !d.new => (*b).clone(),
                _ => return theirs.clone(),
            };
            w.take_settings(&theirs);
            for k in &d.removed_tiles {
                w.tiles_mut().remove(&(k[0], k[1]));
            }
            for (k, t) in theirs.tiles() {
                w.tiles_mut().insert(*k, t.clone());
            }
            w
        };
        match (before.get(&id), pos) {
            (b, None) if d.new || b.is_none() => {
                mine.push(whole(b));
                taken.push((id, d.author.clone()));
            }
            (b, Some(p)) if d.new || b.is_none() => {
                // Made by both under one id: only by chance.
                let t = whole(b);
                if mine[p] != t {
                    out.conflict(key, ConflictKind::Changed, theirs.name().to_string(), None, def);
                    if choice(choices, key, def) == Choice::Theirs {
                        mine[p] = t;
                        taken.push((id, d.author.clone()));
                    }
                }
            }
            (b, None) => {
                out.conflict(key, ConflictKind::DeletedHere, theirs.name().to_string(), None, def);
                if choice(choices, key, def) == Choice::Theirs {
                    mine.push(whole(b));
                    taken.push((id, d.author.clone()));
                }
            }
            // Taken by the guards above.
            (None, Some(_)) => {}
            (Some(b), Some(p)) => {
                let m = &mut mine[p];
                // The settings, apart from the pixels.
                if !theirs.same_settings(b) && !m.same_settings(&theirs) {
                    if m.same_settings(b) {
                        m.take_settings(&theirs);
                    } else {
                        out.conflict(key, ConflictKind::Changed, theirs.name().to_string(), None, def);
                        if choice(choices, key, def) == Choice::Theirs {
                            m.take_settings(&theirs);
                        }
                    }
                }
                // The tiles one by one.
                let keys: BTreeSet<Key> = theirs.tiles().keys().copied().chain(d.removed_tiles.iter().map(|k| (k[0], k[1]))).collect();
                for k in keys {
                    let (bt, mt, tt) = (b.tiles().get(&k).cloned(), m.tiles().get(&k).cloned(), theirs.tiles().get(&k).cloned());
                    if mt == tt {
                        continue;
                    }
                    let take = if mt == bt {
                        true
                    } else {
                        let tkey = ConflictKey::ModTile(list, id as u64, k.0, k.1);
                        out.conflict(tkey, ConflictKind::Changed, m.name().to_string(), place(k), def);
                        choice(choices, tkey, def) == Choice::Theirs
                    };
                    if take {
                        match tt {
                            Some(t) => m.tiles_mut().insert(k, t),
                            None => m.tiles_mut().remove(&k),
                        };
                    }
                }
            }
        }
    }
    for id in removed {
        let id = *id as u32;
        let Some(p) = mine.iter().position(|m| m.id() == id) else { continue };
        let key = ConflictKey::Element(list, id as u64);
        if before.get(&id).is_some_and(|b| **b == mine[p]) {
            mine.remove(p);
            continue;
        }
        let def = default(id, &ModDelta { id, ..Default::default() });
        out.conflict(key, ConflictKind::DeletedThere, mine[p].name().to_string(), None, def);
        if choice(choices, key, def) == Choice::Theirs {
            mine.remove(p);
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::satmod::SatKind;

    fn tile(v: u8) -> Tile {
        Arc::new(vec![v; 4])
    }
    fn modifier(tiles: &[(Key, u8)]) -> SatModifier {
        SatModifier { id: 7, name: "clone".into(), kind: SatKind::Clone, enabled: true, opacity: 255, tiles: tiles.iter().map(|(k, v)| (*k, tile(*v))).collect() }
    }
    fn run(base: &[SatModifier], theirs: &[SatModifier], mine: &mut Vec<SatModifier>, choices: &HashMap<ConflictKey, Choice>) -> Merged {
        let (set, deltas, removed) = diff(base, theirs, |_| "Ania".into());
        let mut out = Merged::default();
        merge("sat_modifiers", base, mine, set, &deltas, &removed, |_, _| Choice::Mine, |_| None, choices, &mut out);
        out
    }

    #[test]
    fn strokes_of_both_in_one_modifier_stay() {
        let base = vec![modifier(&[((0, 0), 1)])];
        let theirs = vec![modifier(&[((0, 0), 1), ((5, 5), 2)])];
        let mut mine = vec![modifier(&[((0, 0), 1), ((9, 9), 3)])];
        let r = run(&base, &theirs, &mut mine, &HashMap::new());
        assert!(r.conflicts.is_empty());
        assert_eq!(mine[0].tiles.len(), 3);
        // Only the changed tile travels.
        let (set, _, _) = diff(&base, &theirs, |_| String::new());
        assert_eq!(set[0].tiles.len(), 1);
    }

    #[test]
    fn a_tile_both_changed_is_asked_and_settings_merge_apart() {
        let base = vec![modifier(&[((0, 0), 1), ((1, 0), 1)])];
        let mut theirs = vec![modifier(&[((0, 0), 2), ((1, 0), 1)])];
        theirs[0].enabled = false;
        let mut mine_start = vec![modifier(&[((0, 0), 3), ((1, 0), 4)])];
        mine_start[0].opacity = 100;
        let mut mine = mine_start.clone();
        let r = run(&base, &theirs, &mut mine, &HashMap::new());
        assert_eq!(r.conflicts.len(), 2, "{:?}", r.conflicts);
        assert_eq!(mine[0].tiles[&(0, 0)], tile(3));
        assert_eq!(mine[0].tiles[&(1, 0)], tile(4));
        // Their hiding and my opacity: both settings changed, so it is asked as one.
        let key = ConflictKey::ModTile("sat_modifiers", 7, 0, 0);
        assert!(r.conflicts.iter().any(|c| c.key == key));
        let mut mine = mine_start.clone();
        run(&base, &theirs, &mut mine, &HashMap::from([(key, Choice::Theirs)]));
        assert_eq!(mine[0].tiles[&(0, 0)], tile(2));
    }

    #[test]
    fn hiding_by_one_and_painting_by_the_other_both_stay() {
        let base = vec![modifier(&[((0, 0), 1)])];
        let mut theirs = base.clone();
        theirs[0].enabled = false;
        let mut mine = vec![modifier(&[((0, 0), 1), ((3, 3), 5)])];
        let r = run(&base, &theirs, &mut mine, &HashMap::new());
        assert!(r.conflicts.is_empty());
        assert!(!mine[0].enabled && mine[0].tiles.len() == 2);
    }

    #[test]
    fn a_tile_wiped_by_the_other_goes() {
        let base = vec![modifier(&[((0, 0), 1), ((1, 1), 1)])];
        let theirs = vec![modifier(&[((0, 0), 1)])];
        let mut mine = base.clone();
        run(&base, &theirs, &mut mine, &HashMap::new());
        assert_eq!(mine[0].tiles.len(), 1);
    }
}

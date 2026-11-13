//! Keyboard shortcuts in one place: every action the keyboard reaches, its default key, the
//! keys the user chose instead and the window that changes them. The modifiers held while
//! using a brush are kept here too, so that every brush reads them the same way.
use crate::i18n::t;
use crate::theme;
use eframe::egui::{self, Key, Modifiers};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A key with the modifiers held with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shortcut {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Shortcut {
    pub const fn new(key: Key) -> Self {
        Self { key, ctrl: false, shift: false, alt: false }
    }
    pub const fn ctrl(key: Key) -> Self {
        Self { key, ctrl: true, shift: false, alt: false }
    }
    pub const fn ctrl_shift(key: Key) -> Self {
        Self { key, ctrl: true, shift: true, alt: false }
    }
    pub const fn shift(key: Key) -> Self {
        Self { key, ctrl: false, shift: true, alt: false }
    }
    /// As it is shown and stored, such as `Ctrl+Shift+S`.
    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.ctrl { s.push_str("Ctrl+"); }
        if self.shift { s.push_str("Shift+"); }
        if self.alt { s.push_str("Alt+"); }
        s.push_str(key_name(self.key));
        s
    }
    pub fn parse(text: &str) -> Option<Self> {
        let mut s = Shortcut::new(Key::A);
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" => s.ctrl = true,
                "shift" => s.shift = true,
                "alt" => s.alt = true,
                _ => key = Key::ALL.iter().copied().find(|k| key_name(*k).eq_ignore_ascii_case(part) || k.name().eq_ignore_ascii_case(part)),
            }
        }
        s.key = key?;
        Some(s)
    }
    pub fn matches(&self, key: Key, m: Modifiers) -> bool {
        key == self.key && (m.ctrl || m.command) == self.ctrl && m.shift == self.shift && m.alt == self.alt
    }
}

/// A short name of a key; egui's own for the rest.
fn key_name(k: Key) -> &'static str {
    match k {
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Comma => ",",
        Key::Period => ".",
        Key::Minus => "-",
        Key::Plus => "+",
        Key::Equals => "=",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::Semicolon => ";",
        Key::Delete => "Del",
        Key::Escape => "Esc",
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        k => k.name(),
    }
}

/// Everything the keyboard can do.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub enum Action {
    NewProject,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    Settings,
    Shortcuts,
    Build,
    Play,
    SelectAll,
    Deselect,
    Duplicate,
    /// A copy of the selection under the cursor.
    PasteHere,
    /// A copy of what was selected last, under the cursor.
    PasteLast,
    Delete,
    Focus,
    Underground,
    ModeSelect,
    ModeSculpt,
    ModePaint,
    ModePlace,
    ModeRoads,
    ModeShapes,
    ModeKeyPoints,
    ModeSatellite,
    ModeMission,
    GizmoMove,
    GizmoRotate,
    GizmoScale,
    GizmoSpace,
    View3d,
    View2d,
    ViewSplit,
    CameraBelow,
    ToggleGrass,
    ToggleObjects,
    ToggleShapes,
    BrushSmaller,
    BrushBigger,
    BrushWeaker,
    BrushStronger,
    SearchAssets,
    CameraForward,
    CameraBack,
    CameraLeft,
    CameraRight,
    CameraUp,
    CameraDown,
}

impl Action {
    pub const ALL: [Action; 49] = [
        Action::NewProject, Action::Open, Action::Save, Action::SaveAs, Action::Undo, Action::Redo, Action::Settings, Action::Shortcuts, Action::Build, Action::Play,
        Action::SelectAll, Action::Deselect, Action::Duplicate, Action::PasteHere, Action::PasteLast, Action::Delete, Action::Focus, Action::Underground,
        Action::ModeSelect, Action::ModeSculpt, Action::ModePaint, Action::ModePlace, Action::ModeRoads, Action::ModeShapes, Action::ModeKeyPoints, Action::ModeSatellite, Action::ModeMission,
        Action::GizmoMove, Action::GizmoRotate, Action::GizmoScale, Action::GizmoSpace,
        Action::View3d, Action::View2d, Action::ViewSplit, Action::CameraBelow, Action::ToggleGrass, Action::ToggleObjects, Action::ToggleShapes,
        Action::BrushSmaller, Action::BrushBigger, Action::BrushWeaker, Action::BrushStronger, Action::SearchAssets,
        Action::CameraForward, Action::CameraBack, Action::CameraLeft, Action::CameraRight, Action::CameraUp, Action::CameraDown,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Action::NewProject => "New project",
            Action::Open => "Open project",
            Action::Save => "Save",
            Action::SaveAs => "Save as",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::Settings => "Settings",
            Action::Shortcuts => "Keyboard shortcuts",
            Action::Build => "Build PBO",
            Action::Play => "Build and play",
            Action::SelectAll => "Select all",
            Action::Deselect => "Clear the selection",
            Action::Duplicate => "Duplicate",
            Action::PasteHere => "Copy the selection to the cursor",
            Action::PasteLast => "Copy the last selection to the cursor",
            Action::Delete => "Delete",
            Action::Focus => "Frame the selection",
            Action::Underground => "Underground flag",
            Action::ModeSelect => "Select",
            Action::ModeSculpt => "Sculpt",
            Action::ModePaint => "Paint",
            Action::ModePlace => "Place",
            Action::ModeRoads => "Roads",
            Action::ModeShapes => "Shapes",
            Action::ModeKeyPoints => "Key points",
            Action::ModeSatellite => "Satellite",
            Action::ModeMission => "Mission",
            Action::GizmoMove => "Move",
            Action::GizmoRotate => "Rotate",
            Action::GizmoScale => "Scale",
            Action::GizmoSpace => "World or local axes",
            Action::View3d => "3D view",
            Action::View2d => "2D view",
            Action::ViewSplit => "3D and 2D side by side",
            Action::CameraBelow => "Camera below the terrain",
            Action::ToggleGrass => "Show grass",
            Action::ToggleObjects => "Show objects",
            Action::ToggleShapes => "Show shapes and road lines",
            Action::BrushSmaller => "Smaller brush",
            Action::BrushBigger => "Bigger brush",
            Action::BrushWeaker => "Weaker brush",
            Action::BrushStronger => "Stronger brush",
            Action::SearchAssets => "Search the Content Browser",
            Action::CameraForward => "Fly forward",
            Action::CameraBack => "Fly back",
            Action::CameraLeft => "Fly left",
            Action::CameraRight => "Fly right",
            Action::CameraUp => "Fly up",
            Action::CameraDown => "Fly down",
        }
    }
    pub fn group(self) -> &'static str {
        use Action::*;
        match self {
            NewProject | Open | Save | SaveAs | Undo | Redo | Settings | Shortcuts | Build | Play => "General",
            SelectAll | Deselect | Duplicate | PasteHere | PasteLast | Delete | Focus | Underground => "Selection",
            ModeSelect | ModeSculpt | ModePaint | ModePlace | ModeRoads | ModeShapes | ModeKeyPoints | ModeSatellite | ModeMission | GizmoMove | GizmoRotate | GizmoScale | GizmoSpace => "Tools",
            View3d | View2d | ViewSplit | CameraBelow | ToggleGrass | ToggleObjects | ToggleShapes | SearchAssets => "View",
            BrushSmaller | BrushBigger | BrushWeaker | BrushStronger => "Brushes",
            CameraForward | CameraBack | CameraLeft | CameraRight | CameraUp | CameraDown => "Camera (with the right mouse button held)",
        }
    }
    pub fn default_key(self) -> Option<Shortcut> {
        use Action::*;
        Some(match self {
            NewProject => Shortcut::ctrl(Key::N),
            Open => Shortcut::ctrl(Key::O),
            Save => Shortcut::ctrl(Key::S),
            SaveAs => Shortcut::ctrl_shift(Key::S),
            Undo => Shortcut::ctrl(Key::Z),
            Redo => Shortcut::ctrl(Key::Y),
            Settings => Shortcut::ctrl(Key::Comma),
            Shortcuts => Shortcut::new(Key::F1),
            Build => Shortcut::new(Key::F7),
            Play => Shortcut::new(Key::F5),
            SelectAll => Shortcut::ctrl(Key::A),
            Deselect => Shortcut::new(Key::Escape),
            Duplicate => Shortcut::ctrl(Key::D),
            PasteHere => Shortcut::new(Key::C),
            PasteLast => Shortcut::new(Key::V),
            Delete => Shortcut::new(Key::Delete),
            Focus => Shortcut::new(Key::F),
            Underground => Shortcut::new(Key::U),
            ModeSelect => Shortcut::new(Key::Num1),
            ModeSculpt => Shortcut::new(Key::Num2),
            ModePaint => Shortcut::new(Key::Num3),
            ModePlace => Shortcut::new(Key::Num4),
            ModeRoads => Shortcut::new(Key::Num5),
            ModeShapes => Shortcut::new(Key::Num6),
            ModeKeyPoints => Shortcut::new(Key::Num7),
            ModeSatellite => Shortcut::new(Key::Num8),
            ModeMission => Shortcut::new(Key::Num9),
            GizmoMove => Shortcut::new(Key::W),
            GizmoRotate => Shortcut::new(Key::E),
            GizmoScale => Shortcut::new(Key::R),
            GizmoSpace => Shortcut::new(Key::L),
            View3d => Shortcut::ctrl(Key::Num1),
            View2d => Shortcut::ctrl(Key::Num2),
            ViewSplit => Shortcut::ctrl(Key::Num3),
            CameraBelow => Shortcut::new(Key::B),
            ToggleGrass => Shortcut::new(Key::G),
            ToggleObjects => Shortcut::new(Key::H),
            ToggleShapes => Shortcut::shift(Key::H),
            BrushSmaller => Shortcut::new(Key::OpenBracket),
            BrushBigger => Shortcut::new(Key::CloseBracket),
            BrushWeaker => Shortcut::shift(Key::OpenBracket),
            BrushStronger => Shortcut::shift(Key::CloseBracket),
            SearchAssets => Shortcut::ctrl(Key::F),
            CameraForward => Shortcut::new(Key::W),
            CameraBack => Shortcut::new(Key::S),
            CameraLeft => Shortcut::new(Key::A),
            CameraRight => Shortcut::new(Key::D),
            CameraUp => Shortcut::new(Key::E),
            CameraDown => Shortcut::new(Key::Q),
        })
    }
    /// Fly keys are held while the right mouse button is, so they may share keys with the rest.
    fn while_flying(self) -> bool {
        self.group().starts_with("Camera")
    }
    /// Pressed again and again while the key is held.
    fn repeats(self) -> bool {
        matches!(self, Action::BrushSmaller | Action::BrushBigger | Action::BrushWeaker | Action::BrushStronger | Action::Undo | Action::Redo)
    }
}

/// A modifier key held while using a brush.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ModKey {
    Ctrl,
    Shift,
    Alt,
}
impl ModKey {
    pub const ALL: [ModKey; 3] = [ModKey::Ctrl, ModKey::Shift, ModKey::Alt];
    pub fn name(self) -> &'static str {
        match self {
            ModKey::Ctrl => "Ctrl",
            ModKey::Shift => "Shift",
            ModKey::Alt => "Alt",
        }
    }
    pub fn held(self, m: Modifiers) -> bool {
        match self {
            ModKey::Ctrl => m.ctrl || m.command,
            ModKey::Shift => m.shift,
            ModKey::Alt => m.alt,
        }
    }
}

/// What a modifier does while a brush is in use. Every brush reads these the same way.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub enum Hold {
    /// Lowers instead of raising, erases holes or placed objects.
    Invert,
    /// Smooths the terrain instead of the chosen sculpt tool.
    Smooth,
    /// Picks the surface under the cursor in the paint tool.
    Pick,
    /// With the mouse wheel: the size of the brush.
    WheelSize,
    /// With the mouse wheel: the strength of the brush.
    WheelStrength,
    /// With the mouse wheel: the noise at the edge of the brush.
    WheelNoise,
    /// Flies faster.
    Fast,
    /// Lines and turns go by whole steps of the angle set in the settings.
    AngleSnap,
    /// Moving, turning and scaling follow the mouse five times slower, for fine work.
    Precise,
}
impl Hold {
    pub const ALL: [Hold; 9] = [Hold::Invert, Hold::Smooth, Hold::Pick, Hold::WheelSize, Hold::WheelStrength, Hold::WheelNoise, Hold::Fast, Hold::AngleSnap, Hold::Precise];
    pub fn name(self) -> &'static str {
        match self {
            Hold::Invert => "Invert the brush (lower, erase)",
            Hold::Smooth => "Smooth instead",
            Hold::Pick => "Pick the surface under the cursor",
            Hold::WheelSize => "Wheel: brush size",
            Hold::WheelStrength => "Wheel: brush strength",
            Hold::WheelNoise => "Wheel: edge noise",
            Hold::Fast => "Fly faster",
            Hold::AngleSnap => "Snap to angle steps",
            Hold::Precise => "Move and turn slowly",
        }
    }
    pub fn default_key(self) -> ModKey {
        match self {
            Hold::Invert | Hold::WheelNoise => ModKey::Ctrl,
            Hold::Smooth | Hold::WheelSize | Hold::Fast => ModKey::Shift,
            Hold::Pick | Hold::WheelStrength | Hold::Precise => ModKey::Alt,
            Hold::AngleSnap => ModKey::Ctrl,
        }
    }
}

/// The keys the user chose; what is not listed keeps its default.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KeyMap {
    /// Shortcut text by action; empty leaves the action without a key.
    pub keys: BTreeMap<Action, String>,
    pub holds: BTreeMap<Hold, ModKey>,
}

impl KeyMap {
    pub fn get(&self, a: Action) -> Option<Shortcut> {
        match self.keys.get(&a) {
            Some(s) => Shortcut::parse(s),
            None => a.default_key(),
        }
    }
    pub fn set(&mut self, a: Action, s: Option<Shortcut>) {
        if s == a.default_key() {
            self.keys.remove(&a);
        } else {
            self.keys.insert(a, s.map(|s| s.text()).unwrap_or_default());
        }
    }
    pub fn hold(&self, h: Hold) -> ModKey {
        self.holds.get(&h).copied().unwrap_or(h.default_key())
    }
    pub fn held(&self, h: Hold, m: Modifiers) -> bool {
        self.hold(h).held(m)
    }
    /// The key of an action as text, or "" without one.
    pub fn text(&self, a: Action) -> String {
        self.get(a).map(|s| s.text()).unwrap_or_default()
    }
    /// The name of an action with its key, for tooltips.
    pub fn tip(&self, a: Action) -> String {
        match self.get(a) {
            Some(s) => format!("{}  ({})", t(a.name()), s.text()),
            None => t(a.name()),
        }
    }
    /// True once when the action's key was pressed; the key press is used up.
    pub fn pressed(&self, ctx: &egui::Context, a: Action) -> bool {
        let Some(s) = self.get(a) else { return false };
        ctx.input_mut(|i| {
            // Shift turns some keys into others ("[" into "{"), so the key in its place on the
            // keyboard counts as well.
            let at = i.events.iter().position(|e| matches!(e, egui::Event::Key { key, physical_key, pressed: true, repeat, modifiers } if (s.matches(*key, *modifiers) || physical_key.is_some_and(|p| s.matches(p, *modifiers))) && (!repeat || a.repeats())));
            match at {
                Some(at) => {
                    i.events.remove(at);
                    true
                }
                None => false,
            }
        })
    }
    /// True while the key of a held action is down.
    pub fn down(&self, i: &egui::InputState, a: Action) -> bool {
        self.get(a).is_some_and(|s| i.key_down(s.key))
    }
    /// Actions that share a key with another one they could clash with.
    pub fn clashes(&self) -> Vec<(Action, Action)> {
        let mut out = Vec::new();
        for (n, a) in Action::ALL.iter().enumerate() {
            for b in &Action::ALL[n + 1..] {
                if a.while_flying() != b.while_flying() {
                    continue;
                }
                if let (Some(x), Some(y)) = (self.get(*a), self.get(*b)) {
                    if x == y {
                        out.push((*a, *b));
                    }
                }
            }
        }
        out
    }
}

/// The window that changes the shortcuts. `capture` is the action waiting for its new key.
pub fn window(ctx: &egui::Context, keys: &mut KeyMap, capture: &mut Option<Action>, open: &mut bool) {
    // The next key pressed becomes the shortcut; Esc gives up, Backspace takes the key away.
    if let Some(a) = *capture {
        let got = ctx.input_mut(|i| {
            let at = i.events.iter().position(|e| matches!(e, egui::Event::Key { pressed: true, .. }));
            at.map(|at| i.events.remove(at))
        });
        if let Some(egui::Event::Key { key, modifiers, .. }) = got {
            match key {
                Key::Escape if modifiers.is_none() => {}
                Key::Backspace if modifiers.is_none() => keys.set(a, None),
                _ => keys.set(a, Some(Shortcut { key, ctrl: modifiers.ctrl || modifiers.command, shift: modifiers.shift, alt: modifiers.alt })),
            }
            *capture = None;
        }
    }
    let clashes = keys.clashes();
    let clashing = |a: Action| clashes.iter().any(|c| c.0 == a || c.1 == a);
    egui::Window::new(format!("{} {}", crate::icons::KEYBOARD, t("Keyboard shortcuts"))).collapsible(false).resizable(true).default_size([520.0, 620.0]).open(open).show(ctx, |ui| {
        ui.weak(t("Click a shortcut and press the new key with its modifiers. Esc cancels, Backspace leaves the action without a key."));
        if !clashes.is_empty() {
            ui.colored_label(theme::WARN, t("Some actions share a key; only the first of them will work."));
        }
        ui.horizontal(|ui| {
            if ui.button(format!("{} {}", crate::icons::ARROW_COUNTER_CLOCKWISE, t("Reset all to defaults"))).clicked() {
                *keys = KeyMap::default();
                *capture = None;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut group = "";
            for a in Action::ALL {
                if a.group() != group {
                    group = a.group();
                    ui.add_space(6.0);
                    ui.strong(t(group));
                }
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(230.0, 20.0), egui::Sense::hover());
                    ui.painter().text(egui::pos2(rect.left() + 8.0, rect.center().y), egui::Align2::LEFT_CENTER, t(a.name()), egui::FontId::proportional(13.0), theme::TEXT);
                    let waiting = *capture == Some(a);
                    let text = if waiting { t("Press a key...") } else { keys.get(a).map(|s| s.text()).unwrap_or_else(|| "—".into()) };
                    let mut b = egui::Button::new(egui::RichText::new(text).monospace().color(if clashing(a) { theme::WARN } else { theme::TEXT })).min_size(egui::vec2(140.0, 20.0));
                    if waiting { b = b.fill(theme::ACCENT_SOFT); }
                    if ui.add(b).clicked() { *capture = Some(a); }
                    if keys.get(a) != a.default_key() && ui.small_button(crate::icons::ARROW_COUNTER_CLOCKWISE).on_hover_text(format!("{}: {}", t("Default"), a.default_key().map(|s| s.text()).unwrap_or_else(|| "—".into()))).clicked() {
                        keys.set(a, a.default_key());
                    }
                });
            }
            ui.add_space(8.0);
            ui.strong(t("Held while using a brush"));
            ui.weak(t("The same for every brush: sculpting, painting, holes, placing and the clone brush."));
            for h in Hold::ALL {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(230.0, 20.0), egui::Sense::hover());
                    ui.painter().text(egui::pos2(rect.left() + 8.0, rect.center().y), egui::Align2::LEFT_CENTER, t(h.name()), egui::FontId::proportional(13.0), theme::TEXT);
                    let mut k = keys.hold(h);
                    egui::ComboBox::from_id_salt(("hold", h as u8)).width(140.0).selected_text(k.name()).show_ui(ui, |ui| {
                        for m in ModKey::ALL { ui.selectable_value(&mut k, m, m.name()); }
                    });
                    if k != keys.hold(h) {
                        if k == h.default_key() { keys.holds.remove(&h); } else { keys.holds.insert(h, k); }
                    }
                });
            }
        });
    });
    if !*open {
        *capture = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_read_back_and_defaults_are_not_stored() {
        for a in Action::ALL {
            if let Some(s) = a.default_key() {
                assert_eq!(Shortcut::parse(&s.text()), Some(s), "{a:?}");
            }
        }
        let mut k = KeyMap::default();
        k.set(Action::Save, Action::Save.default_key());
        assert!(k.keys.is_empty());
        k.set(Action::Save, Some(Shortcut::ctrl_shift(Key::W)));
        k.set(Action::Focus, None);
        let back: KeyMap = serde_json::from_str(&serde_json::to_string(&k).unwrap()).unwrap();
        assert_eq!(back.get(Action::Save), Some(Shortcut::ctrl_shift(Key::W)));
        assert_eq!(back.get(Action::Focus), None);
        assert!(Shortcut::ctrl(Key::Z).matches(Key::Z, Modifiers::COMMAND));
        assert!(!Shortcut::ctrl(Key::Z).matches(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT));
    }
    #[test]
    fn the_defaults_do_not_clash() {
        assert_eq!(KeyMap::default().clashes(), Vec::new());
    }
}

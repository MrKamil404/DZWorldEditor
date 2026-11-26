//! Keyboard shortcuts in one place: every action the keyboard reaches, its default key, the
//! keys the user chose instead and the window that changes them. The modifiers held while
//! using a brush are kept here too, so that every brush reads them the same way.
use crate::i18n::{t, tf};
use crate::theme;
use eframe::egui::{self, Color32, Key, Modifiers, RichText, Stroke};
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
    pub const fn alt(key: Key) -> Self {
        Self { key, ctrl: false, shift: false, alt: true }
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
    /// Fly the camera to a key point, a position or a square of the map grid.
    GoTo,
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
    /// The gizmo lays copies of the selection in a row.
    GizmoCopy,
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
    /// Key help shown in the panels or not.
    ToggleHints,
    /// Laying a road piece by piece: the chosen piece goes on.
    RoadPieceAdd,
    RoadPiecePrev,
    RoadPieceNext,
    /// Curves bend the other way.
    RoadPieceSide,
    CameraForward,
    CameraBack,
    CameraLeft,
    CameraRight,
    CameraUp,
    CameraDown,
    /// A copy of the selection laid one length further on along it, or back.
    CopyAhead,
    CopyBehind,
    /// The selection moves so its end meets the end of the object next to it.
    JoinNeighbour,
    /// Drops the selected objects onto the highest object surface or terrain underneath.
    DropOnSurface,
    /// Toggles placing objects on top of other objects instead of only on the terrain.
    TogglePlaceOnSurface,
    /// Temporarily hides the selected objects in the view.
    HideSelected,
    /// Shows all temporarily hidden objects again.
    ShowHidden,
    /// Hides everything except the selected objects (or restores all when already isolated).
    IsolateSelection,
}

impl Action {
    pub const ALL: [Action; 64] = [
        Action::NewProject, Action::Open, Action::Save, Action::SaveAs, Action::Undo, Action::Redo, Action::Settings, Action::Shortcuts, Action::Build, Action::Play,
        Action::SelectAll, Action::Deselect, Action::Duplicate, Action::CopyAhead, Action::CopyBehind, Action::JoinNeighbour, Action::DropOnSurface, Action::HideSelected, Action::ShowHidden, Action::IsolateSelection, Action::PasteHere, Action::PasteLast, Action::Delete, Action::Focus, Action::GoTo, Action::Underground,
        Action::ModeSelect, Action::ModeSculpt, Action::ModePaint, Action::ModePlace, Action::ModeRoads, Action::ModeShapes, Action::ModeKeyPoints, Action::ModeSatellite, Action::ModeMission,
        Action::GizmoMove, Action::GizmoRotate, Action::GizmoScale, Action::GizmoCopy, Action::GizmoSpace, Action::TogglePlaceOnSurface,
        Action::View3d, Action::View2d, Action::ViewSplit, Action::CameraBelow, Action::ToggleGrass, Action::ToggleObjects, Action::ToggleShapes,
        Action::BrushSmaller, Action::BrushBigger, Action::BrushWeaker, Action::BrushStronger, Action::SearchAssets, Action::ToggleHints,
        Action::RoadPieceAdd, Action::RoadPiecePrev, Action::RoadPieceNext, Action::RoadPieceSide,
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
            Action::CopyAhead => "Copy on ahead",
            Action::CopyBehind => "Copy on behind",
            Action::JoinNeighbour => "Join to the neighbour",
            Action::DropOnSurface => "Drop onto surface",
            Action::HideSelected => "Hide selected",
            Action::ShowHidden => "Show hidden objects",
            Action::IsolateSelection => "Isolate selection",
            Action::PasteHere => "Copy the selection to the cursor",
            Action::PasteLast => "Copy the last selection to the cursor",
            Action::Delete => "Delete",
            Action::Focus => "Frame the selection",
            Action::GoTo => "Go to",
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
            Action::GizmoCopy => "Copy in a row",
            Action::GizmoSpace => "World or local axes",
            Action::TogglePlaceOnSurface => "Place on top of objects",
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
            Action::ToggleHints => "Show key help",
            Action::RoadPieceAdd => "Lay the chosen piece",
            Action::RoadPiecePrev => "Previous piece",
            Action::RoadPieceNext => "Next piece",
            Action::RoadPieceSide => "Curve to the other side",
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
            SelectAll | Deselect | Duplicate | CopyAhead | CopyBehind | JoinNeighbour | DropOnSurface | HideSelected | ShowHidden | IsolateSelection | PasteHere | PasteLast | Delete | Focus | Underground => "Selection",
            ModeSelect | ModeSculpt | ModePaint | ModePlace | ModeRoads | ModeShapes | ModeKeyPoints | ModeSatellite | ModeMission | GizmoMove | GizmoRotate | GizmoScale | GizmoCopy | GizmoSpace | TogglePlaceOnSurface => "Tools",
            GoTo | View3d | View2d | ViewSplit | CameraBelow | ToggleGrass | ToggleObjects | ToggleShapes | SearchAssets | ToggleHints => "View",
            RoadPieceAdd | RoadPiecePrev | RoadPieceNext | RoadPieceSide => "Roads piece by piece",
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
            CopyAhead => Shortcut::ctrl_shift(Key::D),
            CopyBehind => Shortcut::ctrl_shift(Key::A),
            JoinNeighbour => Shortcut::new(Key::J),
            DropOnSurface => Shortcut::new(Key::End),
            HideSelected => Shortcut::ctrl(Key::H),
            ShowHidden => Shortcut::alt(Key::H),
            IsolateSelection => Shortcut::new(Key::Slash),
            PasteHere => Shortcut::new(Key::C),
            PasteLast => Shortcut::new(Key::V),
            Delete => Shortcut::new(Key::Delete),
            Focus => Shortcut::new(Key::F),
            GoTo => Shortcut::ctrl(Key::G),
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
            GizmoCopy => Shortcut::new(Key::T),
            GizmoSpace => Shortcut::new(Key::L),
            TogglePlaceOnSurface => Shortcut::shift(Key::End),
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
            ToggleHints => Shortcut::new(Key::F2),
            RoadPieceAdd => Shortcut::new(Key::Space),
            RoadPiecePrev => Shortcut::new(Key::Z),
            RoadPieceNext => Shortcut::new(Key::X),
            RoadPieceSide => Shortcut::new(Key::S),
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

fn group_icon(group: &str) -> &'static str {
    match group {
        "General" => crate::icons::GEAR,
        "Selection" => crate::icons::CURSOR,
        "Tools" => crate::icons::WRENCH,
        "View" => crate::icons::EYE,
        "Roads piece by piece" => crate::icons::PATH,
        "Brushes" => crate::icons::PAINT_BRUSH,
        _ => crate::icons::CAMERA,
    }
}

fn fit_frame(ui: &mut egui::Ui, frame: egui::Frame, add: impl FnOnce(&mut egui::Ui)) {
    frame.show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), add);
    });
}

/// Captures a pressed key when `capture` is active and renders the shortcuts editor inside `ui`.
/// Used both in the standalone Keyboard shortcuts window (`F1`) and in the Settings tab.
pub fn panel(ctx: &egui::Context, ui: &mut egui::Ui, keys: &mut KeyMap, capture: &mut Option<Action>) {
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
    let custom_count = Action::ALL.iter().filter(|&&a| keys.get(a) != a.default_key()).count()
        + Hold::ALL.iter().filter(|&&h| keys.hold(h) != h.default_key()).count();

    let search_id = ui.make_persistent_id("shortcuts_search");
    let mut query: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();

    // Top toolbar: search bar, custom counter, and Reset all button.
    ui.horizontal(|ui| {
        let reset_label = crate::icons::with(crate::icons::ARROW_COUNTER_CLOCKWISE, &t("Reset all to defaults"));
        egui::Frame::new()
            .fill(theme::FIELD)
            .stroke(Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(3)
            .inner_margin(egui::Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(crate::icons::MAGNIFYING_GLASS).color(theme::TEXT_DIM));
                    ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .hint_text(t("Search shortcuts..."))
                            .frame(false)
                            .desired_width(180.0),
                    );
                    if !query.is_empty() && ui.add(egui::Button::new(RichText::new(crate::icons::X).color(theme::TEXT_DIM)).frame(false)).clicked() {
                        query.clear();
                    }
                });
            });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(custom_count > 0, egui::Button::new(reset_label).min_size(egui::vec2(0.0, 24.0))).clicked() {
                *keys = KeyMap::default();
                *capture = None;
            }
            if custom_count > 0 {
                ui.label(RichText::new(tf("{0} custom", &[&custom_count.to_string()])).small().color(theme::TOGGLE_ON_TEXT));
            }
        });
    });
    ui.data_mut(|d| d.insert_temp(search_id, query.clone()));

    ui.add_space(3.0);
    crate::panels::about(ui, &t("Click a shortcut and press the new key with its modifiers. Esc cancels, Backspace leaves the action without a key."));
    if !clashes.is_empty() {
        ui.add_space(4.0);
        fit_frame(
            ui,
            egui::Frame::new()
                .fill(theme::DARK)
                .stroke(Stroke::new(1.0_f32, theme::WARN.gamma_multiply(0.75)))
                .corner_radius(4)
                .inner_margin(egui::Margin::symmetric(8, 6)),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(theme::WARN, crate::icons::WARNING);
                    ui.colored_label(theme::WARN, t("Some actions share a key; only the first of them will work."));
                });
            },
        );
    }
    ui.add_space(4.0);

    let needle = query.trim().to_lowercase();
    let matches_query = |name: &str, group: &str, key_text: &str| -> bool {
        if needle.is_empty() {
            return true;
        }
        name.to_lowercase().contains(&needle) || group.to_lowercase().contains(&needle) || key_text.to_lowercase().contains(&needle)
    };

    egui::ScrollArea::vertical().id_salt("shortcuts_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let mut shown_any = false;
        let mut i = 0usize;
        while i < Action::ALL.len() {
            let group = Action::ALL[i].group();
            let start = i;
            while i < Action::ALL.len() && Action::ALL[i].group() == group {
                i += 1;
            }
            let group_label = t(group);
            let group_actions: Vec<Action> = Action::ALL[start..i]
                .iter()
                .copied()
                .filter(|&a| {
                    let ktxt = keys.get(a).map(|s| s.text()).unwrap_or_default();
                    matches_query(&t(a.name()), &group_label, &ktxt)
                })
                .collect();
            if group_actions.is_empty() {
                continue;
            }
            shown_any = true;
            theme::section(ui, &crate::icons::with(group_icon(group), &group_label), true, |ui| {
                fit_frame(
                    ui,
                    egui::Frame::new()
                        .fill(theme::DARK)
                        .stroke(Stroke::new(1.0_f32, theme::BORDER))
                        .corner_radius(4)
                        .inner_margin(egui::Margin::same(4)),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for (idx, a) in group_actions.iter().copied().enumerate() {
                            let waiting = *capture == Some(a);
                            let current = keys.get(a);
                            let modified = current != a.default_key();
                            let clash = clashing(a);
                            let bg = if waiting {
                                theme::ACCENT_SOFT
                            } else if idx % 2 == 1 {
                                theme::ROW_ALT
                            } else {
                                Color32::TRANSPARENT
                            };
                            fit_frame(ui, egui::Frame::new().fill(bg).corner_radius(2).inner_margin(egui::Margin::symmetric(8, 3)), |ui| {
                                ui.horizontal(|ui| {
                                    let name_color = if waiting || modified { theme::TEXT_BRIGHT } else { theme::TEXT };
                                    ui.add(egui::Label::new(RichText::new(t(a.name())).color(name_color)).truncate());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        // Fixed 24 px slot for the per-action reset button so all keycaps align.
                                        if modified {
                                            let tip = format!("{}: {}", t("Default"), a.default_key().map(|s| s.text()).unwrap_or_else(|| "—".into()));
                                            let rbtn = egui::Button::new(RichText::new(crate::icons::ARROW_COUNTER_CLOCKWISE).color(theme::TEXT_DIM))
                                                .frame(false)
                                                .min_size(egui::vec2(22.0, 20.0));
                                            if ui.add(rbtn).on_hover_text(tip).clicked() {
                                                keys.set(a, a.default_key());
                                            }
                                        } else {
                                            ui.allocate_exact_size(egui::vec2(22.0, 20.0), egui::Sense::hover());
                                        }
                                        let text = if waiting {
                                            t("Press a key...")
                                        } else {
                                            current.map(|s| s.text()).unwrap_or_else(|| "—".into())
                                        };
                                        let (fill, stroke, txt_col) = if waiting {
                                            (theme::ACCENT, Stroke::new(1.0_f32, theme::ACCENT), Color32::WHITE)
                                        } else if clash {
                                            (Color32::from_rgb(60, 44, 24), Stroke::new(1.0_f32, theme::WARN), theme::WARN)
                                        } else if modified {
                                            (theme::TOGGLE_ON, Stroke::new(1.0_f32, theme::ACCENT.gamma_multiply(0.7)), theme::TOGGLE_ON_TEXT)
                                        } else {
                                            (theme::FIELD, Stroke::new(1.0_f32, theme::BORDER), if current.is_some() { theme::TEXT_BRIGHT } else { theme::TEXT_DIM })
                                        };
                                        let b = egui::Button::new(RichText::new(text).monospace().color(txt_col))
                                            .fill(fill)
                                            .stroke(stroke)
                                            .min_size(egui::vec2(142.0, 22.0));
                                        if ui.add(b).clicked() {
                                            *capture = if waiting { None } else { Some(a) };
                                        }
                                    });
                                });
                            });
                        }
                    },
                );
            });
        }

        let hold_group_label = t("Held while using a brush");
        let holds: Vec<Hold> = Hold::ALL
            .iter()
            .copied()
            .filter(|&h| matches_query(&t(h.name()), &hold_group_label, keys.hold(h).name()))
            .collect();
        if !holds.is_empty() {
            shown_any = true;
            theme::section(ui, &crate::icons::with(crate::icons::HAND, &hold_group_label), true, |ui| {
                crate::panels::about(ui, &t("The same for every brush: sculpting, painting, holes, placing and the clone brush."));
                ui.add_space(3.0);
                fit_frame(
                    ui,
                    egui::Frame::new()
                        .fill(theme::DARK)
                        .stroke(Stroke::new(1.0_f32, theme::BORDER))
                        .corner_radius(4)
                        .inner_margin(egui::Margin::same(4)),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        for (idx, h) in holds.iter().copied().enumerate() {
                            let mut k = keys.hold(h);
                            let modified = k != h.default_key();
                            let bg = if idx % 2 == 1 { theme::ROW_ALT } else { Color32::TRANSPARENT };
                            fit_frame(ui, egui::Frame::new().fill(bg).corner_radius(2).inner_margin(egui::Margin::symmetric(8, 2)), |ui| {
                                ui.horizontal(|ui| {
                                    ui.add(egui::Label::new(RichText::new(t(h.name())).color(if modified { theme::TEXT_BRIGHT } else { theme::TEXT })).truncate());
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if modified {
                                            let tip = format!("{}: {}", t("Default"), h.default_key().name());
                                            let rbtn = egui::Button::new(RichText::new(crate::icons::ARROW_COUNTER_CLOCKWISE).color(theme::TEXT_DIM))
                                                .frame(false)
                                                .min_size(egui::vec2(22.0, 20.0));
                                            if ui.add(rbtn).on_hover_text(tip).clicked() {
                                                keys.holds.remove(&h);
                                                k = h.default_key();
                                            }
                                        } else {
                                            ui.allocate_exact_size(egui::vec2(22.0, 20.0), egui::Sense::hover());
                                        }
                                        let mod_choices: Vec<(ModKey, String, String)> = ModKey::ALL.iter().map(|&m| (m, m.name().to_string(), String::new())).collect();
                                        if theme::segmented(ui, &mut k, &mod_choices) {
                                            if k == h.default_key() {
                                                keys.holds.remove(&h);
                                            } else {
                                                keys.holds.insert(h, k);
                                            }
                                        }
                                    });
                                });
                            });
                        }
                    },
                );
            });
        }

        if !shown_any {
            ui.add_space(12.0);
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(t("No matching shortcuts.")).color(theme::TEXT_DIM));
            });
        }
    });
}

/// The window that changes the shortcuts. `capture` is the action waiting for its new key.
pub fn window(ctx: &egui::Context, keys: &mut KeyMap, capture: &mut Option<Action>, open: &mut bool) {
    let title = crate::icons::with(crate::icons::KEYBOARD, &t("Keyboard shortcuts"));
    theme::PopupWindow::new(egui::Id::new("keyboard_shortcuts"), &title)
        .default_size([600.0, 620.0])
        .min_size([440.0, 360.0])
        .show(ctx, open, |ui| {
            panel(ctx, ui, keys, capture);
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

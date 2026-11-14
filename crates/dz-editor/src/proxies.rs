//! Models a building places through proxies in its visual LOD: the furniture and fittings the
//! game shows inside it. Proxies of the other LODs (geometry, fire, memory, roadway…) are not
//! read.
use dz_core::model_info::ModelDb;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc};

/// One model placed in a building, in the building's model coordinates.
pub struct ProxyPlace {
    /// Index into [`Proxies::models`].
    pub model: u32,
    /// Images of the model's axes.
    pub axes: [[f32; 3]; 3],
    pub at: [f32; 3],
}

type Found = (String, Vec<ProxyPlace>);

/// The proxies of the building models seen so far, read off the interface thread.
pub struct Proxies {
    /// Paths of the models proxies place.
    pub models: Vec<String>,
    index: HashMap<String, u32>,
    of: HashMap<String, Arc<Vec<ProxyPlace>>>,
    pending: HashSet<String>,
    tx: mpsc::Sender<(String, Vec<(String, [f32; 12])>, [f32; 3])>,
    rx: mpsc::Receiver<(String, Vec<(String, [f32; 12])>, [f32; 3])>,
    /// Goes up whenever the proxies of another model come in.
    pub rev: u64,
}

impl Default for Proxies {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { models: Vec::new(), index: HashMap::new(), of: HashMap::new(), pending: HashSet::new(), tx, rx, rev: 0 }
    }
}

/// The path a proxy names, as the model database spells paths.
fn model_path(name: &str) -> String {
    let p = name.trim().trim_start_matches(['\\', '/']).replace('/', "\\").to_ascii_lowercase();
    if p.ends_with(".p3d") { p } else { format!("{p}.p3d") }
}

impl Proxies {
    /// The proxies of the model at `path`, once they are read; the first ask starts the reading.
    pub fn get(&mut self, path: &str, db: &ModelDb, ctx: &eframe::egui::Context) -> Option<Arc<Vec<ProxyPlace>>> {
        let key = path.to_ascii_lowercase();
        if let Some(found) = self.of.get(&key) {
            return Some(found.clone());
        }
        if self.pending.insert(key.clone()) {
            let (file, center, tx, ctx) = (db.file(path), db.get(path).center, self.tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                // Models saved by Object Builder (MLOD) have their proxies where the mesh is; the
                // binarised ones less the centre the mesh is drawn with.
                let (list, centred) = forest_core::model3d::visual_proxies(&file).unwrap_or_default();
                let _ = tx.send((key, list, if centred { center } else { [0.0; 3] }));
                ctx.request_repaint();
            });
        }
        None
    }

    /// Takes in what the readers found. True when something came in.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok((parent, list, center)) = self.rx.try_recv() {
            let found: Found = (parent, list.into_iter().filter(|(name, _)| !name.trim().is_empty()).map(|(name, m)| {
                let path = model_path(&name);
                let next = self.models.len() as u32;
                let model = *self.index.entry(path.clone()).or_insert(next);
                if model == next {
                    self.models.push(path);
                }
                // The matrix holds the axes, then the position, in the coordinates of the LOD,
                // which are those of the model less its centre.
                ProxyPlace { model, axes: [[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]], at: [m[9] + center[0], m[10] + center[1], m[11] + center[2]] }
            }).collect());
            self.pending.remove(&found.0);
            self.of.insert(found.0, Arc::new(found.1));
            any = true;
        }
        if any {
            self.rev += 1;
        }
        any
    }
}

/// `p` placed by the axes and position of a building, then turned into the instance of a proxy:
/// the images of the proxy model's axes and where its origin goes.
pub fn place(building: &[[f32; 3]; 3], origin: [f32; 3], p: &ProxyPlace) -> ([[f32; 3]; 3], [f32; 3]) {
    let apply = |v: [f32; 3]| -> [f32; 3] { std::array::from_fn(|k| building[0][k] * v[0] + building[1][k] * v[1] + building[2][k] * v[2]) };
    let axes = [apply(p.axes[0]), apply(p.axes[1]), apply(p.axes[2])];
    let at = apply(p.at);
    (axes, [origin[0] + at[0], origin[1] + at[1], origin[2] + at[2]])
}

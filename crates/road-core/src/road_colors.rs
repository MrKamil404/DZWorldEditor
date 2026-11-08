//! Road type colours stored in `.dzroad` documents.
use std::collections::BTreeMap;

pub type Palette = BTreeMap<String, [u8; 3]>;
pub const DEFAULT: [u8; 3] = [92, 182, 236];

pub fn family(model: &str) -> String {
    model.rsplit(['\\', '/']).next().unwrap_or(model).split('_').next().unwrap_or("").to_owned()
}

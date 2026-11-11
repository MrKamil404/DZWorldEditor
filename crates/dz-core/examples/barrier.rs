//! Prints how the objects of a model near a point are stored and where their ends come out.
//! `barrier <project> <x> <z> <radius> <part of the model name>`
use dz_core::model_info::{skew, tilted_lift, ModelDb};
use dz_core::objects::{rotation, to_terrain_builder, ObjectStore};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let db0 = ModelDb::new("P:/");
    let w = if a[0].to_ascii_lowercase().ends_with(".wrp") { dz_core::import::import_wrp(std::path::Path::new(&a[0]), None, &db0, &dz_core::Job::default()).unwrap() } else { dz_core::project::load(std::path::Path::new(&a[0])).unwrap() };
    let (x, z, r): (f32, f32, f32) = (a[1].parse().unwrap(), a[2].parse().unwrap(), a[3].parse().unwrap());
    let db = ModelDb::new("P:/");
    w.objects.for_each_in(x - r, z - r, x + r, z + r, |id, o| {
        let path = w.objects.model_path(o);
        if !path.contains(a[4].as_str()) {
            return;
        }
        let info = db.get(path);
        let ground = w.height.sample(o.x, o.z);
        let y = ObjectStore::origin_y(o, &w.height);
        let tb = to_terrain_builder(o.yaw, o.pitch, o.roll);
        println!("#{id} {} at {:.2} {:.2} elevation {:.3} flags {} scale {:.2}", path.rsplit('\\').next().unwrap(), o.x, o.z, o.elevation, o.flags, o.scale);
        println!("   editor yaw {:.2} pitch {:.2} roll {:.2} | terrain builder yaw {:.2} pitch {:.2} roll {:.2} | placement {:?}", o.yaw, o.pitch, o.roll, tb.0, tb.1, tb.2, info.placement);
        let m = rotation(o.yaw, o.pitch, o.roll, o.scale);
        let sk = skew(o, &info, &w.height);
        let lift = tilted_lift(o, &info, &w.height);
        println!("   lift of a tilted model {lift:.2}");
        for end in [info.min[0], 0.0, info.max[0]] {
            let (wx, wz) = (o.x + m[0][0] * end, o.z + m[0][2] * end);
            let stored = y + lift + m[0][1] * end;
            let sheared = sk.map(|s| y + s.lift + s.along_x * end);
            println!("   model x {end:6.2}: ground {:.2}  origin height as drawn {:.2} ({:+.2} over ground)  sheared {:?}", w.height.sample(wx, wz), stored, stored - w.height.sample(wx, wz), sheared.map(|v| v - w.height.sample(wx, wz)));
        }
        println!("   ground at origin {ground:.2}  right {:?} up {:?}", m[0].map(|v| (v * 1000.0).round() / 1000.0), m[1].map(|v| (v * 1000.0).round() / 1000.0));
    });
}

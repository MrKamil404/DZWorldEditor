//! Finds DayZ Tools and the game, packs the map and starts a local test session.
use crate::Job;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct ToolPaths {
    pub work_drive: PathBuf,
    pub dayz_tools: PathBuf,
    pub dayz: PathBuf,
    /// Private key (`.biprivatekey`) the built PBOs are signed with; empty leaves them unsigned.
    #[serde(default)]
    pub sign_key: PathBuf,
}

#[cfg(windows)]
fn hide(c: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    c.creation_flags(0x0800_0000)
}
#[cfg(not(windows))]
fn hide(c: &mut Command) -> &mut Command {
    c
}

fn reg_value(key: &str, value: &str) -> Option<String> {
    let out = hide(Command::new("reg").args(["query", key, "/v", value])).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.trim_start().to_ascii_lowercase().starts_with(&value.to_ascii_lowercase()))?;
    let (_, v) = line.split_once("REG_SZ")?;
    Some(v.trim().to_string())
}

fn steam_libraries() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(steam) = reg_value("HKCU\\Software\\Valve\\Steam", "SteamPath") else { return out };
    let steam = PathBuf::from(steam);
    out.push(steam.clone());
    if let Ok(text) = std::fs::read_to_string(steam.join("steamapps/libraryfolders.vdf")) {
        for l in text.lines() {
            if l.trim_start().starts_with("\"path\"") {
                if let Some(v) = l.split('"').nth(3) {
                    out.push(PathBuf::from(v.replace("\\\\", "\\")));
                }
            }
        }
    }
    out
}

/// Best guess of where everything is; every field may be corrected in the settings.
pub fn detect() -> ToolPaths {
    let mut p = ToolPaths { work_drive: PathBuf::from("P:\\"), ..Default::default() };
    if let Some(v) = reg_value("HKCU\\Software\\bohemia interactive\\Dayz Tools", "path") {
        p.dayz_tools = PathBuf::from(v);
    }
    let libraries = steam_libraries();
    for lib in &libraries {
        let c = lib.join("steamapps/common");
        if !p.dayz_tools.join("Bin/AddonBuilder/AddonBuilder.exe").is_file() && c.join("DayZ Tools/Bin/AddonBuilder/AddonBuilder.exe").is_file() {
            p.dayz_tools = c.join("DayZ Tools");
        }
        if !p.dayz.join("DayZDiag_x64.exe").is_file() && c.join("DayZ/DayZDiag_x64.exe").is_file() {
            p.dayz = c.join("DayZ");
        }
    }
    p
}

impl ToolPaths {
    pub fn sign_tool(&self) -> PathBuf {
        self.dayz_tools.join("Bin/DsUtils/DSSignFile.exe")
    }
    pub fn addon_builder(&self) -> PathBuf {
        self.dayz_tools.join("Bin/AddonBuilder/AddonBuilder.exe")
    }
    pub fn navmesh_generator(&self) -> PathBuf {
        self.dayz_tools.join("Bin/NavMeshGenerator/NavMeshGenerator_x64.exe")
    }
    pub fn diag(&self) -> PathBuf {
        self.dayz.join("DayZDiag_x64.exe")
    }
    /// Problems to show the user before building.
    pub fn check(&self) -> Vec<String> {
        let mut v = Vec::new();
        if !self.work_drive.is_dir() {
            v.push(format!("Work drive {} is not mounted", self.work_drive.display()));
        }
        if !self.addon_builder().is_file() {
            v.push("AddonBuilder.exe not found; set the DayZ Tools folder".into());
        }
        if !self.diag().is_file() {
            v.push("DayZDiag_x64.exe not found; set the DayZ folder".into());
        }
        v
    }
}

fn stream(mut cmd: Command, job: &Job) -> Result<i32> {
    let mut child = hide(cmd.stdout(Stdio::piped()).stderr(Stdio::piped())).spawn().context("Could not start the tool")?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let j = job.clone();
    let t = std::thread::spawn(move || {
        for l in BufReader::new(err).lines().map_while(Result::ok) {
            j.log(l);
        }
    });
    for l in BufReader::new(out).lines().map_while(Result::ok) {
        job.log(l);
        if job.cancelled() {
            let _ = child.kill();
            break;
        }
    }
    let _ = t.join();
    let code = child.wait()?.code().unwrap_or(-1);
    Ok(code)
}

pub fn mod_folder(paths: &ToolPaths, id: &str) -> PathBuf {
    paths.work_drive.join("Mods").join(format!("@{id}"))
}

/// Folders of the export packed each into a PBO of its own; `surfaces` holds the map's own
/// and changed surfaces (CfgSurfaces) when it has any.
pub const MAP_PARTS: [&str; 5] = ["world", "data", "navmesh", "surfaces", "ce"];

fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
}

/// A folder of the work drive packed with the map into a PBO of its own: buildings and other
/// content of the map's mod.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct BuildFolder {
    /// Relative to the work drive, e.g. `MyMap\structures`; it is also the PBO prefix.
    pub folder: String,
    pub enabled: bool,
    /// Binarised with AddonBuilder (models, configs); off packs the files as they are, for
    /// models that are binarised already.
    pub binarize: bool,
}

impl BuildFolder {
    pub fn new(folder: &str) -> Self {
        Self { folder: normalize(folder), enabled: true, binarize: true }
    }
    /// The PBO's file name without `.pbo`: the part after the map's folder, or the whole path.
    pub fn pbo_name(&self, id: &str) -> String {
        let f = normalize(&self.folder);
        let inside = f.len() > id.len() + 1 && f[..id.len()].eq_ignore_ascii_case(id) && f.as_bytes()[id.len()] == b'\\';
        let name = if inside { &f[id.len() + 1..] } else { &f[..] };
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
    }
}

fn normalize(folder: &str) -> String {
    folder.trim().replace('/', "\\").trim_matches('\\').to_string()
}

/// Folders inside `P:\<id>` that the export does not write, which may hold buildings and other
/// content of the mod, as `<id>\<folder>`.
pub fn mod_folders(work_drive: &Path, id: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(work_drive.join(id)) else { return Vec::new() };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.') && !MAP_PARTS.iter().any(|p| p.eq_ignore_ascii_case(n)) && !n.eq_ignore_ascii_case("source"))
        .map(|n| format!("{id}\\{n}"))
        .collect();
    out.sort_by_key(|n| n.to_ascii_lowercase());
    out
}

/// Runs AddonBuilder on `source`, writing its PBO into `out`.
fn addon_builder(paths: &ToolPaths, source: &Path, out: &Path, prefix: &str, temp: &Path, include: &Path, job: &Job) -> Result<i32> {
    let mut c = Command::new(paths.addon_builder());
    c.current_dir(&paths.work_drive)
        .arg(source)
        .arg(out)
        .arg(format!("-prefix={prefix}"))
        .arg(format!("-temp={}", temp.display()))
        .arg(format!("-project={}", paths.work_drive.display()))
        .arg(format!("-include={}", include.display()))
        .arg("-clear");
    let code = stream(c, job)?;
    if job.cancelled() {
        bail!("Cancelled");
    }
    Ok(code)
}

/// Binarises what the export wrote in `P:\<id>` with AddonBuilder, one PBO per folder, into
/// `P:\Mods\@<id>\Addons`, then the folders of `extra` that are switched on. Anything else in
/// `P:\<id>` is left alone.
pub fn build_pbo(paths: &ToolPaths, id: &str, extra: &[BuildFolder], job: &Job) -> Result<PathBuf> {
    ensure!(paths.addon_builder().is_file(), "AddonBuilder.exe not found");
    let root = paths.work_drive.join(id);
    ensure!(root.join("world").is_dir(), "Export the map first: {} does not exist", root.join("world").display());
    let target = mod_folder(paths, id).join("Addons");
    std::fs::create_dir_all(&target)?;
    let temp = paths.work_drive.join("temp").join(format!("dzwe_{id}"));
    std::fs::create_dir_all(&temp)?;
    // AddonBuilder only copies listed extensions.
    let include = temp.join("include.lst");
    std::fs::write(&include, "*.paa;*.rvmat;*.wrp;*.nm;*.xml;*.bin;*.cfg;*.hpp;*.csv;*.json;*.txt;*.layout;*.emat;*.edds;*.ptc;*.bisurf;*.ogg;*.wss;*.lip;*.c;*.map")?;
    // Earlier versions packed the whole map into one PBO, which would now load twice.
    let _ = std::fs::remove_file(target.join(format!("{id}.pbo")));
    let mut first = None;
    let mut built: Vec<PathBuf> = Vec::new();
    for part in MAP_PARTS {
        let source = root.join(part);
        let pbo = target.join(format!("{part}.pbo"));
        if !has_files(&source) {
            // Surfaces or a mission removed from the project must not stay in the mod.
            if part == "surfaces" || part == "ce" {
                let _ = std::fs::remove_file(&pbo);
                remove_signatures(&pbo);
            }
            continue;
        }
        // A failed build must leave the last good PBO in place.
        let started = std::time::SystemTime::now() - std::time::Duration::from_secs(2);
        job.stage(format!("Binarising {part} with AddonBuilder"));
        let code = addon_builder(paths, &source, &target, &format!("{id}\\{part}"), &temp, &include, job)?;
        ensure!(pbo.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= started), "AddonBuilder finished with code {code} and produced no new {part}.pbo; see the log above");
        job.log(format!("{} ({:.1} MB)", pbo.display(), pbo.metadata()?.len() as f64 / 1_048_576.0));
        built.push(pbo.clone());
        first.get_or_insert(pbo);
    }
    let mut names: Vec<String> = MAP_PARTS.iter().map(|p| p.to_string()).collect();
    for f in extra.iter().filter(|f| f.enabled && !normalize(&f.folder).is_empty()) {
        let prefix = normalize(&f.folder);
        let source = paths.work_drive.join(&prefix);
        if !has_files(&source) {
            job.log(format!("Skipped {}: the folder is empty or missing", source.display()));
            continue;
        }
        let name = f.pbo_name(id);
        ensure!(!names.iter().any(|n| n.eq_ignore_ascii_case(&name)), "Two folders would both be packed as {name}.pbo; rename one of them");
        names.push(name.clone());
        let pbo = target.join(format!("{name}.pbo"));
        if f.binarize {
            // Built apart, so a folder named like a part of the map cannot overwrite it.
            let out = temp.join("out");
            let _ = std::fs::remove_dir_all(&out);
            std::fs::create_dir_all(&out)?;
            job.stage(format!("Binarising {prefix} with AddonBuilder"));
            let code = addon_builder(paths, &source, &out, &prefix, &temp, &include, job)?;
            let made = std::fs::read_dir(&out)?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pbo")));
            let made = made.with_context(|| format!("AddonBuilder finished with code {code} and produced no PBO of {prefix}; see the log above"))?;
            std::fs::copy(&made, &pbo)?;
            let _ = std::fs::remove_dir_all(&out);
        } else {
            job.stage(format!("Packing {prefix}"));
            crate::pbo::pack(&source, &pbo, &prefix)?;
        }
        job.log(format!("{} ({:.1} MB)", pbo.display(), pbo.metadata()?.len() as f64 / 1_048_576.0));
        built.push(pbo);
    }
    if !paths.sign_key.as_os_str().is_empty() {
        sign(paths, id, &built, job)?;
    }
    first.context("Nothing to pack")
}

/// Signatures of a PBO by any key: `<file>.pbo.<key>.bisign`.
fn remove_signatures(pbo: &Path) {
    let (Some(dir), Some(name)) = (pbo.parent(), pbo.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase())) else { return };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let f = e.file_name().to_string_lossy().to_ascii_lowercase();
        if f.starts_with(&format!("{name}.")) && f.ends_with(".bisign") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The public key (`.bikey`) of a private one: next to it, with the same name.
pub fn public_key(private: &Path) -> PathBuf {
    private.with_extension("bikey")
}

/// Signs the PBOs with the private key of the paths, as servers that check signatures want:
/// each gets its `.bisign`, signatures by other keys go, and the public key is put into
/// `Keys` of the mod for servers to take.
pub fn sign(paths: &ToolPaths, id: &str, pbos: &[PathBuf], job: &Job) -> Result<()> {
    let key = &paths.sign_key;
    ensure!(key.extension().is_some_and(|e| e.eq_ignore_ascii_case("biprivatekey")) && key.is_file(), "The signing key {} is not a .biprivatekey file", key.display());
    ensure!(paths.sign_tool().is_file(), "DSSignFile.exe not found in DayZ Tools (Bin\\DsUtils)");
    job.stage(format!("Signing {} PBOs", pbos.len()));
    for pbo in pbos {
        remove_signatures(pbo);
        let mut c = Command::new(paths.sign_tool());
        c.current_dir(pbo.parent().unwrap_or(Path::new("."))).arg(key).arg(pbo);
        let code = stream(c, job)?;
        // Named after the authority inside the key, which is usually the key's file name.
        let file = pbo.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let signed = pbo.parent().and_then(|d| std::fs::read_dir(d).ok()).is_some_and(|rd| rd.flatten().any(|e| {
            let f = e.file_name().to_string_lossy().to_ascii_lowercase();
            f.starts_with(&format!("{file}.")) && f.ends_with(".bisign")
        }));
        ensure!(code == 0 && signed, "DSSignFile finished with code {code} and wrote no signature for {}", pbo.display());
    }
    let public = public_key(key);
    if public.is_file() {
        let keys = mod_folder(paths, id).join("Keys");
        std::fs::create_dir_all(&keys)?;
        std::fs::copy(&public, keys.join(public.file_name().unwrap_or_default()))?;
        job.log(format!("Signed with {}; its public key is in {}", key.display(), keys.display()));
    } else {
        job.log(format!("Signed with {}; no {} next to it to give servers", key.display(), public.display()));
    }
    Ok(())
}

/// Prepares `mpmissions/dayzOffline.<id>` from a vanilla mission with a spawn on the new map.
pub fn prepare_mission(paths: &ToolPaths, id: &str, spawn: [f32; 2], job: &Job) -> Result<PathBuf> {
    let server = paths.dayz.parent().map(|p| p.join("DayZServer").join("mpmissions")).unwrap_or_default();
    let name = format!("dayzOffline.{id}");
    let target = paths.dayz.join("mpmissions").join(&name);
    if !target.join("init.c").is_file() {
        let mut template = None;
        for dir in [paths.dayz.join("mpmissions"), server] {
            for t in ["dayzOffline.enoch", "dayzOffline.chernarusplus"] {
                if template.is_none() && dir.join(t).join("init.c").is_file() {
                    template = Some(dir.join(t));
                }
            }
        }
        let template = template.context("No vanilla mission (dayzOffline.enoch) found in mpmissions of DayZ or DayZServer")?;
        job.log(format!("Mission template: {}", template.display()));
        copy_mission(&template, &target)?;
    }
    write_spawn_points(&target.join("cfgplayerspawnpoints.xml"), spawn)?;
    Ok(target)
}

fn copy_mission(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with("storage_") || name.contains("_storage") {
            continue;
        }
        if e.file_type()?.is_dir() {
            copy_mission(&e.path(), &to.join(&name))?;
        } else {
            std::fs::copy(e.path(), to.join(&name))?;
        }
    }
    Ok(())
}

pub fn write_spawn_points(path: &Path, spawn: [f32; 2]) -> Result<()> {
    let group = |tag: &str| format!(
r#"    <{tag}>
        <spawn_params>
            <min_dist_infected>0</min_dist_infected>
            <max_dist_infected>9999</max_dist_infected>
            <min_dist_player>0</min_dist_player>
            <max_dist_player>9999</max_dist_player>
            <min_dist_static>0</min_dist_static>
            <max_dist_static>9999</max_dist_static>
        </spawn_params>
        <generator_params>
            <grid_density>4</grid_density>
            <grid_width>40</grid_width>
            <grid_height>40</grid_height>
            <min_dist_static>0</min_dist_static>
            <max_dist_static>9999</max_dist_static>
            <min_steepness>-45</min_steepness>
            <max_steepness>45</max_steepness>
        </generator_params>
        <generator_posbubbles>
            <pos x="{:.1}" z="{:.1}" />
        </generator_posbubbles>
    </{tag}>
"#, spawn[0], spawn[1]);
    let text = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>\n<playerspawnpoints>\n{}{}{}</playerspawnpoints>\n", group("fresh"), group("hop"), group("travel"));
    crate::fsutil::atomic_write(path, text.as_bytes())
}

pub struct Session {
    pub server: std::process::Child,
    pub client: Option<std::process::Child>,
}

/// Writes the local server configuration of the map.
pub fn write_server_config(dir: &Path, id: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("serverDZ.cfg");
    let text = format!(
        "hostname = \"DZ World Editor test: {id}\";\npassword = \"\";\npasswordAdmin = \"\";\nmaxPlayers = 4;\nverifySignatures = 0;\nforceSameBuild = 1;\ndisableVoN = 1;\nvonCodecQuality = 7;\ndisable3rdPerson = 0;\ndisableCrosshair = 0;\nserverTime = \"SystemTime\";\nserverTimeAcceleration = 1;\nserverTimePersistent = 0;\nguaranteedUpdates = 1;\nloginQueueConcurrentPlayers = 5;\nloginQueueMaxPlayers = 500;\ninstanceId = 1;\nstorageAutoFix = 1;\nallowFilePatching = 1;\nsteamQueryPort = 2305;\nclass Missions\n{{\n\tclass DayZ\n\t{{\n\t\ttemplate = \"dayzOffline.{id}\";\n\t}};\n}};\n"
    );
    crate::fsutil::atomic_write(&path, text.as_bytes())?;
    Ok(path)
}

/// Starts server and optionally a client connected to it.
pub fn launch(paths: &ToolPaths, id: &str, session_dir: &Path, with_client: bool, extra_mods: &str, job: &Job) -> Result<Session> {
    ensure!(paths.diag().is_file(), "DayZDiag_x64.exe not found");
    let mission = paths.dayz.join("mpmissions").join(format!("dayzOffline.{id}"));
    ensure!(mission.join("init.c").is_file(), "The test mission is missing");
    let cfg = write_server_config(session_dir, id)?;
    let (server_profile, client_profile) = (session_dir.join("server"), session_dir.join("client"));
    std::fs::create_dir_all(&server_profile)?;
    std::fs::create_dir_all(&client_profile)?;
    let map_mod = mod_folder(paths, id).display().to_string();
    let mods = if extra_mods.trim().is_empty() { map_mod } else { format!("{};{map_mod}", extra_mods.trim().trim_end_matches(';')) };
    job.stage("Starting local server");
    let server = Command::new(paths.diag())
        .current_dir(&paths.dayz)
        .arg("-server")
        .arg(format!("-config={}", cfg.display()))
        .arg(format!("-profiles={}", server_profile.display()))
        .arg(format!("-mission={}", mission.display()))
        .arg(format!("-mod={mods}"))
        .arg("-port=2302")
        .arg("-doLogs")
        .arg("-newErrorsAreWarnings=1")
        .spawn()
        .context("Could not start DayZDiag as server")?;
    let client = if with_client {
        job.stage("Starting game");
        Some(Command::new(paths.diag())
            .current_dir(&paths.dayz)
            .arg("-window")
            .arg(format!("-profiles={}", client_profile.display()))
            .arg(format!("-mod={mods}"))
            .arg("-connect=127.0.0.1")
            .arg("-port=2302")
            .arg("-doLogs")
            .arg("-newErrorsAreWarnings=1")
            .spawn()
            .context("Could not start the DayZDiag client")?)
    } else {
        None
    };
    Ok(Session { server, client })
}

/// Server exporting world data plus the navmesh generator window connected to it.
pub fn launch_navmesh(paths: &ToolPaths, id: &str, session_dir: &Path, job: &Job) -> Result<std::process::Child> {
    ensure!(paths.diag().is_file(), "DayZDiag_x64.exe not found");
    ensure!(paths.navmesh_generator().is_file(), "NavMeshGenerator_x64.exe not found");
    let mission = paths.dayz.join("mpmissions").join(format!("dayzOffline.{id}"));
    let cfg = write_server_config(session_dir, id)?;
    let profile = session_dir.join("navmesh");
    std::fs::create_dir_all(&profile)?;
    std::fs::create_dir_all(paths.work_drive.join(id).join("navmesh"))?;
    job.stage("Starting navmesh data server");
    let server = Command::new(paths.diag())
        .current_dir(&paths.dayz)
        .arg("-server")
        .arg("-startNavmeshDataServer")
        .arg(format!("-config={}", cfg.display()))
        .arg(format!("-profiles={}", profile.display()))
        .arg(format!("-mission={}", mission.display()))
        .arg(format!("-mod={}", mod_folder(paths, id).display()))
        .arg("-port=2302")
        .arg("-newErrorsAreWarnings=1")
        .spawn()
        .context("Could not start DayZDiag for the navmesh")?;
    Command::new(paths.navmesh_generator()).current_dir(paths.navmesh_generator().parent().unwrap()).spawn().context("Could not start NavMeshGenerator")?;
    Ok(server)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signing_a_pbo_with_a_real_key() {
        // DZ_SIGN_KEY: a .biprivatekey; DayZ Tools found as the editor finds them.
        let Ok(key) = std::env::var("DZ_SIGN_KEY") else { return };
        let d = tempfile::tempdir().unwrap();
        let mut paths = detect();
        paths.work_drive = d.path().to_path_buf();
        paths.sign_key = key.into();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        std::fs::write(d.path().join("src/config.cpp"), "class CfgPatches { class T { units[]={}; }; };").unwrap();
        let pbo = d.path().join("Mods/@T/Addons/t.pbo");
        std::fs::create_dir_all(pbo.parent().unwrap()).unwrap();
        crate::pbo::pack(&d.path().join("src"), &pbo, "T").unwrap();
        // An old signature by another key goes.
        std::fs::write(d.path().join("Mods/@T/Addons/t.pbo.Old.bisign"), b"x").unwrap();
        sign(&paths, "T", &[pbo.clone()], &Job::default()).unwrap();
        let names: Vec<String> = std::fs::read_dir(pbo.parent().unwrap()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        println!("{names:?}");
        assert!(names.iter().any(|n| n.ends_with(".bisign")) && !names.iter().any(|n| n == "t.pbo.Old.bisign"));
        assert!(std::fs::read_dir(d.path().join("Mods/@T/Keys")).unwrap().count() == 1);
    }

    #[test]
    fn extra_folders_get_names_and_are_found() {
        assert_eq!(BuildFolder::new("MyMap\\structures").pbo_name("MyMap"), "structures");
        assert_eq!(BuildFolder::new("mymap/structures/houses/").pbo_name("MyMap"), "structures_houses");
        assert_eq!(BuildFolder::new("\\OtherMod\\buildings").pbo_name("MyMap"), "OtherMod_buildings");
        assert_eq!(BuildFolder::new("MyMapX\\data").pbo_name("MyMap"), "MyMapX_data");
        let d = tempfile::tempdir().unwrap();
        for f in ["MyMap/world", "MyMap/structures", "MyMap/navmesh", "MyMap/Props"] {
            std::fs::create_dir_all(d.path().join(f)).unwrap();
        }
        assert_eq!(mod_folders(d.path(), "MyMap"), vec!["MyMap\\Props".to_string(), "MyMap\\structures".to_string()]);
    }
    #[test]
    fn spawn_points_contain_the_position() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.xml");
        write_spawn_points(&p, [123.5, 456.5]).unwrap();
        let t = std::fs::read_to_string(p).unwrap();
        assert_eq!(t.matches("<pos x=\"123.5\" z=\"456.5\" />").count(), 3);
    }
    #[test]
    fn missing_tools_are_reported() {
        let p = ToolPaths { work_drive: "Z:/nope".into(), dayz_tools: "Z:/x".into(), dayz: "Z:/y".into(), ..Default::default() };
        assert_eq!(p.check().len(), 3);
    }
    #[test]
    fn server_config_uses_the_map_mission() {
        let d = tempfile::tempdir().unwrap();
        let t = std::fs::read_to_string(write_server_config(d.path(), "Foo").unwrap()).unwrap();
        assert!(t.contains("template = \"dayzOffline.Foo\";"));
    }
}

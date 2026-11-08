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
    paths.dayz.join(format!("@{id}"))
}

/// Binarises `P:\<id>` with AddonBuilder into `@<id>\addons` of the game folder.
pub fn build_pbo(paths: &ToolPaths, id: &str, job: &Job) -> Result<PathBuf> {
    ensure!(paths.addon_builder().is_file(), "AddonBuilder.exe not found");
    let source = paths.work_drive.join(id);
    ensure!(source.is_dir(), "Export the map first: {} does not exist", source.display());
    let target = mod_folder(paths, id).join("addons");
    std::fs::create_dir_all(&target)?;
    let temp = paths.work_drive.join("temp").join(format!("dzwe_{id}"));
    std::fs::create_dir_all(&temp)?;
    // AddonBuilder only copies listed extensions.
    let include = temp.join("include.lst");
    std::fs::write(&include, "*.paa;*.rvmat;*.wrp;*.nm;*.xml;*.bin;*.cfg;*.hpp;*.csv;*.json;*.txt;*.layout;*.emat;*.edds;*.ptc;*.bisurf")?;
    job.stage("Binarising with AddonBuilder");
    let mut c = Command::new(paths.addon_builder());
    c.current_dir(&paths.work_drive)
        .arg(&source)
        .arg(&target)
        .arg(format!("-prefix={id}"))
        .arg(format!("-temp={}", temp.display()))
        .arg(format!("-project={}", paths.work_drive.display()))
        .arg(format!("-include={}", include.display()))
        .arg("-clear");
    let code = stream(c, job)?;
    if job.cancelled() {
        bail!("Cancelled");
    }
    let pbo = target.join(format!("{id}.pbo"));
    ensure!(pbo.is_file(), "AddonBuilder finished with code {code} and produced no PBO; see the log above");
    let size = pbo.metadata()?.len();
    job.log(format!("{} ({:.1} MB)", pbo.display(), size as f64 / 1_048_576.0));
    Ok(pbo)
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
    let mods = if extra_mods.trim().is_empty() { format!("@{id}") } else { format!("{};@{id}", extra_mods.trim().trim_end_matches(';')) };
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
        .arg(format!("-mod=@{id}"))
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
    fn spawn_points_contain_the_position() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s.xml");
        write_spawn_points(&p, [123.5, 456.5]).unwrap();
        let t = std::fs::read_to_string(p).unwrap();
        assert_eq!(t.matches("<pos x=\"123.5\" z=\"456.5\" />").count(), 3);
    }
    #[test]
    fn missing_tools_are_reported() {
        let p = ToolPaths { work_drive: "Z:/nope".into(), dayz_tools: "Z:/x".into(), dayz: "Z:/y".into() };
        assert_eq!(p.check().len(), 3);
    }
    #[test]
    fn server_config_uses_the_map_mission() {
        let d = tempfile::tempdir().unwrap();
        let t = std::fs::read_to_string(write_server_config(d.path(), "Foo").unwrap()).unwrap();
        assert!(t.contains("template = \"dayzOffline.Foo\";"));
    }
}

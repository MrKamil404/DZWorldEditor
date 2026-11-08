//! Sprawdza nowe wydania na GitHubie i podmienia działający plik programu.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use eframe::egui;
use serde::Deserialize;

use crate::i18n::{t, tf};

const REPOSITORY: &str = "MrKamil404/DZWorldEditor";
const USER_AGENT: &str = concat!("DZWorldEditor-updater/", env!("CARGO_PKG_VERSION"));
/// Ustawienie tej zmiennej wyłącza automatyczne sprawdzanie przy starcie.
const SKIP_CHECK_ENV: &str = "DZWORLDEDITOR_NO_UPDATE_CHECK";
const API_ENV: &str = "DZWORLDEDITOR_UPDATE_API";
const GUI_ASSET_PREFIX: &str = "dzworldeditor";

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Deserialize, Clone, Debug)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
}

/// Liczbowe części wersji, np. `v2.4.0` albo `2.5.0-beta`.
fn parse_version(text: &str) -> Option<Vec<u64>> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    let core = text.split(['-', '+']).next()?;
    core.split('.').map(|p| p.parse().ok()).collect()
}

pub fn is_newer(tag: &str, current: &str) -> bool {
    let (Some(mut a), Some(mut b)) = (parse_version(tag), parse_version(current)) else {
        return false;
    };
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

fn agent() -> Result<ureq::Agent> {
    let tls = native_tls::TlsConnector::new()
        .context("Nie udało się przygotować połączenia HTTPS")?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .user_agent(USER_AGENT)
        .build())
}

/// Adres listy wydań. Zmienna środowiskowa pozwala użyć własnego lub testowego serwera.
fn releases_url() -> String {
    std::env::var(API_ENV).unwrap_or_else(|_| format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"))
}

/// Najnowsze opublikowane wydanie (bez szkiców i wersji wstępnych).
fn latest() -> Result<Release> {
    latest_from(&releases_url())
}

fn latest_from(url: &str) -> Result<Release> {
    agent()?
        .get(url)
        .set("Accept", "application/vnd.github+json")
        .call()
        .context("Nie udało się połączyć z serwerem wydań")?
        .into_json()
        .context("Nieprawidłowa odpowiedź serwera wydań")
}

/// Plik do pobrania: nazwany jak działający program, program GUI albo jedyny `.exe`.
/// Wydanie zawiera też `dzwe-cli.exe`, więc samo „jedyny exe” by nie wystarczyło.
fn asset<'a>(release: &'a Release, exe_name: &str) -> Option<&'a Asset> {
    let exes: Vec<_> = release
        .assets
        .iter()
        .filter(|a| a.name.to_ascii_lowercase().ends_with(".exe"))
        .collect();
    exes.iter()
        .find(|a| a.name.eq_ignore_ascii_case(exe_name))
        .or_else(|| {
            exes.iter()
                .find(|a| a.name.to_ascii_lowercase().starts_with(GUI_ASSET_PREFIX))
        })
        .or(if exes.len() == 1 { exes.first() } else { None })
        .copied()
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

fn previous(exe: &Path) -> PathBuf {
    sibling(exe, ".old")
}

/// Usuwa pliki zostawione przez wcześniejszą aktualizację.
fn cleanup(exe: &Path) {
    let _ = std::fs::remove_file(previous(exe));
    let _ = std::fs::remove_file(sibling(exe, ".new"));
}

/// Pobiera plik obok `exe` i podmienia program. Działający proces korzysta dalej
/// ze starego pliku (przemianowanego na `.old`) do czasu ponownego uruchomienia.
fn install(asset: &Asset, exe: &Path, progress: &AtomicU64) -> Result<()> {
    let new = sibling(exe, ".new");
    let response = agent()?
        .get(&asset.browser_download_url)
        .call()
        .context("Nie udało się pobrać nowej wersji")?;
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(&new)
        .with_context(|| format!("Nie można zapisać pliku {}", new.display()))?;
    let mut buf = vec![0; 64 * 1024];
    let mut total = 0u64;
    let mut head = Vec::new();
    loop {
        let n = reader
            .read(&mut buf)
            .context("Przerwano pobieranie nowej wersji")?;
        if n == 0 {
            break;
        }
        if head.len() < 2 {
            head.extend_from_slice(&buf[..n.min(2)]);
        }
        file.write_all(&buf[..n])?;
        total += n as u64;
        progress.store(total, Ordering::Relaxed);
    }
    file.sync_all()?;
    drop(file);
    let check = || -> Result<()> {
        ensure!(
            total == asset.size,
            "Pobrano {} z {} bajtów nowej wersji",
            total,
            asset.size
        );
        ensure!(head.starts_with(b"MZ"), "Pobrany plik nie jest programem Windows");
        Ok(())
    };
    if let Err(e) = check() {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    replace(exe, &new)
}

/// Windows pozwala zmienić nazwę działającego pliku, więc nie trzeba procesu pomocniczego.
fn replace(exe: &Path, new: &Path) -> Result<()> {
    let old = previous(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old)
        .with_context(|| format!("Nie można zastąpić pliku {}", exe.display()))?;
    if let Err(e) = std::fs::rename(new, exe) {
        let _ = std::fs::rename(&old, exe);
        bail!("Nie można zastąpić pliku {}: {e}", exe.display());
    }
    Ok(())
}

enum State {
    Idle,
    Checking(Receiver<Result<Release>>),
    UpToDate,
    Failed(String),
    Available(Release),
    Downloading(Release, u64, Arc<AtomicU64>, Receiver<Result<()>>),
    Installed(Release),
}

/// Sprawdzanie wydań, okno z opisem zmian i aktualizacja w miejscu.
pub struct Updater {
    state: State,
    /// Ostatnie sprawdzenie uruchomił przycisk, więc jego wynik jest pokazywany.
    manual: bool,
    window: bool,
    error: Option<String>,
    restart: bool,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            state: State::Idle,
            manual: false,
            window: false,
            error: None,
            restart: false,
        }
    }
}

fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

fn exe_name() -> String {
    current_exe()
        .and_then(|e| e.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

impl Updater {
    /// Porządki po poprzedniej aktualizacji i automatyczne sprawdzenie przy starcie.
    pub fn startup(&mut self) {
        if let Some(exe) = current_exe() {
            cleanup(&exe);
        }
        if std::env::var_os(SKIP_CHECK_ENV).is_none() {
            self.check(false);
        }
    }

    fn check(&mut self, manual: bool) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(latest());
        });
        self.manual = manual;
        self.state = State::Checking(rx);
    }

    fn found(&mut self, release: Release) {
        self.state = if is_newer(&release.tag_name, version()) {
            // Nowe wydanie otwiera opis raz; później wraca do niego przycisk na pasku.
            self.window = true;
            State::Available(release)
        } else {
            State::UpToDate
        };
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        match &self.state {
            State::Checking(rx) => match rx.try_recv() {
                Ok(Ok(release)) => self.found(release),
                Ok(Err(e)) => self.state = State::Failed(format!("{e:#}")),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.state = State::Failed("Przerwano sprawdzanie aktualizacji".into())
                }
            },
            State::Downloading(release, _, _, rx) => match rx.try_recv() {
                Ok(Ok(())) => self.state = State::Installed(release.clone()),
                Ok(Err(e)) => {
                    self.error = Some(format!("{e:#}"));
                    self.state = State::Available(release.clone());
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.state = State::Available(release.clone())
                }
            },
            _ => {}
        }
        if matches!(self.state, State::Checking(_) | State::Downloading(..)) {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn install(&mut self) {
        let State::Available(release) = &self.state else {
            return;
        };
        let Some(exe) = current_exe() else {
            self.error = Some("Nie można ustalić ścieżki programu".into());
            return;
        };
        let Some(asset) = asset(release, &exe_name()).cloned() else {
            return;
        };
        let progress = Arc::new(AtomicU64::new(0));
        let (tx, rx) = channel();
        let shared = progress.clone();
        let size = asset.size;
        std::thread::spawn(move || {
            let _ = tx.send(install(&asset, &exe, &shared));
        });
        self.error = None;
        self.state = State::Downloading(release.clone(), size, progress, rx);
    }

    /// Pozycja na pasku stanu (układ od prawej do lewej).
    pub fn bar(&mut self, ui: &mut egui::Ui) {
        let mut check = false;
        match &self.state {
            State::Checking(_) => {
                ui.spinner();
                if self.manual {
                    ui.label(t("Checking for updates..."));
                }
            }
            State::Available(release) => {
                let text = egui::RichText::new(tf("New version {0}", &[&release.tag_name]))
                    .color(egui::Color32::BLACK);
                if ui
                    .add(egui::Button::new(text).fill(egui::Color32::from_rgb(232, 196, 92)))
                    .clicked()
                {
                    self.window = true;
                }
            }
            State::Downloading(release, size, progress, _) => {
                let done = progress.load(Ordering::Relaxed) as f32 / (*size).max(1) as f32;
                let bar = egui::ProgressBar::new(done.min(1.0))
                    .desired_width(160.0)
                    .text(tf("Downloading {0}", &[&release.tag_name]));
                if ui.add(bar).clicked() {
                    self.window = true;
                }
            }
            State::Installed(release) => {
                let text = egui::RichText::new(tf("Restart to use {0}",
                    &[&release.tag_name],
                ))
                .color(egui::Color32::BLACK);
                if ui
                    .add(egui::Button::new(text).fill(egui::Color32::from_rgb(120, 200, 140)))
                    .clicked()
                {
                    self.window = true;
                }
            }
            State::Idle | State::UpToDate | State::Failed(_) => {
                let mut button = ui.small_button(t("Check for updates"));
                match &self.state {
                    State::Failed(e) => {
                        button = button.on_hover_text(e.as_str());
                        if self.manual {
                            ui.colored_label(
                                egui::Color32::from_rgb(230, 120, 110),
                                t("Could not check for updates"),
                            )
                            .on_hover_text(e.as_str());
                        }
                    }
                    State::UpToDate if self.manual => {
                        ui.weak(t("You have the latest version"));
                    }
                    _ => {}
                }
                check = button.clicked();
            }
        }
        if check {
            self.check(true);
        }
    }

    pub fn window(&mut self, ctx: &egui::Context) {
        let release = match &self.state {
            State::Available(r) | State::Downloading(r, ..) | State::Installed(r) => r.clone(),
            _ => return,
        };
        if !self.window {
            return;
        }
        let mut open = true;
        let mut start_install = false;
        let mut later = false;
        let mut restart = false;
        let download = asset(&release, &exe_name()).cloned();
        egui::Window::new(t("Program update"))
            .id(egui::Id::new("update-window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                ui.heading(tf("Version {0} is available", &[&release.tag_name]));
                ui.label(tf("Your version: {0}", &[version()]));
                if let Some(name) = release
                    .name
                    .as_deref()
                    .filter(|n| !n.trim().is_empty() && *n != release.tag_name)
                {
                    ui.strong(name);
                }
                ui.separator();
                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    let notes = release
                        .body
                        .as_deref()
                        .unwrap_or("")
                        .replace("**", "")
                        .replace('\r', "");
                    if notes.trim().is_empty() {
                        ui.weak(t("No release notes"));
                    } else {
                        ui.label(notes);
                    }
                });
                ui.separator();
                if let Some(e) = &self.error {
                    ui.colored_label(egui::Color32::from_rgb(230, 120, 110), e.as_str());
                }
                ui.horizontal(|ui| match &self.state {
                    State::Available(_) => {
                        if let Some(download) = &download {
                            let size = format!("{:.1}", download.size as f64 / 1_048_576.0);
                            if ui
                                .button(tf("Download and install ({0} MB)", &[&size]))
                                .on_hover_text(t("The download replaces the current file; the previous one stays as .old until the next start.",
                                ))
                                .clicked()
                            {
                                start_install = true;
                            }
                        } else {
                            ui.weak(t("The release has no .exe to download"));
                        }
                        if ui.button(t("Release page")).clicked() {
                            ctx.open_url(egui::OpenUrl::new_tab(&release.html_url));
                        }
                        if ui.button(t("Later")).clicked() {
                            later = true;
                        }
                    }
                    State::Downloading(_, size, progress, _) => {
                        let done = progress.load(Ordering::Relaxed);
                        ui.add(
                            egui::ProgressBar::new((done as f32 / (*size).max(1) as f32).min(1.0))
                                .show_percentage()
                                .desired_width(300.0),
                        );
                        ui.label(format!(
                            "{:.1} / {:.1} MB",
                            done as f64 / 1_048_576.0,
                            *size as f64 / 1_048_576.0
                        ));
                    }
                    State::Installed(_) => {
                        ui.label(t("Installed. The new version starts after a restart.",
                        ));
                        if ui.button(t("Restart")).clicked() {
                            restart = true;
                        }
                    }
                    _ => {}
                });
            });
        if !open || later {
            self.window = false;
        }
        if start_install {
            self.install();
        }
        if restart {
            self.restart = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Uruchamia nowy program, gdy okno faktycznie się zamknęło.
    pub fn on_exit(&self) {
        if self.restart && matches!(self.state, State::Installed(_)) {
            if let Some(exe) = current_exe() {
                let _ = std::process::Command::new(exe).spawn();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal release server: one JSON document and one file.
    fn serve(requests: usize, json: String, file: Vec<u8>) -> String {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                }
                let body = if line.contains("/latest") { json.clone().into_bytes() } else { file.clone() };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        base
    }

    #[test]
    fn checks_downloads_and_replaces_the_program() {
        let new_exe = b"MZ new version".to_vec();
        let listener_probe = serve(1, String::new(), new_exe.clone());
        let json = format!(
            r#"{{"tag_name":"v9.9.9","html_url":"x","body":"notes","assets":[{{"name":"DZWorldEditor.exe","size":{},"browser_download_url":"{listener_probe}/DZWorldEditor.exe"}}]}}"#,
            new_exe.len()
        );
        let api = serve(1, json, Vec::new());
        let release = latest_from(&format!("{api}/releases/latest")).unwrap();
        assert!(is_newer(&release.tag_name, version()));
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("DZWorldEditor.exe");
        std::fs::write(&exe, b"MZ old").unwrap();
        let progress = AtomicU64::new(0);
        install(asset(&release, "DZWorldEditor.exe").unwrap(), &exe, &progress).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), new_exe);
        assert_eq!(std::fs::read(previous(&exe)).unwrap(), b"MZ old");
        assert_eq!(progress.load(Ordering::Relaxed), new_exe.len() as u64);
        cleanup(&exe);
        assert!(!previous(&exe).exists());
    }

    #[test]
    fn broken_download_leaves_the_program_alone() {
        let base = serve(1, String::new(), b"<html>not a program".to_vec());
        let a = Asset { name: "x.exe".into(), size: 19, browser_download_url: format!("{base}/x.exe") };
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("x.exe");
        std::fs::write(&exe, b"MZ old").unwrap();
        assert!(install(&a, &exe, &AtomicU64::new(0)).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZ old");
        assert!(!sibling(&exe, ".new").exists());
    }

    fn release(names: &[&str]) -> Release {
        Release {
            tag_name: "2.5.0".into(),
            name: None,
            body: None,
            html_url: String::new(),
            assets: names
                .iter()
                .map(|n| Asset {
                    name: (*n).into(),
                    size: 1,
                    browser_download_url: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("2.10.0", "2.9.3"));
        assert!(is_newer("v2.5", "2.4.9"));
        assert!(is_newer("3", "2.99.99"));
        assert!(!is_newer("2.4.0", "2.4.0"));
        assert!(!is_newer("2.4", "2.4.0"));
        assert!(!is_newer("2.3.9", "2.4.0"));
        assert!(!is_newer("2.5.0-beta", "2.5.0"));
        assert!(!is_newer("Release", "0.1.0"), "obecny tag repozytorium nie jest wersją");
    }

    #[test]
    fn picks_the_gui_executable() {
        let r = release(&["notes.md", "dzworldeditor-x64.exe", "dzwe-cli.exe"]);
        assert_eq!(asset(&r, "DZWorldEditor.exe").unwrap().name, "dzworldeditor-x64.exe");
        assert_eq!(asset(&r, "przemianowany.exe").unwrap().name, "dzworldeditor-x64.exe");
        let r = release(&["notes.md", "studio.exe"]);
        assert_eq!(asset(&r, "renamed.exe").unwrap().name, "studio.exe");
        assert!(asset(&release(&["a.exe", "b.exe"]), "renamed.exe").is_none());
        assert!(asset(&release(&["DayZForestBuilder.zip"]), "DZWorldEditor.exe").is_none());
    }

    #[test]
    fn replacing_keeps_the_previous_executable_and_cleanup_removes_it() {
        let dir = std::env::temp_dir().join(format!("forest-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("app.exe");
        std::fs::write(&exe, b"MZold").unwrap();
        let new = sibling(&exe, ".new");
        std::fs::write(&new, b"MZnew").unwrap();
        replace(&exe, &new).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"MZnew");
        assert_eq!(std::fs::read(previous(&exe)).unwrap(), b"MZold");
        assert!(!new.exists());
        cleanup(&exe);
        assert!(!previous(&exe).exists());
        assert!(exe.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Serwuje `body` jednemu klientowi przez HTTP i zwraca adres pliku.
    fn serve_once(body: Vec<u8>) -> String {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/DZWorldEditor.exe", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });
        url
    }

    fn install_dir(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("forest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("DZWorldEditor.exe");
        std::fs::write(&exe, b"MZold").unwrap();
        (dir, exe)
    }

    #[test]
    fn download_replaces_the_program_and_reports_progress() {
        let (dir, exe) = install_dir("install");
        let body = b"MZnew-program".to_vec();
        let asset = Asset {
            name: "DZWorldEditor.exe".into(),
            size: body.len() as u64,
            browser_download_url: serve_once(body.clone()),
        };
        let progress = AtomicU64::new(0);
        install(&asset, &exe, &progress).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), body);
        assert_eq!(std::fs::read(previous(&exe)).unwrap(), b"MZold");
        assert_eq!(progress.load(Ordering::Relaxed), asset.size);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn broken_downloads_leave_the_program_untouched() {
        for (name, body, size) in [
            ("not-exe", b"<html>nope</html>".to_vec(), 17u64),
            ("truncated", b"MZshort".to_vec(), 999u64),
        ] {
            let (dir, exe) = install_dir(name);
            let asset = Asset {
                name: "DZWorldEditor.exe".into(),
                size,
                browser_download_url: serve_once(body),
            };
            assert!(install(&asset, &exe, &AtomicU64::new(0)).is_err(), "{name}");
            assert_eq!(std::fs::read(&exe).unwrap(), b"MZold", "{name}");
            assert!(!sibling(&exe, ".new").exists(), "{name}");
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// Łączy się z GitHubem: `cargo test -p DZWorldEditor -- --ignored latest_release`.
    #[test]
    #[ignore]
    fn latest_release_parses() {
        let release = latest().unwrap();
        println!("{} {:?}", release.tag_name, release.assets);
        assert!(!release.html_url.is_empty());
    }

    #[test]
    fn found_release_opens_the_window_only_when_newer() {
        let mut updater = Updater::default();
        let mut r = release(&["DZWorldEditor.exe"]);
        r.tag_name = "v0.0.1".into();
        updater.found(r.clone());
        assert!(matches!(updater.state, State::UpToDate));
        assert!(!updater.window);
        r.tag_name = "v999.0.0".into();
        updater.found(r);
        assert!(matches!(updater.state, State::Available(_)));
        assert!(updater.window);
    }
}

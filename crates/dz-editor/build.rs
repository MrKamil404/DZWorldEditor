use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn target_folder(arch: &str) -> &'static str {
    match arch {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        "arm" => "arm",
        other => panic!("Unsupported Windows icon target architecture: {other}"),
    }
}

fn resource_machine(arch: &str) -> &'static str {
    match arch {
        "x86_64" => "X64",
        "x86" => "X86",
        "aarch64" => "ARM64",
        "arm" => "ARM",
        other => panic!("Unsupported Windows icon target architecture: {other}"),
    }
}

fn path_tool(name: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?).map(|p| p.join(name)).find(|p| p.is_file())
}

fn sdk_rc(target: &str) -> Option<PathBuf> {
    if let Some(root) = env::var_os("WindowsSdkDir") {
        let root = PathBuf::from(root).join("bin");
        if let Some(version) = env::var_os("WindowsSDKVersion") {
            let version = version.to_string_lossy();
            let version = version.trim_matches(|c| c == '\\' || c == '/');
            let candidate = root.join(version).join(target).join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let mut roots = Vec::new();
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(root) = env::var_os(key) {
            roots.push(PathBuf::from(root).join("Windows Kits").join("10").join("bin"));
        }
    }

    for root in roots {
        let mut versions: Vec<_> = fs::read_dir(root)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        versions.sort();
        for version in versions.into_iter().rev() {
            let candidate = version.join(target).join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn msvc_cvtres(target: &str) -> Option<PathBuf> {
    if let Some(root) = env::var_os("VCToolsInstallDir") {
        let host_arch = env::var("HOST").unwrap_or_default();
        let preferred_host = if host_arch.starts_with("aarch64") {
            "Hostarm64"
        } else if host_arch.starts_with("i686") || host_arch.starts_with("i586") {
            "Hostx86"
        } else {
            "Hostx64"
        };
        for host in [preferred_host, "Hostx64", "Hostx86", "Hostarm64"] {
            let candidate = PathBuf::from(&root)
                .join("bin")
                .join(host)
                .join(target)
                .join("cvtres.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(root) = env::var_os(key) else { continue };
        let versions_root = PathBuf::from(root)
            .join("Microsoft Visual Studio")
            .join("2022");
        let Ok(editions) = fs::read_dir(versions_root) else { continue };
        for edition in editions.filter_map(Result::ok) {
            let tools = edition.path().join("VC").join("Tools").join("MSVC");
            let Ok(versions) = fs::read_dir(tools) else { continue };
            for version in versions.filter_map(Result::ok) {
                for host in ["Hostx64", "Hostx86", "Hostarm64"] {
                    let candidate = version
                        .path()
                        .join("bin")
                        .join(host)
                        .join(target)
                        .join("cvtres.exe");
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    None
}

fn compile_windows_icon() {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("target architecture is set");
    let folder = target_folder(&arch);
    let machine = resource_machine(&arch);
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir is set"));
    let icon = manifest_dir.join("../../assets/app-icon.ico");
    let icon = icon.canonicalize().expect("application ICO asset must exist");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    let rc_file = out_dir.join("app-icon.rc");
    let res_file = out_dir.join("app-icon.res");
    let obj_file = out_dir.join("app-icon.obj");
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    fs::write(&rc_file, format!("1 ICON \"{icon_path}\"\n"))
        .expect("write Windows icon resource file");

    let rc = sdk_rc(folder).or_else(|| path_tool("rc.exe")).unwrap_or_else(|| {
        panic!("Windows SDK rc.exe was not found; install the Windows 10/11 SDK to embed the application icon")
    });
    let cvtres = msvc_cvtres(folder).or_else(|| path_tool("cvtres.exe")).unwrap_or_else(|| {
        panic!("MSVC cvtres.exe was not found; install the Visual Studio C++ build tools to embed the application icon")
    });

    let status = Command::new(&rc)
        .arg("/nologo")
        .arg(format!("/fo{}", res_file.display()))
        .arg(&rc_file)
        .status()
        .expect("run Windows resource compiler");
    assert!(status.success(), "Windows resource compiler failed");

    let status = Command::new(&cvtres)
        .arg("/nologo")
        .arg(format!("/MACHINE:{machine}"))
        .arg(format!("/OUT:{}", obj_file.display()))
        .arg(&res_file)
        .status()
        .expect("run MSVC resource converter");
    assert!(status.success(), "MSVC resource converter failed");

    println!("cargo:rustc-link-arg={}", obj_file.display());
}

fn main() {
    println!("cargo:rerun-if-changed=../../assets/app-icon.ico");
    println!("cargo:rerun-if-env-changed=WindowsSdkDir");
    println!("cargo:rerun-if-env-changed=WindowsSDKVersion");
    println!("cargo:rerun-if-env-changed=VCToolsInstallDir");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        compile_windows_icon();
    } else {
        println!("cargo:warning=The Windows executable icon resource is embedded only for MSVC targets");
    }
}

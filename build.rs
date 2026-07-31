use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/lane-pilot.rc");
    println!("cargo:rerun-if-changed=assets/lane-pilot.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let resource_compiler =
        find_resource_compiler().expect("Windows SDK resource compiler (rc.exe) was not found");
    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set")).join("lane-pilot.res");

    let status = Command::new(resource_compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&output)
        .arg("assets/lane-pilot.rc")
        .status()
        .expect("failed to start rc.exe");
    assert!(status.success(), "rc.exe failed with status {status}");

    println!("cargo:rustc-link-arg-bin=lane-pilot={}", output.display());
}

fn find_resource_compiler() -> Option<PathBuf> {
    if let Some(path) = find_on_path("rc.exe") {
        return Some(path);
    }

    let program_files = env::var_os("ProgramFiles(x86)")?;
    let kits_bin = PathBuf::from(program_files)
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let mut versions = std::fs::read_dir(kits_bin)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    versions
        .into_iter()
        .map(|version| version.join("x64").join("rc.exe"))
        .find(|path| path.is_file())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| Path::new(path).is_file())
}

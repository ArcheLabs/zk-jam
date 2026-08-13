use std::{env, fs, path::PathBuf};

#[derive(serde::Deserialize)]
struct JambdaManifest {
    repository: String,
    revision: String,
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"))
        .join("../../integration/jambda-m3.json");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let value: JambdaManifest = serde_json::from_slice(
        &fs::read(&manifest).unwrap_or_else(|error| panic!("read {}: {error}", manifest.display())),
    )
    .unwrap_or_else(|error| panic!("parse {}: {error}", manifest.display()));
    assert_eq!(
        value.repository, "ArcheLabs/jambda",
        "unsupported Jambda repository"
    );
    assert!(
        is_sha(&value.revision),
        "Jambda revision must be a 40-character SHA"
    );
    println!(
        "cargo:rustc-env=ZK_JAM_JAMBDA_REPOSITORY={}",
        value.repository
    );
    println!("cargo:rustc-env=ZK_JAM_JAMBDA_REVISION={}", value.revision);
}

fn is_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

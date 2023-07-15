use failure::Error;
use regex::Regex;
use std::{
    env,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const DEPLOYMENT_TARGET_VAR: &str = "MACOSX_DEPLOYMENT_TARGET";

fn out_dir() -> PathBuf {
    std::env::var("OUT_DIR").expect("OUT_DIR environment var not set.").into()
}

#[cfg(not(feature = "bundled"))]
mod webrtc {
    use super::*;
    use failure::bail;

    const LIB_NAME: &str = "webrtc-audio-processing";

    pub(super) fn get_build_paths() -> Result<(PathBuf, PathBuf), Error> {
        let (pkgconfig_include_path, pkgconfig_lib_path) = find_pkgconfig_paths()?;

        let include_path = std::env::var("WEBRTC_AUDIO_PROCESSING_INCLUDE")
            .ok()
            .map(|x| x.into())
            .or(pkgconfig_include_path);
        let lib_path = std::env::var("WEBRTC_AUDIO_PROCESSING_LIB")
            .ok()
            .map(|x| x.into())
            .or(pkgconfig_lib_path);

        println!("{:?}, {:?}", include_path, lib_path);

        match (include_path, lib_path) {
            (Some(include_path), Some(lib_path)) => Ok((include_path, lib_path)),
            _ => {
                eprintln!("Couldn't find either header or lib files for {}.", LIB_NAME);
                eprintln!("See the crate README for installation instructions, or use the 'bundled' feature to statically compile.");
                bail!("Aborting compilation due to linker failure.");
            },
        }
    }

    pub(super) fn build_if_necessary() -> Result<(), Error> {
        Ok(())
    }

    fn find_pkgconfig_paths() -> Result<(Option<PathBuf>, Option<PathBuf>), Error> {
        Ok(pkg_config::Config::new()
            .probe(LIB_NAME)
            .and_then(|mut lib| Ok((lib.include_paths.pop(), lib.link_paths.pop())))?)
    }
}

#[cfg(feature = "bundled")]
mod webrtc {
    use super::*;
    use failure::bail;

    const BUNDLED_SOURCE_PATH: &str = "webrtc-audio-processing";

    pub(super) fn get_build_paths() -> Result<(PathBuf, PathBuf), Error> {
        let include_path = out_dir().join(BUNDLED_SOURCE_PATH);
        let lib_path = out_dir().join("lib");
        Ok((include_path, lib_path))
    }

    fn get_target() -> String {
        let target = std::env::var("TARGET").unwrap();
        let mut target_tuple = target.split("-").collect::<Vec<_>>();

        if target_tuple.len() <= 3 {
            // Remove the vendor compo  nent.
            target_tuple.remove(1);
        }

        assert_eq!(3, target_tuple.len());
        target_tuple.join("-")
    }

    fn copy_source_to_out_dir() -> Result<PathBuf, Error> {
        use fs_extra::dir::CopyOptions;

        if Path::new(BUNDLED_SOURCE_PATH).read_dir().expect("failed to read_dir").next().is_none() {
            eprintln!("The webrtc-audio-processing source directory is empty.");
            eprintln!("See the crate README for installation instructions.");
            eprintln!("Remember to clone the repo recursively if building from source.");
            bail!("Aborting compilation because bundled source directory is empty.");
        }

        let out_dir = out_dir();
        // let mut options = CopyOptions::new();
        // options.overwrite = true;

        // fs_extra::dir::copy(BUNDLED_SOURCE_PATH, &out_dir, &options).expect("fs_extra::dir::copy");
        let source_in_out = &out_dir.join(BUNDLED_SOURCE_PATH);
        std::fs::create_dir_all(&source_in_out).unwrap();
        cp_r(BUNDLED_SOURCE_PATH, &source_in_out).expect("cp_r");

        Ok(out_dir.join(BUNDLED_SOURCE_PATH))
    }

    pub(super) fn build_if_necessary() -> Result<(), Error> {
        let build_dir = copy_source_to_out_dir().expect("failed copy_source_to_out_dir");

        if cfg!(target_os = "macos") {
            run_command(&build_dir, "glibtoolize", None).expect("failed glibtoolize");
        } else {
            run_command(&build_dir, "libtoolize", None).expect("failed libtoolize");
        }

        run_command(&build_dir, "aclocal", None).expect("failed aclocal");
        run_command(&build_dir, "automake", Some(&["--add-missing", "--copy"]))
            .expect("failed automake");
        run_command(&build_dir, "autoconf", None).expect("failed autoconf");

        autotools::Config::new(build_dir)
            .cflag("-fPIC")
            .cxxflag("-fPIC")
            .disable_shared()
            .enable_static()
            .host(&get_target())
            .build();

        Ok(())
    }

    fn run_command<P: AsRef<Path>>(
        curr_dir: P,
        cmd: &str,
        args_opt: Option<&[&str]>,
    ) -> Result<(), Error> {
        let mut command = std::process::Command::new(cmd);

        command.current_dir(curr_dir);

        if let Some(args) = args_opt {
            command.args(args);
        }

        let _output = command.output().map_err(|e| {
            failure::format_err!(
                "Error running command '{}' with args '{:?}' - {:?}",
                cmd,
                args_opt,
                e
            )
        })?;

        Ok(())
    }
}

// TODO: Consider fixing this with the upstream.
// https://github.com/rust-lang/rust-bindgen/issues/1089
// https://github.com/rust-lang/rust-bindgen/issues/1301
fn derive_serde(binding_file: &Path) -> Result<(), Error> {
    let mut contents = String::new();
    File::open(binding_file)?.read_to_string(&mut contents)?;

    let new_contents = format!(
        "use serde::{{Serialize, Deserialize}};\n{}",
        Regex::new(r"#\s*\[\s*derive\s*\((?P<d>[^)]+)\)\s*\]\s*pub\s*(?P<s>struct|enum)")?
            .replace_all(&contents, "#[derive($d, Serialize, Deserialize)] pub $s")
    );

    File::create(&binding_file)?.write_all(new_contents.as_bytes())?;

    Ok(())
}

fn main() -> Result<(), Error> {
    // Ref: https://github.com/rust-lang/git2-rs/blob/master/libgit2-sys/build.rs#L27
    if !Path::new("webrtc-audio-processing/src").exists() {
        println!("Installing from git...");
        let _ = std::process::Command::new("git")
            .args(&["submodule", "update", "--init", "webrtc-audio-processing"])
            .status();
    }

    webrtc::build_if_necessary().expect("failed to build if necessary");
    let (webrtc_include, webrtc_lib) =
        webrtc::get_build_paths().expect("failed to get build paths");

    let mut cc_build = cc::Build::new();

    // set mac minimum version
    if cfg!(target_os = "macos") {
        let min_version = match env::var(DEPLOYMENT_TARGET_VAR) {
            Ok(ver) => ver,
            Err(_) => {
                String::from(match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
                    "x86_64" => "10.10", // Using what I found here https://github.com/webrtc-uwp/chromium-build/blob/master/config/mac/mac_sdk.gni#L17
                    "aarch64" => "11.0", // Apple silicon started here.
                    arch => panic!("unknown arch: {}", arch),
                })
            },
        };

        // `cc` doesn't try to pick up on this automatically, but `clang` needs it to
        // generate a "correct" Objective-C symbol table which better matches XCode.
        // See https://github.com/h4llow3En/mac-notification-sys/issues/45.
        cc_build.flag(&format!("-mmacos-version-min={}", min_version));
    }

    // do not set on macos
    if !cfg!(target_os = "macos") {
        cc_build.flag("-std=c++11");
    }
    // use libstdc++ for older macs
    // match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
    //     "x86_64" => {
    //         cc_build.cpp_link_stdlib("stdc++");
    //     },
    //     // on apple silicon it fails
    //     _ => {},
    // };

    cc_build
        .cpp(true)
        .file("src/wrapper.cpp")
        .include(&webrtc_include)
        .flag("-Wno-unused-parameter")
        .flag("-Wno-deprecated-declarations")
        .out_dir(&out_dir())
        .compile("webrtc_audio_processing_wrapper");

    println!("cargo:rustc-link-search=native={}", webrtc_lib.display());
    println!("cargo:rustc-link-lib=static=webrtc_audio_processing_wrapper");

    println!("cargo:rerun-if-env-changed={}", DEPLOYMENT_TARGET_VAR);

    if cfg!(feature = "bundled") {
        println!("cargo:rustc-link-lib=static=webrtc_audio_processing");
    } else {
        println!("cargo:rustc-link-lib=dylib=webrtc_audio_processing");
    }

    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }

    let binding_file = out_dir().join("bindings.rs");
    bindgen::Builder::default()
        .header("src/wrapper.hpp")
        .generate_comments(true)
        .rustified_enum(".*")
        .derive_debug(true)
        .derive_default(true)
        .derive_partialeq(true)
        .clang_arg(&format!("-I{}", &webrtc_include.display()))
        .disable_name_namespacing()
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(&binding_file)
        .expect("Couldn't write bindings!");

    if cfg!(feature = "derive_serde") {
        derive_serde(&binding_file).expect("Failed to modify derive macros");
    }

    rerun_if(Path::new("webrtc-audio-processing/webrtc"));
    rerun_if(Path::new("webrtc-audio-processing/configure.ac"));

    Ok(())
}

fn rerun_if(path: &Path) {
    if path.is_dir() {
        for entry in std::fs::read_dir(path).expect("read_dir") {
            rerun_if(&entry.expect("entry").path());
        }
    } else {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn cp_r(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<(), Error> {
    let profile = std::env::var("PROFILE").unwrap();
    let profile = profile.as_str();

    for e in from.as_ref().read_dir().unwrap() {
        let e = e.unwrap();
        let from = e.path();
        let to = to.as_ref().join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&to)?;
            cp_r(&from, &to)?;
        } else {
            if profile == "debug" && from.clone().to_str().unwrap().contains(".git/") {
                continue;
            }
            println!("{} => {}", from.display(), to.display());
            std::fs::copy(&from, &to)?;
        }
    }

    Ok(())
}

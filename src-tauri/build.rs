fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
            println!("cargo:rerun-if-changed=native/local_copilot.swift");
            println!("cargo:rerun-if-changed=../scripts/bundle-local-runtime.py");
            assert!(
                std::process::Command::new("python3")
                    .arg("../scripts/bundle-local-runtime.py")
                    .status()
                    .expect("Python required for bundling")
                    .success(),
                "local runtime bundling failed"
            );
            let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap())
                .join("local-copilot");
            let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
            let arch = if arch == "aarch64" { "arm64" } else { "x86_64" };
            let status = std::process::Command::new("xcrun")
                .args([
                    "swiftc",
                    "-parse-as-library",
                    "-O",
                    "-target",
                    &format!("{arch}-apple-macos26.0"),
                    "native/local_copilot.swift",
                    "-o",
                ])
                .arg(output)
                .status()
                .expect("Swift compiler required");
            assert!(status.success(), "local copilot compilation failed");
        }
        cc::Build::new()
            .file("native/mac_audio.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .flag("-mmacosx-version-min=14.2")
            .compile("unmute_mac_audio");
        for framework in ["Foundation", "AVFoundation", "CoreAudio"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        println!("cargo:rerun-if-changed=native/mac_audio.m");
        // CLI carries its own permission purpose strings without a WebView/app lifecycle.
        let plist = std::env::current_dir()
            .unwrap()
            .join("AudioDiag-Info.plist");
        println!("cargo:rerun-if-changed=AudioDiag-Info.plist");
        println!(
            "cargo:rustc-link-arg-bin=audio-diag=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
    #[cfg(feature = "desktop")]
    tauri_build::build();
}

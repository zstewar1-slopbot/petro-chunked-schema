//! Build script for `petro-chunked-schema-editor`.
//!
//! On immutable Linux distributions (such as Fedora Silverblue / Bazzite) where runtime shared
//! libraries (`/usr/lib64/lib*.so.N`) are present on the host without their unversioned `-devel`
//! symlinks (`/usr/lib64/lib*.so`), creates fallback symlinks in `OUT_DIR` so `skia-bindings` and
//! `freya` link cleanly on the host without requiring container wrappers.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(target_os = "linux")]
    {
        if let Ok(out_dir) = std::env::var("OUT_DIR") {
            let out_path = std::path::Path::new(&out_dir);
            let libs = [
                ("libfontconfig.so", "/usr/lib64/libfontconfig.so.1"),
                ("libfreetype.so", "/usr/lib64/libfreetype.so.6"),
                ("libEGL.so", "/usr/lib64/libEGL.so.1"),
                ("libGL.so", "/usr/lib64/libGL.so.1"),
                ("libwayland-egl.so", "/usr/lib64/libwayland-egl.so.1"),
                ("libGLESv2.so", "/usr/lib64/libGLESv2.so.2"),
            ];
            let mut linked_any = false;
            for (link_name, target) in libs {
                let target_path = std::path::Path::new(target);
                let unversioned = std::path::Path::new("/usr/lib64").join(link_name);
                if !unversioned.exists() && target_path.exists() {
                    let dst = out_path.join(link_name);
                    let _ = std::fs::remove_file(&dst);
                    if std::os::unix::fs::symlink(target_path, &dst).is_ok() {
                        linked_any = true;
                    }
                }
            }
            if linked_any {
                println!("cargo:rustc-link-search=native={out_dir}");
            }
        }
    }
}

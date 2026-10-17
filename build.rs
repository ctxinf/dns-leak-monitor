use std::path::Path;

// Fedora ships libpcap.so.1 without the `libpcap.so` dev symlink unless
// libpcap-devel is installed, so `-lpcap` fails at link time. Create the
// symlink in OUT_DIR and add it to the search path. Harmless (unused) when
// the real dev symlink exists.
fn main() {
    let out = std::env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    for dir in ["/usr/lib64", "/usr/lib/x86_64-linux-gnu", "/usr/lib"] {
        if Path::new(dir).join("libpcap.so").exists() {
            return; // devel package present, nothing to do
        }
        let target = Path::new(dir).join("libpcap.so.1");
        if target.exists() {
            let link = Path::new(&out).join("libpcap.so");
            let _ = std::fs::remove_file(&link);
            if std::os::unix::fs::symlink(&target, &link).is_ok() {
                println!("cargo:rustc-link-search=native={out}");
            }
            return;
        }
    }
    println!("cargo:warning=libpcap not found; install libpcap-devel");
}

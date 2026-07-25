#[cfg(target_os = "windows")]
fn main() {
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icons/icon.ico");
    resource.set("ProductName", "Ludo Royale");
    resource.set("FileDescription", "Ludo Royale");
    resource.set("LegalCopyright", "Copyright © 2026 Ludo contributors");

    if let Err(error) = resource.compile() {
        eprintln!("failed to compile Windows application resources: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}

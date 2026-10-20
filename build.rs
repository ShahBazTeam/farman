fn main() {
    println!("cargo:rerun-if-changed=assets/farman.ico");
    println!("cargo:rerun-if-changed=assets/farman-icon.png");
    println!("cargo:rerun-if-changed=assets/farman-logo.png");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/farman.ico");
        resource.set("ProductName", "FARMAN Terminal");
        resource.set("CompanyName", "Shahbaz Team");
        resource.set(
            "FileDescription",
            "FARMAN Terminal - Persian/Arabic capable Windows terminal",
        );
        resource.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        resource.set("FileVersion", env!("CARGO_PKG_VERSION"));
        resource.set(
            "LegalCopyright",
            "Copyright (c) 2026 Shahbaz Team. MIT app code; bundled fonts retain their licenses.",
        );
        resource.set("Comments", "https://shahbazteam.ir");
        resource.set("OriginalFilename", "farman.exe");
        if let Err(err) = resource.compile() {
            panic!("failed to compile Windows resources: {err}");
        }
    }
}

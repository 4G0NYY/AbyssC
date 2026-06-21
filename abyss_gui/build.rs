//! Embed the AbyssC icon into the Windows executable, so Explorer, the taskbar,
//! and shortcuts show it. A no-op on other platforms; non-fatal if the resource
//! compiler is unavailable (the window still loads its icon at runtime).

fn main() {
    println!("cargo:rerun-if-changed=assets/AbyssC.ico");

    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/AbyssC.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=AbyssC icon not embedded into the exe: {e}");
        }
    }
}

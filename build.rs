fn main() {
    // Use pkg-config to locate libmpv installed by Homebrew (or other package managers).
    // This ensures the linker can find libmpv on macOS where /opt/homebrew is nonstandard.
    if let Err(err) = pkg_config::Config::new()
        .atleast_version("0")
        .probe("mpv")
    {
        // Fall back to env vars if pkg-config is missing or can't find mpv.
        // Users can set LIBRARY_PATH / PKG_CONFIG_PATH as needed.
        println!("cargo:warning=Failed to find libmpv via pkg-config: {}", err);
    }
}

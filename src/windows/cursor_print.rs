pub fn run(_: &str) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "Cursor driver is unavailable on Windows",
    ))
}

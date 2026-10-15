pub const SKILL: &str = "Linux virtual desktop automation is unavailable on Windows.";
pub async fn cli(_: &[String]) -> std::io::Result<i32> {
    Err(std::io::Error::other(SKILL))
}

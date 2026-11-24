#[path = "../../src-tauri/src/confluence/convert.rs"]
pub mod converter;

pub fn golden_convert(s: &str) -> String {
    crate::converter::storage_to_markdown(s)
}

//! Image utilities — encode images to base64 data URLs for the Grok vision API.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use std::path::Path;

/// Encode an image file at `path` into a `data:<mime>;base64,...` URL.
pub async fn encode_image_to_data_url(path: &str) -> Result<String, String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read image: {e}"))?;
    let mime = ext_to_mime(path);
    Ok(format!("data:{};base64,{}", mime, B64.encode(&bytes)))
}

/// Encode raw image bytes (e.g. from clipboard paste) into a data URL.
pub fn encode_bytes_to_data_url(bytes: &[u8], mime: &str) -> String {
    format!("data:{};base64,{}", mime, B64.encode(bytes))
}

/// Derive a MIME type from a file extension (best-effort).
fn ext_to_mime(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("tiff" | "tif") => "image/tiff",
        _ => "application/octet-stream",
    }
}

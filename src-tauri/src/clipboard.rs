use arboard::Clipboard;
use base64::prelude::*;
use image::{ImageBuffer, Rgba};

use crate::error::AppError;

pub fn read_image_as_png_base64() -> Result<(String, String), AppError> {
    let mut clipboard = Clipboard::new().map_err(|e| AppError::Config(format!("Zwischenablage nicht verfügbar: {e}")))?;
    let image_data = clipboard
        .get_image()
        .map_err(|e| AppError::NotFound(format!("Kein Bild in der Zwischenablage: {e}")))?;

    let width = image_data.width as u32;
    let height = image_data.height as u32;
    let buffer: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::from_raw(width, height, image_data.bytes.into_owned())
        .ok_or_else(|| AppError::Config("Zwischenablage-Bild hat unerwartetes Format".to_string()))?;

    let mut png_bytes: Vec<u8> = Vec::new();
    buffer
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| AppError::Io(format!("PNG-Kodierung fehlgeschlagen: {e}")))?;

    Ok((BASE64_STANDARD.encode(&png_bytes), "image/png".to_string()))
}

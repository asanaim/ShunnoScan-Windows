use crate::models::{is_sensitive_token, PageOcrResult, WordBox};
use image::{imageops, DynamicImage, ImageBuffer, Rgb};
use std::fs::File;
use std::io::{BufWriter, Cursor, Write};
use std::path::Path;

pub const DIRECT_EXE_DOWNLOAD_URL: &str =
    "https://github.com/asanaim/ShunnoScan-Windows/releases/latest/download/ShunnoScan.exe";

/// Composites two scanned images (Front & Back of an ID card or passport) onto a single A4 portrait canvas.
pub fn stitch_two_pages_onto_a4(front: &DynamicImage, back: &DynamicImage) -> DynamicImage {
    let a4_w = front.width().max(back.width()).max(1654);
    let a4_h = ((a4_w as f32) * 1.4142) as u32;
    let mut canvas = ImageBuffer::from_pixel(a4_w, a4_h, Rgb([255u8, 255u8, 255u8]));

    let pad = ((a4_w as f32) * 0.06) as u32;
    let slot_w = a4_w - pad * 2;
    let slot_h = (a4_h - pad * 3) / 2;

    let draw_centered = |canvas: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, img: &DynamicImage, top_y: u32| {
        let scale = (slot_w as f32 / img.width() as f32).min(slot_h as f32 / img.height() as f32);
        let dst_w = ((img.width() as f32) * scale).max(1.0) as u32;
        let dst_h = ((img.height() as f32) * scale).max(1.0) as u32;
        let resized = img
            .resize_exact(dst_w, dst_h, imageops::FilterType::Lanczos3)
            .to_rgb8();
        let left_x = (a4_w.saturating_sub(dst_w)) / 2;
        let offset_y = top_y + (slot_h.saturating_sub(dst_h)) / 2;
        imageops::overlay(canvas, &resized, left_x as i64, offset_y as i64);
    };

    draw_centered(&mut canvas, front, pad);
    draw_centered(&mut canvas, back, pad * 2 + slot_h);
    DynamicImage::ImageRgb8(canvas)
}

/// Applies permanent black redaction bars directly onto the RGB image pixels for detected PII tokens,
/// then encodes the image into an 85% JPEG stream (cutting PDF size by ~90% with crisp 300 DPI edges).
pub fn prepare_page_jpeg(
    img: &DynamicImage,
    words: &[WordBox],
    redact_pii: bool,
) -> Result<(u32, u32, Vec<u8>), String> {
    let mut rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();

    if redact_pii {
        for word in words {
            if is_sensitive_token(word.text.trim()) {
                let x0 = (word.x.max(0.0) as u32).min(w.saturating_sub(1));
                let y0 = (word.y.max(0.0) as u32).min(h.saturating_sub(1));
                let x1 = ((word.x + word.w).ceil().max(0.0) as u32).min(w);
                let y1 = ((word.y + word.h).ceil().max(0.0) as u32).min(h);
                for py in y0..y1 {
                    for px in x0..x1 {
                        rgb.put_pixel(px, py, Rgb([0, 0, 0]));
                    }
                }
            }
        }
    }

    let mut jpeg_buf = Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, 85);
    encoder
        .encode(&rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("JPEG encode error: {}", e))?;

    Ok((w, h, jpeg_buf.into_inner()))
}

fn escape_pdf_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\\' => out.push_str("\\\\"),
            '\r' | '\n' => out.push(' '),
            c if c.is_ascii() => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

/// Pure-Rust Searchable "Sandwich" PDF 1.4 Writer:
/// - Embeds JPEG 85% stream directly (/Filter /DCTDecode)
/// - Writes ISO PDF Text Rendering Mode 3 (`3 Tr`) invisible selectable words at exact bounding boxes
/// - Embeds the app download link invisibly in the PDF stream (never printed visibly over the page)
pub fn write_searchable_pdf(
    output_path: &Path,
    pages: &[PageOcrResult],
    redact_pii: bool,
) -> Result<(), String> {
    if pages.is_empty() {
        return Err("No pages to write".into());
    }

    let file = File::create(output_path).map_err(|e| format!("Cannot create PDF: {}", e))?;
    let mut writer = BufWriter::new(file);
    let mut buf: Vec<u8> = Vec::new();
    let mut xref_offsets: Vec<usize> = vec![0]; // Object 0 is free

    // PDF 1.4 binary header
    buf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    // Object 1: Catalog
    xref_offsets.push(buf.len());
    buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Object 2: Pages root (we will fill Kids list after knowing page object IDs)
    // Object 3: Standard Type1 Helvetica Font for invisible OCR overlay
    let page_count = pages.len();
    let mut kids_refs = String::new();
    for i in 0..page_count {
        let page_obj_id = 4 + i * 3;
        kids_refs.push_str(&format!("{} 0 R ", page_obj_id));
    }

    xref_offsets.push(buf.len());
    buf.extend_from_slice(
        format!(
            "2 0 obj\n<< /Type /Pages /Kids [ {}] /Count {} >>\nendobj\n",
            kids_refs, page_count
        )
        .as_bytes(),
    );

    xref_offsets.push(buf.len());
    buf.extend_from_slice(
        b"3 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\nendobj\n",
    );

    for (idx, page) in pages.iter().enumerate() {
        let page_obj_id = 4 + idx * 3;
        let content_obj_id = page_obj_id + 1;
        let image_obj_id = page_obj_id + 2;

        let w = page.image_width.max(1) as f32;
        let h = page.image_height.max(1) as f32;

        // Build content stream:
        // 1) Draw full-page JPEG image
        // 2) Draw invisible searchable text overlay (3 Tr = invisible rendering mode)
        let mut content = String::new();
        content.push_str(&format!("q\n{} 0 0 {} 0 0 cm\n/Im0 Do\nQ\n", w, h));
        content.push_str("BT\n3 Tr\n");

        for word in &page.words {
            let trimmed = word.text.trim();
            if trimmed.is_empty() || word.w <= 0.0 || word.h <= 0.0 {
                continue;
            }
            if redact_pii && is_sensitive_token(trimmed) {
                continue;
            }

            let font_size = (word.h * 0.85).max(4.0);
            // Convert top-down image Y to bottom-up PDF Y coordinate
            let pdf_x = word.x.max(0.0);
            let pdf_y = (h - (word.y + word.h * 0.88)).max(2.0);

            let safe_ascii = escape_pdf_literal(trimmed);
            let approx_char_w = font_size * 0.52 * (safe_ascii.chars().count().max(1) as f32);
            let h_scale = ((word.w / approx_char_w) * 100.0).clamp(45.0, 240.0);

            content.push_str(&format!(
                "/F1 {:.2} Tf\n{:.1} Tz\n1 0 0 1 {:.2} {:.2} Tm\n({}) Tj\n",
                font_size, h_scale, pdf_x, pdf_y, safe_ascii
            ));
        }

        // Embed download link invisibly in the PDF stream (never visibly printed on page)
        let hidden_url = escape_pdf_literal(DIRECT_EXE_DOWNLOAD_URL);
        content.push_str(&format!(
            "/F1 8 Tf\n100 Tz\n1 0 0 1 12 6 Tm\n({}) Tj\nET\n",
            hidden_url
        ));

        // Page Object
        xref_offsets.push(buf.len());
        buf.extend_from_slice(
            format!(
                "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R >> /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>\nendobj\n",
                page_obj_id, w, h, image_obj_id, content_obj_id
            )
            .as_bytes(),
        );

        // Content Stream Object
        xref_offsets.push(buf.len());
        buf.extend_from_slice(
            format!(
                "{} 0 obj\n<< /Length {} >>\nstream\n{}endstream\nendobj\n",
                content_obj_id,
                content.len(),
                content
            )
            .as_bytes(),
        );

        // JPEG XObject Image Stream
        xref_offsets.push(buf.len());
        buf.extend_from_slice(
            format!(
                "{} 0 obj\n<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
                image_obj_id,
                page.image_width,
                page.image_height,
                page.jpeg_bytes.len()
            )
            .as_bytes(),
        );
        buf.extend_from_slice(&page.jpeg_bytes);
        buf.extend_from_slice(b"\nendstream\nendobj\n");
    }

    let xref_start = buf.len();
    buf.extend_from_slice(format!("xref\n0 {}\n", xref_offsets.len()).as_bytes());
    buf.extend_from_slice(b"0000000000 65535 f \n");
    for off in xref_offsets.iter().skip(1) {
        buf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }

    buf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            xref_offsets.len(),
            xref_start
        )
        .as_bytes(),
    );

    writer
        .write_all(&buf)
        .map_err(|e| format!("Failed writing PDF: {}", e))?;
    writer.flush().map_err(|e| format!("Flush error: {}", e))
}

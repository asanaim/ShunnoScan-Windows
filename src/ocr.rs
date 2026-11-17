use crate::models::{app_data_dir, tessdata_dir, OcrScript, PageOcrResult, WordBox};
use crate::pdf_gen::prepare_page_jpeg;
use image::DynamicImage;
use rayon::prelude::*;
use std::fs::{self, File};
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Checks if a language script is ready for offline use.
pub fn is_script_installed(script: OcrScript) -> bool {
    if script.is_built_in_default() {
        return true;
    }
    if let Some(fname) = script.traineddata_file() {
        let p = tessdata_dir().join(fname);
        if let Ok(meta) = fs::metadata(&p) {
            return meta.len() > 100_000;
        }
    }
    false
}

/// Downloads a `.traineddata` model on demand into `%LocalAppData%\ShunnoScan\tessdata`.
pub fn download_script_model(script: OcrScript) -> Result<(), String> {
    let fname = match script.traineddata_file() {
        Some(f) => f,
        None => return Ok(()),
    };

    let dir = tessdata_dir();
    let target = dir.join(fname);
    let temp = dir.join(format!("{}.tmp", fname));
    let url = format!("https://github.com/tesseract-ocr/tessdata_fast/raw/main/{}", fname);

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Network error downloading {}: {}", fname, e))?;

    let mut reader = resp.into_reader();
    let mut out = File::create(&temp).map_err(|e| format!("File error: {}", e))?;
    std::io::copy(&mut reader, &mut out).map_err(|e| format!("Write error: {}", e))?;
    out.flush().map_err(|e| format!("Flush error: {}", e))?;
    drop(out);

    if fs::metadata(&temp).map(|m| m.len()).unwrap_or(0) < 100_000 {
        let _ = fs::remove_file(&temp);
        return Err("Downloaded model file was incomplete.".into());
    }

    let _ = fs::remove_file(&target);
    fs::rename(&temp, &target).map_err(|e| format!("Rename error: {}", e))?;
    Ok(())
}

/// Deletes an optional downloaded `.traineddata` model to free disk space.
pub fn delete_script_model(script: OcrScript) {
    if let Some(fname) = script.traineddata_file() {
        let target = tessdata_dir().join(fname);
        let _ = fs::remove_file(target);
    }
}

/// Ensures `eng.traineddata` and `ben.traineddata` exist in `%LocalAppData%\ShunnoScan\tessdata`.
/// Checks next to `ShunnoScan.exe` first (bundled release), or auto-fetches if absent.
pub fn ensure_default_tessdata() {
    let dest_dir = tessdata_dir();
    let exe_tessdir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("tessdata")));

    for fname in ["eng.traineddata", "ben.traineddata"] {
        let target = dest_dir.join(fname);
        if target.exists() && fs::metadata(&target).map(|m| m.len()).unwrap_or(0) > 100_000 {
            continue;
        }
        if let Some(ref bundled_dir) = exe_tessdir {
            let bundled = bundled_dir.join(fname);
            if bundled.exists() {
                let _ = fs::copy(&bundled, &target);
                continue;
            }
        }
        // Background fetch if running standalone .exe without bundled tessdata folder
        let url = format!("https://github.com/tesseract-ocr/tessdata_fast/raw/main/{}", fname);
        if let Ok(resp) = ureq::get(&url).call() {
            if let Ok(mut f) = File::create(&target) {
                let _ = std::io::copy(&mut resp.into_reader(), &mut f);
            }
        }
    }
}

fn find_tesseract_exe() -> Option<PathBuf> {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(dir) = exe_path.parent() {
            let local_tess = dir.join("tesseract").join("tesseract.exe");
            if local_tess.exists() {
                return Some(local_tess);
            }
            let sibling_tess = dir.join("tesseract.exe");
            if sibling_tess.exists() {
                return Some(sibling_tess);
            }
        }
    }
    let std_prog = PathBuf::from(r"C:\Program Files\Tesseract-OCR\tesseract.exe");
    if std_prog.exists() {
        return Some(std_prog);
    }
    // Check PATH
    let mut cmd = Command::new("tesseract");
    cmd.arg("--version");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    if cmd.output().is_ok() {
        return Some(PathBuf::from("tesseract"));
    }
    None
}

/// Processes a batch of images in parallel across all CPU cores using Rayon,
/// returning word bounding boxes, recognized text, and 85% compressed JPEG streams.
pub fn process_pages_parallel(
    images: &[DynamicImage],
    script: OcrScript,
    redact_pii: bool,
) -> Vec<PageOcrResult> {
    images
        .par_iter()
        .enumerate()
        .map(|(idx, img)| {
            let (words, full_text) = run_single_page_ocr(img, script, idx);
            let (w, h, jpeg_bytes) =
                prepare_page_jpeg(img, &words, redact_pii).unwrap_or((img.width(), img.height(), Vec::new()));
            PageOcrResult {
                image_width: w,
                image_height: h,
                jpeg_bytes,
                words,
                full_text,
            }
        })
        .collect()
}

fn run_single_page_ocr(img: &DynamicImage, script: OcrScript, page_idx: usize) -> (Vec<WordBox>, String) {
    // 1. Try Tesseract 5 TSV word-box engine first (supports Bangla+English out-of-the-box)
    if let Some(tess_exe) = find_tesseract_exe() {
        if let Some(res) = run_tesseract_tsv(&tess_exe, img, script.tess_code(), page_idx) {
            return res;
        }
    }

    // 2. Fallback to native Windows 10/11 built-in WinRT OCR engine (zero external binaries needed!)
    #[cfg(windows)]
    {
        if let Some(res) = run_windows_native_ocr(img, script.winrt_bcp47()) {
            return res;
        }
    }

    (Vec::new(), String::new())
}

fn run_tesseract_tsv(
    tess_exe: &PathBuf,
    img: &DynamicImage,
    lang_code: &str,
    page_idx: usize,
) -> Option<(Vec<WordBox>, String)> {
    let temp_png = app_data_dir().join(format!("tmp_ocr_page_{}_{}.png", std::process::id(), page_idx));
    img.save(&temp_png).ok()?;

    let mut cmd = Command::new(tess_exe);
    cmd.arg(&temp_png)
        .arg("stdout")
        .arg("--tessdata-dir")
        .arg(tessdata_dir())
        .arg("-l")
        .arg(lang_code)
        .arg("tsv");

    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

    let output = cmd.output().ok();
    let _ = fs::remove_file(&temp_png);
    let out = output?;
    if !out.status.success() {
        return None;
    }

    let tsv = String::from_utf8_lossy(&out.stdout);
    let mut words = Vec::new();
    let mut lines_map: std::collections::BTreeMap<(i32, i32, i32), Vec<String>> =
        std::collections::BTreeMap::new();

    for (line_no, row) in tsv.lines().enumerate() {
        if line_no == 0 {
            continue; // Header: level page_num block_num par_num line_num word_num left top width height conf text
        }
        let cols: Vec<&str> = row.split('\t').collect();
        if cols.len() < 12 {
            continue;
        }
        let level: i32 = cols[0].parse().unwrap_or(0);
        if level != 5 {
            continue; // Word level only
        }
        let text = cols[11].trim();
        if text.is_empty() {
            continue;
        }
        let block_num: i32 = cols[2].parse().unwrap_or(0);
        let par_num: i32 = cols[3].parse().unwrap_or(0);
        let line_num: i32 = cols[4].parse().unwrap_or(0);
        let left: f32 = cols[6].parse().unwrap_or(0.0);
        let top: f32 = cols[7].parse().unwrap_or(0.0);
        let width: f32 = cols[8].parse().unwrap_or(0.0);
        let height: f32 = cols[9].parse().unwrap_or(0.0);

        words.push(WordBox {
            text: text.to_string(),
            x: left,
            y: top,
            w: width,
            h: height,
        });

        lines_map
            .entry((block_num, par_num, line_num))
            .or_default()
            .push(text.to_string());
    }

    let full_text = lines_map
        .into_values()
        .map(|w_list| w_list.join(" "))
        .collect::<Vec<_>>()
        .join("\n");

    Some((words, full_text))
}

#[cfg(windows)]
fn run_windows_native_ocr(img: &DynamicImage, bcp47: &str) -> Option<(Vec<WordBox>, String)> {
    use windows::core::HSTRING;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    let mut png_bytes = Cursor::new(Vec::new());
    img.write_to(&mut png_bytes, image::ImageFormat::Png).ok()?;
    let raw_bytes = png_bytes.into_inner();

    let stream = InMemoryRandomAccessStream::new().ok()?;
    let writer = DataWriter::CreateDataWriter(&stream).ok()?;
    writer.WriteBytes(&raw_bytes).ok()?;
    writer.StoreAsync().ok()?.get().ok()?;
    writer.FlushAsync().ok()?.get().ok()?;
    stream.Seek(0).ok()?;

    let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let software_bitmap = decoder.GetSoftwareBitmapAsync().ok()?.get().ok()?;

    let engine = Language::CreateLanguage(&HSTRING::from(bcp47))
        .ok()
        .and_then(|lang| OcrEngine::TryCreateFromLanguage(&lang).ok())
        .or_else(|| OcrEngine::TryCreateFromUserProfileLanguages().ok())?;

    let result = engine.RecognizeAsync(&software_bitmap).ok()?.get().ok()?;
    let lines = result.Lines().ok()?;

    let mut words = Vec::new();
    let mut full_lines = Vec::new();

    for i in 0..lines.Size().unwrap_or(0) {
        if let Ok(line) = lines.GetAt(i) {
            if let Ok(line_text) = line.Text() {
                full_lines.push(line_text.to_string());
            }
            if let Ok(line_words) = line.Words() {
                for j in 0..line_words.Size().unwrap_or(0) {
                    if let Ok(w) = line_words.GetAt(j) {
                        if let (Ok(t), Ok(r)) = (w.Text(), w.BoundingRect()) {
                            words.push(WordBox {
                                text: t.to_string(),
                                x: r.X,
                                y: r.Y,
                                w: r.Width,
                                h: r.Height,
                            });
                        }
                    }
                }
            }
        }
    }

    Some((words, full_lines.join("\n")))
}

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OcrScript {
    English,
    BanglaEnglish,
    DevanagariEnglish,
    ChineseEnglish,
    JapaneseEnglish,
    KoreanEnglish,
}

impl OcrScript {
    pub fn all() -> &'static [OcrScript] {
        &[
            OcrScript::English,
            OcrScript::BanglaEnglish,
            OcrScript::DevanagariEnglish,
            OcrScript::ChineseEnglish,
            OcrScript::JapaneseEnglish,
            OcrScript::KoreanEnglish,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            OcrScript::English => "English",
            OcrScript::BanglaEnglish => "Bangla + English",
            OcrScript::DevanagariEnglish => "Devanagari + English",
            OcrScript::ChineseEnglish => "Chinese + English",
            OcrScript::JapaneseEnglish => "Japanese + English",
            OcrScript::KoreanEnglish => "Korean + English",
        }
    }

    pub fn tess_code(&self) -> &'static str {
        match self {
            OcrScript::English => "eng",
            OcrScript::BanglaEnglish => "ben+eng",
            OcrScript::DevanagariEnglish => "hin+eng",
            OcrScript::ChineseEnglish => "chi_sim+eng",
            OcrScript::JapaneseEnglish => "jpn+eng",
            OcrScript::KoreanEnglish => "kor+eng",
        }
    }

    pub fn winrt_bcp47(&self) -> &'static str {
        match self {
            OcrScript::English => "en-US",
            OcrScript::BanglaEnglish => "bn-BD",
            OcrScript::DevanagariEnglish => "hi-IN",
            OcrScript::ChineseEnglish => "zh-Hans-CN",
            OcrScript::JapaneseEnglish => "ja-JP",
            OcrScript::KoreanEnglish => "ko-KR",
        }
    }

    pub fn traineddata_file(&self) -> Option<&'static str> {
        match self {
            OcrScript::English => None,
            OcrScript::BanglaEnglish => Some("ben.traineddata"),
            OcrScript::DevanagariEnglish => Some("hin.traineddata"),
            OcrScript::ChineseEnglish => Some("chi_sim.traineddata"),
            OcrScript::JapaneseEnglish => Some("jpn.traineddata"),
            OcrScript::KoreanEnglish => Some("kor.traineddata"),
        }
    }

    pub fn is_built_in_default(&self) -> bool {
        matches!(self, OcrScript::English | OcrScript::BanglaEnglish)
    }

    pub fn approx_size(&self) -> &'static str {
        match self {
            OcrScript::English => "Built-in",
            OcrScript::BanglaEnglish => "Built-in",
            OcrScript::DevanagariEnglish => "1.8 MB",
            OcrScript::ChineseEnglish => "2.4 MB",
            OcrScript::JapaneseEnglish => "2.4 MB",
            OcrScript::KoreanEnglish => "1.6 MB",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordBox {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone)]
pub struct PageOcrResult {
    pub image_width: u32,
    pub image_height: u32,
    pub jpeg_bytes: Vec<u8>,
    pub words: Vec<WordBox>,
    pub full_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannedDoc {
    pub id: u64,
    pub title: String,
    pub pdf_path: String,
    pub txt_path: Option<String>,
    pub csv_path: Option<String>,
    pub vcf_path: Option<String>,
    pub page_count: usize,
    pub date_str: String,
    pub script_label: String,
    pub ocr_text: String,
    pub csv_table: String,
    pub detected_phone: Option<String>,
    pub detected_email: Option<String>,
    pub is_redacted: bool,
    pub is_id_stitch: bool,
}

pub fn app_data_dir() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("ShunnoScan");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn tessdata_dir() -> PathBuf {
    let dir = app_data_dir().join("tessdata");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn default_output_dir() -> PathBuf {
    let base = dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("ShunnoScan");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn load_history() -> Vec<ScannedDoc> {
    let path = app_data_dir().join("shunnoscan_history.json");
    if let Ok(data) = fs::read_to_string(path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub fn save_history(docs: &[ScannedDoc]) {
    let path = app_data_dir().join("shunnoscan_history.json");
    if let Ok(json) = serde_json::to_string_pretty(docs) {
        let _ = fs::write(path, json);
    }
}

pub fn is_sensitive_token(token: &str) -> bool {
    static PII_RE: OnceLock<Regex> = OnceLock::new();
    let re = PII_RE.get_or_init(|| {
        Regex::new(
            r"(\b\d{4}[ -]?\d{4}[ -]?\d{4,7}\b)|(\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b)|(\+?\d[\d -]{7,14}\d)",
        )
        .unwrap()
    });
    re.is_match(token)
}

pub fn extract_phone(text: &str) -> Option<String> {
    static PHONE_RE: OnceLock<Regex> = OnceLock::new();
    let re = PHONE_RE.get_or_init(|| Regex::new(r"(\+?\d[\d -]{7,14}\d)").unwrap());
    re.find(text).map(|m| m.as_str().trim().to_string())
}

pub fn extract_email(text: &str) -> Option<String> {
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    let re = EMAIL_RE.get_or_init(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap());
    re.find(text).map(|m| m.as_str().trim().to_string())
}

/// Preserves Unicode letters AND combining marks (\p{M}) so Bangla vowel signs
/// (া, ি, ী, ু, ূ, ৃ, ে, ৈ, ো, ৌ, ং, ্) remain intact in exported filenames.
pub fn generate_smart_filename(pages: &[PageOcrResult], doc_id: u64) -> String {
    static SANITIZE_RE: OnceLock<Regex> = OnceLock::new();
    let re = SANITIZE_RE.get_or_init(|| Regex::new(r"[^\p{L}\p{M}\p{N}_-]+").unwrap());

    let first_line = pages
        .first()
        .and_then(|p| {
            p.full_text
                .lines()
                .map(|l| l.trim())
                .find(|l| l.chars().count() >= 3 && l.chars().count() <= 48 && l.chars().any(|c| c.is_alphabetic()))
        });

    if let Some(line) = first_line {
        let cleaned = re.replace_all(line, "_");
        let trimmed = cleaned.trim_matches('_');
        let short: String = trimmed.chars().take(36).collect();
        if !short.is_empty() {
            return format!("{}_{}", short, doc_id % 10000);
        }
    }
    format!("ShunnoScan_{}", doc_id % 100000)
}

pub fn generate_vcard(name: &str, phone: Option<&str>, email: Option<&str>) -> String {
    let mut vcf = String::from("BEGIN:VCARD\r\nVERSION:3.0\r\n");
    vcf.push_str(&format!("FN:{}\r\nN:{};;;;\r\n", name, name));
    if let Some(p) = phone {
        vcf.push_str(&format!("TEL;TYPE=CELL:{}\r\n", p));
    }
    if let Some(e) = email {
        vcf.push_str(&format!("EMAIL;TYPE=INTERNET:{}\r\n", e));
    }
    vcf.push_str("END:VCARD\r\n");
    vcf
}

pub fn current_timestamp() -> (u64, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Simple UTC/local readable timestamp without heavy chrono crate bloat
    let secs_in_day = now % 86400;
    let hours = (secs_in_day / 3600) % 24;
    let mins = (secs_in_day % 3600) / 60;
    let days = now / 86400;
    let approx_year = 1970 + (days / 365);
    let day_of_year = (days % 365) + 1;
    (now * 1000, format!("{} • Day {} • {:02}:{:02}", approx_year, day_of_year, hours, mins))
}

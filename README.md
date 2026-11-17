# Shunno Scan for Windows (Native Rust Edition)

**Shunno Scan** (*শূন্য স্ক্যান* — meaning **Zero** / `0`) is an ultra-lightweight (**~6 MB**), native **Rust** desktop document scanner, multi-script Optical Character Recognition (OCR) suite, and searchable PDF generator for Windows 10 and 11.

Built with **Rust (`eframe`/`egui` + `rayon`)**, it opens instantaneously (< 0.1s), uses under 25 MB of RAM, and requires **zero external runtimes** (no Python, no .NET, no Java).

---

## 📲 1-Click Direct Download for Windows (Non-IT Friendly)

Click the link below on any Windows 10 or 11 PC to download and run **Shunno Scan** immediately:

- **[👉 Download ShunnoScan.exe (Standalone ~6 MB Native Rust EXE)](https://github.com/asanaim/ShunnoScan-Windows/releases/latest/download/ShunnoScan.exe)**
- **[📦 Download ShunnoScan-Windows-Bundle.zip (Includes Pre-Bundled Bangla + English Offline Models)](https://github.com/asanaim/ShunnoScan-Windows/releases/latest/download/ShunnoScan-Windows-Bundle.zip)**

---

## 🎯 What It Does

1. **Drag-and-Drop, File Import & Clipboard Snip (`Ctrl+V`) OCR:**
   - Drag and drop single or multi-page document scans (PNG, JPG, WebP, BMP, TIFF) directly onto the window, pick files via the native Windows file dialog, or press **`Win + Shift + S`** to snip any screen region and press **`Ctrl + V`** inside Shunno Scan to convert it into a searchable PDF.
2. **True Searchable "Sandwich" PDF Generation:**
   - Generates standard PDF 1.4 documents where every scanned page image is paired with an aligned, invisible selectable text layer (`3 Tr` rendering mode) so you can search (`Ctrl+F`), highlight, and copy words in Adobe Acrobat, Edge, Chrome, or SumatraPDF.
3. **Hybrid Cupertino Multi-Script Offline OCR:**
   - **Default Built-In Languages:** **English** and **Bangla + English (`বাংলা + English`)** work 100% offline out of the box.
   - **On-Demand Downloadable & Removable Scripts:** Click **`+ Languages`** to download **Devanagari (Hindi) + English**, **Chinese + English**, **Japanese + English**, or **Korean + English**, and remove them anytime (via **Remove** or right-clicking any downloaded language chip) to save disk space.
4. **Unicode-Aware Smart File Naming:**
   - Inspects the top heading of Page 1 and automatically names exported `.pdf`, `.txt`, and `.csv` files after the document's title while preserving Bangla combining vowel signs (`া`, `ি`, `ী`, `ু`, `ূ`, `ৃ`, `ে`, `ৈ`, `ো`, `ৌ`, `ং`, `্`).
5. **🪪 ID Card 2-in-1 A4 Page Stitcher:**
   - Toggle on **`ID Card 2-in-1 A4 Page`** to composite the Front and Back of an ID card, passport, or driver's license onto a single A4 page before OCR and PDF export.
6. **🔒 Auto-Redact PII (Privacy Sanitizer):**
   - Automatically detects phone numbers, emails, and 12-to-16-digit card/ID numbers, paints permanent black redaction bars directly onto the image pixels, and removes those tokens from the PDF text layer and `.txt` output.
7. **📊 Spatial Table-to-CSV & 📇 Business Card `.vcf` Export:**
   - Clusters word bounding boxes into rows and columns to export Excel-ready `_table.csv` files, and extracts phone/email metadata into one-click `.vcf` contact cards.

---

## ⚙️ How It Does It (Native Rust Architecture)

- **Pure-Rust Searchable PDF 1.4 Writer (`src/pdf_gen.rs`):** Encodes images through an 85% JPEG byte stream (shrinking 40 MB multi-page raw scans down to ~2 MB) and embeds them via `/Filter /DCTDecode` alongside ISO PDF `3 Tr` (invisible rendering mode) word-positioned text operators and an invisible embedded download link.
- **Parallel Multi-Core Processing (`src/ocr.rs` & `rayon`):** Uses `par_iter()` across all CPU cores to process multi-page batches simultaneously, combining Tesseract 5 TSV word-box extraction (`ben+eng` & optional scripts) with Windows 10/11's built-in native `Windows.Media.Ocr` WinRT API fallback.
- **Embedded Native Windows Icon (`build.rs` & `assets/app.ico`):** Uses `winres` at compile time to embed the multi-resolution Apple-UI **Shunno (`0`)** cobalt-blue squircle and mint-green laser icon directly into `ShunnoScan.exe`.

---

## 🚀 How to Build `ShunnoScan.exe` on GitHub

1. Extract **`ShunnoScan-Rust-Windows-Suite.zip`** on your computer.
2. Create a new GitHub repository (e.g., `ShunnoScan-Windows`), click **Add file > Upload files**, drag all extracted items (`src`, `assets`, `.github`, `docs`, `Cargo.toml`, `build.rs`, `.gitignore`, `README.md`) into the window, and click **Commit changes**.
3. *(If Windows hides the `.github` folder during drag-and-drop, click **Add file > Create new file**, name it `.github/workflows/main.yml`, paste the contents of `main.yml` from the ZIP, and commit.)*
4. Open the **Actions** tab: once the green checkmark (`✓`) appears, download **`ShunnoScan.exe`** from your repository's **Releases** page!

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
mod ocr;
mod pdf_gen;
mod table_csv;

use arboard::Clipboard;
use eframe::egui::{self, Color32, CornerRadius, FontId, RichText, Stroke, Vec2};
use image::{DynamicImage, ImageBuffer, Rgba};
use models::{
    current_timestamp, default_output_dir, extract_email, extract_phone, generate_smart_filename,
    generate_vcard, is_sensitive_token, load_history, save_history, OcrScript, ScannedDoc,
};
use ocr::{
    delete_script_model, download_script_model, ensure_default_tessdata, is_script_installed,
    process_pages_parallel,
};
use pdf_gen::{stitch_two_pages_onto_a4, write_searchable_pdf};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use table_csv::convert_words_to_csv;

enum WorkerEvent {
    ScanCompleted(ScannedDoc),
    ScanFailed(String),
    LanguageDownloaded(OcrScript, Result<(), String>),
}

struct ShunnoScanApp {
    selected_script: OcrScript,
    installed_scripts: Vec<OcrScript>,
    id_card_stitch_mode: bool,
    redact_pii_mode: bool,
    search_query: String,
    docs: Vec<ScannedDoc>,
    active_ocr_doc: Option<ScannedDoc>,
    show_language_manager: bool,
    confirm_remove_script: Option<OcrScript>,
    downloading_script: Option<OcrScript>,
    is_processing: bool,
    status_message: String,
    tx: Sender<WorkerEvent>,
    rx: Receiver<WorkerEvent>,
}

impl ShunnoScanApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();

        // Warm default Bangla + English traineddata in background
        thread::spawn(|| {
            ensure_default_tessdata();
        });

        let installed_scripts: Vec<OcrScript> = OcrScript::all()
            .iter()
            .copied()
            .filter(|s| is_script_installed(*s))
            .collect();

        let docs = load_history();

        Self {
            selected_script: OcrScript::BanglaEnglish,
            installed_scripts,
            id_card_stitch_mode: false,
            redact_pii_mode: false,
            search_query: String::new(),
            docs,
            active_ocr_doc: None,
            show_language_manager: false,
            confirm_remove_script: None,
            downloading_script: None,
            is_processing: false,
            status_message: "Ready • Drop images, Import Files, or Paste Snip (Ctrl+V)".into(),
            tx,
            rx,
        }
    }

    fn process_image_paths(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() || self.is_processing {
            return;
        }
        self.is_processing = true;
        self.status_message = format!(
            "Processing {} file(s) with {}...",
            paths.len(),
            self.selected_script.label()
        );

        let script = self.selected_script;
        let stitch_id = self.id_card_stitch_mode;
        let redact_pii = self.redact_pii_mode;
        let tx = self.tx.clone();

        thread::spawn(move || {
            let mut loaded_images: Vec<DynamicImage> = Vec::new();
            for p in &paths {
                if let Ok(img) = image::open(p) {
                    loaded_images.push(img);
                }
            }
            if loaded_images.is_empty() {
                let _ = tx.send(WorkerEvent::ScanFailed(
                    "No valid image files found (supports PNG, JPG, WebP, BMP, TIFF).".into(),
                ));
                return;
            }
            Self::finish_image_pipeline(loaded_images, script, stitch_id, redact_pii, tx);
        });
    }

    fn process_clipboard_image(&mut self) {
        if self.is_processing {
            return;
        }
        let mut clipboard = match Clipboard::new() {
            Ok(cb) => cb,
            Err(_) => {
                self.status_message = "Clipboard unavailable.".into();
                return;
            }
        };
        let img_data = match clipboard.get_image() {
            Ok(data) => data,
            Err(_) => {
                self.status_message = "No image found in clipboard (use Win+Shift+S to snip first).".into();
                return;
            }
        };

        let rgba = match ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(
            img_data.width as u32,
            img_data.height as u32,
            img_data.bytes.into_owned(),
        ) {
            Some(buf) => DynamicImage::ImageRgba8(buf),
            None => return,
        };

        self.is_processing = true;
        self.status_message = format!("Running OCR on clipboard image ({})...", self.selected_script.label());
        let script = self.selected_script;
        let redact_pii = self.redact_pii_mode;
        let tx = self.tx.clone();

        thread::spawn(move || {
            Self::finish_image_pipeline(vec![rgba], script, false, redact_pii, tx);
        });
    }

    fn finish_image_pipeline(
        mut images: Vec<DynamicImage>,
        script: OcrScript,
        stitch_id: bool,
        redact_pii: bool,
        tx: Sender<WorkerEvent>,
    ) {
        if stitch_id && images.len() >= 2 {
            let stitched = stitch_two_pages_onto_a4(&images[0], &images[1]);
            let mut next = vec![stitched];
            next.extend(images.into_iter().skip(2));
            images = next;
        }

        let page_results = process_pages_parallel(&images, script, redact_pii);
        let (doc_id, date_str) = current_timestamp();
        let smart_base = generate_smart_filename(&page_results, doc_id);
        let out_dir = default_output_dir();

        let pdf_file_name = format!("{}.pdf", smart_base);
        let pdf_path = out_dir.join(&pdf_file_name);

        if let Err(e) = write_searchable_pdf(&pdf_path, &page_results, redact_pii) {
            let _ = tx.send(WorkerEvent::ScanFailed(e));
            return;
        }

        let combined_text = page_results
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let raw = p.full_text.trim();
                if raw.is_empty() {
                    return None;
                }
                let cleaned = if redact_pii {
                    raw.split_whitespace()
                        .map(|tok| if is_sensitive_token(tok) { "[REDACTED]" } else { tok })
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    raw.to_string()
                };
                if page_results.len() == 1 {
                    Some(cleaned)
                } else {
                    Some(format!("--- Page {} ---\n{}", i + 1, cleaned))
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let txt_path = if !combined_text.is_empty() {
            let p = out_dir.join(format!("{}.txt", smart_base));
            let _ = fs::write(&p, &combined_text);
            Some(p.to_string_lossy().to_string())
        } else {
            None
        };

        let csv_table = convert_words_to_csv(&page_results);
        let csv_path = if !csv_table.is_empty() {
            let p = out_dir.join(format!("{}_table.csv", smart_base));
            let _ = fs::write(&p, &csv_table);
            Some(p.to_string_lossy().to_string())
        } else {
            None
        };

        let phone = extract_phone(&combined_text);
        let email = extract_email(&combined_text);
        let vcf_path = if phone.is_some() || email.is_some() {
            let vcf_content = generate_vcard(&smart_base, phone.as_deref(), email.as_deref());
            let p = out_dir.join(format!("{}.vcf", smart_base));
            let _ = fs::write(&p, vcf_content);
            Some(p.to_string_lossy().to_string())
        } else {
            None
        };

        let doc = ScannedDoc {
            id: doc_id,
            title: pdf_file_name,
            pdf_path: pdf_path.to_string_lossy().to_string(),
            txt_path,
            csv_path,
            vcf_path,
            page_count: page_results.len(),
            date_str,
            script_label: script.label().to_string(),
            ocr_text: combined_text,
            csv_table,
            detected_phone: phone,
            detected_email: email,
            is_redacted: redact_pii,
            is_id_stitch: stitch_id,
        };

        let _ = tx.send(WorkerEvent::ScanCompleted(doc));
    }

    fn open_in_windows(path_str: &str) {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", path_str])
            .spawn();
    }
}

impl eframe::App for ShunnoScanApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll background worker events
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                WorkerEvent::ScanCompleted(doc) => {
                    self.is_processing = false;
                    self.status_message = format!("Saved {} to Downloads\\ShunnoScan", doc.title);
                    self.docs.insert(0, doc);
                    save_history(&self.docs);
                }
                WorkerEvent::ScanFailed(err) => {
                    self.is_processing = false;
                    self.status_message = format!("Error: {}", err);
                }
                WorkerEvent::LanguageDownloaded(script, result) => {
                    self.downloading_script = None;
                    match result {
                        Ok(()) => {
                            if !self.installed_scripts.contains(&script) {
                                self.installed_scripts.push(script);
                            }
                            self.selected_script = script;
                            self.status_message = format!("{} downloaded for offline use.", script.label());
                        }
                        Err(e) => {
                            self.status_message = e;
                        }
                    }
                }
            }
        }

        // Handle Ctrl+V paste image from clipboard
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::V)) {
            self.process_clipboard_image();
        }

        // Handle Drag-and-Drop files onto window
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.process_image_paths(dropped);
        }

        // Apple Cupertino Palette
        let apple_bg = Color32::from_rgb(242, 242, 247);
        let apple_card = Color32::WHITE;
        let apple_blue = Color32::from_rgb(10, 132, 255);
        let apple_green = Color32::from_rgb(48, 209, 88);
        let apple_orange = Color32::from_rgb(255, 159, 10);
        let apple_red = Color32::from_rgb(255, 59, 48);
        let apple_gray_text = Color32::from_rgb(110, 110, 115);

        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = apple_bg;
        visuals.window_fill = apple_card;
        ctx.set_visuals(visuals);

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(apple_bg).inner_margin(24.0))
            .show(ctx, |ui| {
                // Header Row with Apple-UI "0" (Shunno) Squircle Badge
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(44.0, 44.0), egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, CornerRadius::same(11), apple_blue);
                    // Geometric "0" (Shunno) ring
                    let zero_rect = egui::Rect::from_center_size(rect.center(), Vec2::new(19.0, 27.0));
                    painter.rect_stroke(
                        zero_rect,
                        CornerRadius::same(10),
                        Stroke::new(2.8, Color32::WHITE),
                        egui::epaint::StrokeKind::Middle,
                    );
                    // Green laser horizon beam through "0"
                    painter.line_segment(
                        [
                            egui::pos2(rect.left() + 6.0, rect.center().y),
                            egui::pos2(rect.right() - 6.0, rect.center().y),
                        ],
                        Stroke::new(2.4, apple_green),
                    );

                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Shunno Scan")
                                .font(FontId::proportional(28.0))
                                .strong()
                                .color(Color32::BLACK),
                        );
                        ui.label(
                            RichText::new("Zero-Cloud Offline Scanner • Searchable PDF • Multi-Script OCR")
                                .font(FontId::proportional(13.0))
                                .color(apple_gray_text),
                        );
                    });
                });

                ui.add_space(16.0);

                // Hybrid Cupertino Language Selector Bar
                ui.horizontal_wrapped(|ui| {
                    let scripts_snapshot = self.installed_scripts.clone();
                    for script in scripts_snapshot {
                        let is_sel = self.selected_script == script;
                        let fill = if is_sel { apple_blue } else { apple_card };
                        let txt_color = if is_sel { Color32::WHITE } else { Color32::BLACK };

                        let btn = egui::Button::new(RichText::new(script.label()).size(13.0).color(txt_color))
                            .fill(fill)
                            .corner_radius(CornerRadius::same(18))
                            .stroke(Stroke::new(1.0, Color32::from_rgb(220, 220, 226)));

                        let resp = ui.add(btn);
                        if resp.clicked() {
                            self.selected_script = script;
                        }
                        if resp.secondary_clicked() && !script.is_built_in_default() {
                            self.confirm_remove_script = Some(script);
                        }
                        if !script.is_built_in_default() {
                            resp.on_hover_text("Click to select • Right-click or open '+ Languages' to remove");
                        }
                    }

                    let add_lang_btn = egui::Button::new(
                        RichText::new("+ Languages")
                            .size(13.0)
                            .strong()
                            .color(apple_blue),
                    )
                    .fill(apple_card)
                    .corner_radius(CornerRadius::same(18))
                    .stroke(Stroke::new(1.2, apple_blue));

                    if ui.add(add_lang_btn).clicked() {
                        self.show_language_manager = true;
                    }
                });

                ui.add_space(10.0);

                // Feature Chips (2-in-1 ID Stitch & Auto-Redact PII)
                ui.horizontal_wrapped(|ui| {
                    let id_fill = if self.id_card_stitch_mode { apple_orange } else { apple_card };
                    let id_txt = if self.id_card_stitch_mode { Color32::WHITE } else { Color32::BLACK };
                    if ui
                        .add(
                            egui::Button::new(RichText::new("🪪 ID Card 2-in-1 A4 Page").size(12.5).color(id_txt))
                                .fill(id_fill)
                                .corner_radius(CornerRadius::same(16))
                                .stroke(Stroke::new(1.0, Color32::from_rgb(220, 220, 226))),
                        )
                        .clicked()
                    {
                        self.id_card_stitch_mode = !self.id_card_stitch_mode;
                    }

                    let pii_fill = if self.redact_pii_mode { Color32::BLACK } else { apple_card };
                    let pii_txt = if self.redact_pii_mode { Color32::WHITE } else { Color32::BLACK };
                    if ui
                        .add(
                            egui::Button::new(RichText::new("🔒 Auto-Redact PII").size(12.5).color(pii_txt))
                                .fill(pii_fill)
                                .corner_radius(CornerRadius::same(16))
                                .stroke(Stroke::new(1.0, Color32::from_rgb(220, 220, 226))),
                        )
                        .clicked()
                    {
                        self.redact_pii_mode = !self.redact_pii_mode;
                    }
                });

                ui.add_space(12.0);

                // Primary Action Buttons
                ui.horizontal(|ui| {
                    let import_btn = egui::Button::new(
                        RichText::new("  Import Images / Scans  ")
                            .size(15.0)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(apple_blue)
                    .min_size(Vec2::new(200.0, 44.0))
                    .corner_radius(CornerRadius::same(12));

                    if ui.add_enabled(!self.is_processing, import_btn).clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"])
                            .set_title("Select Document or Whiteboard Images")
                            .pick_files()
                        {
                            self.process_image_paths(files);
                        }
                    }

                    let paste_btn = egui::Button::new(
                        RichText::new("  Paste Clipboard (Ctrl+V)  ")
                            .size(14.0)
                            .color(apple_blue),
                    )
                    .fill(apple_card)
                    .min_size(Vec2::new(180.0, 44.0))
                    .corner_radius(CornerRadius::same(12))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(210, 210, 215)));

                    if ui.add_enabled(!self.is_processing, paste_btn).clicked() {
                        self.process_clipboard_image();
                    }

                    let folder_btn = egui::Button::new(
                        RichText::new("  Open Output Folder  ")
                            .size(14.0)
                            .color(Color32::BLACK),
                    )
                    .fill(apple_card)
                    .min_size(Vec2::new(160.0, 44.0))
                    .corner_radius(CornerRadius::same(12))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(210, 210, 215)));

                    if ui.add(folder_btn).clicked() {
                        Self::open_in_windows(&default_output_dir().to_string_lossy());
                    }
                });

                ui.add_space(10.0);

                // Status & Progress Bar
                ui.horizontal(|ui| {
                    if self.is_processing {
                        ui.spinner();
                    }
                    ui.label(RichText::new(&self.status_message).size(12.5).color(apple_gray_text));
                });

                ui.add_space(10.0);

                // Instant Full-Text Search Bar
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("Search inside scanned documents & OCR transcriptions...")
                        .desired_width(f32::INFINITY)
                        .margin(Vec2::new(12.0, 10.0)),
                );

                ui.add_space(14.0);

                // Document Library Feed
                let q = self.search_query.trim().to_lowercase();
                let filtered: Vec<ScannedDoc> = self
                    .docs
                    .iter()
                    .filter(|d| {
                        q.is_empty()
                            || d.title.to_lowercase().contains(&q)
                            || d.ocr_text.to_lowercase().contains(&q)
                    })
                    .cloned()
                    .collect();

                if filtered.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("Drag & drop images anywhere onto this window, or click 'Import Images / Scans'.")
                                .size(14.5)
                                .color(apple_gray_text),
                        );
                    });
                } else {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for doc in filtered {
                            egui::Frame::default()
                                .fill(apple_card)
                                .corner_radius(CornerRadius::same(16))
                                .stroke(Stroke::new(1.0, Color32::from_rgb(230, 230, 235)))
                                .inner_margin(16.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(
                                                RichText::new(&doc.title)
                                                    .size(16.0)
                                                    .strong()
                                                    .color(Color32::BLACK),
                                            );
                                            ui.label(
                                                RichText::new(format!(
                                                    "{} page(s) • {} • {}",
                                                    doc.page_count, doc.script_label, doc.date_str
                                                ))
                                                .size(12.0)
                                                .color(apple_gray_text),
                                            );
                                            let mut badge = String::from("✓ Searchable PDF");
                                            if doc.is_id_stitch {
                                                badge.push_str(" • 2-in-1 ID");
                                            }
                                            if doc.is_redacted {
                                                badge.push_str(" • PII Redacted");
                                            }
                                            ui.label(RichText::new(badge).size(12.0).color(apple_green));
                                        });

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .button(
                                                        RichText::new("Open PDF")
                                                            .size(13.0)
                                                            .strong()
                                                            .color(apple_blue),
                                                    )
                                                    .clicked()
                                                {
                                                    Self::open_in_windows(&doc.pdf_path);
                                                }
                                            },
                                        );
                                    });

                                    if !doc.ocr_text.is_empty() {
                                        ui.add_space(8.0);
                                        egui::Frame::default()
                                            .fill(apple_bg)
                                            .corner_radius(CornerRadius::same(10))
                                            .inner_margin(10.0)
                                            .show(ui, |ui| {
                                                let preview: String =
                                                    doc.ocr_text.chars().take(220).collect();
                                                ui.label(
                                                    RichText::new(preview)
                                                        .size(12.5)
                                                        .color(Color32::from_rgb(50, 50, 55)),
                                                );
                                                ui.add_space(8.0);
                                                ui.horizontal(|ui| {
                                                    if ui.button("Full OCR").clicked() {
                                                        self.active_ocr_doc = Some(doc.clone());
                                                    }
                                                    if ui
                                                        .add(
                                                            egui::Button::new(
                                                                RichText::new("Copy Text").color(Color32::WHITE),
                                                            )
                                                            .fill(apple_green),
                                                        )
                                                        .clicked()
                                                    {
                                                        if let Ok(mut cb) = Clipboard::new() {
                                                            let _ = cb.set_text(doc.ocr_text.clone());
                                                            self.status_message =
                                                                "OCR text copied to Windows clipboard.".into();
                                                        }
                                                    }
                                                    if let Some(ref csv_p) = doc.csv_path {
                                                        if ui.button("Open Table CSV").clicked() {
                                                            Self::open_in_windows(csv_p);
                                                        }
                                                    }
                                                    if let Some(ref vcf_p) = doc.vcf_path {
                                                        if ui.button("Save Contact (.vcf)").clicked() {
                                                            Self::open_in_windows(vcf_p);
                                                        }
                                                    }
                                                });
                                            });
                                    }
                                });
                            ui.add_space(10.0);
                        }
                    });
                }
            });

        // Hybrid Cupertino Language Manager Modal
        if self.show_language_manager {
            let mut open = self.show_language_manager;
            egui::Window::new("Manage Offline OCR Languages")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_min_width(400.0);
                    ui.label(
                        RichText::new(
                            "English and Bangla + English are built-in defaults. Download optional scripts when needed or remove them anytime to save disk space.",
                        )
                        .size(12.5)
                        .color(apple_gray_text),
                    );
                    ui.add_space(10.0);

                    for script in OcrScript::all() {
                        let installed = self.installed_scripts.contains(script);
                        let is_dl = self.downloading_script == Some(*script);

                        egui::Frame::default()
                            .fill(apple_bg)
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(RichText::new(script.label()).strong().size(13.5));
                                        ui.label(
                                            RichText::new(script.approx_size())
                                                .size(11.5)
                                                .color(apple_gray_text),
                                        );
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if script.is_built_in_default() {
                                                ui.label(
                                                    RichText::new("Default")
                                                        .strong()
                                                        .color(apple_green),
                                                );
                                            } else if is_dl {
                                                ui.spinner();
                                            } else if installed {
                                                if ui
                                                    .button(
                                                        RichText::new("Remove")
                                                            .strong()
                                                            .color(apple_red),
                                                    )
                                                    .clicked()
                                                {
                                                    delete_script_model(*script);
                                                    self.installed_scripts.retain(|s| s != script);
                                                    if self.selected_script == *script {
                                                        self.selected_script = OcrScript::BanglaEnglish;
                                                    }
                                                }
                                            } else if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("Download")
                                                            .size(12.0)
                                                            .color(Color32::WHITE),
                                                    )
                                                    .fill(apple_blue),
                                                )
                                                .clicked()
                                            {
                                                let target = *script;
                                                self.downloading_script = Some(target);
                                                let tx = self.tx.clone();
                                                thread::spawn(move || {
                                                    let res = download_script_model(target);
                                                    let _ = tx.send(WorkerEvent::LanguageDownloaded(target, res));
                                                });
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
            self.show_language_manager = open;
        }

        // Right-Click Quick Removal Confirmation Dialog
        if let Some(script) = self.confirm_remove_script {
            egui::Window::new(format!("Remove {}?", script.label()))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Delete the offline {} model ({}) from local storage? You can re-download it anytime.",
                        script.label(),
                        script.approx_size()
                    ));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Remove").color(Color32::WHITE))
                                    .fill(apple_red),
                            )
                            .clicked()
                        {
                            delete_script_model(script);
                            self.installed_scripts.retain(|s| *s != script);
                            if self.selected_script == script {
                                self.selected_script = OcrScript::BanglaEnglish;
                            }
                            self.confirm_remove_script = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_remove_script = None;
                        }
                    });
                });
        }

        // Full OCR Viewer Modal
        if let Some(doc) = self.active_ocr_doc.clone() {
            let mut open = true;
            egui::Window::new(&doc.title)
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_size(Vec2::new(540.0, 400.0))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        let mut text_buf = doc.ocr_text.clone();
                        ui.add(
                            egui::TextEdit::multiline(&mut text_buf)
                                .desired_width(f32::INFINITY)
                                .font(FontId::proportional(14.0)),
                        );
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Copy All").color(Color32::WHITE))
                                    .fill(apple_blue),
                            )
                            .clicked()
                        {
                            if let Ok(mut cb) = Clipboard::new() {
                                let _ = cb.set_text(doc.ocr_text.clone());
                                self.status_message = "Copied full OCR text to clipboard.".into();
                            }
                            self.active_ocr_doc = None;
                        }
                        if ui.button("Close").clicked() {
                            self.active_ocr_doc = None;
                        }
                    });
                });
            if !open {
                self.active_ocr_doc = None;
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([860.0, 680.0])
            .with_min_inner_size([640.0, 520.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "Shunno Scan",
        options,
        Box::new(|cc| Ok(Box::new(ShunnoScanApp::new(cc)))),
    )
}

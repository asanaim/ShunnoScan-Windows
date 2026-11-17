use crate::models::{PageOcrResult, WordBox};

/// Spatial Y/X bounding-box clustering to convert printed tables into RFC-4180 CSV spreadsheets.
pub fn convert_words_to_csv(pages: &[PageOcrResult]) -> String {
    let mut out = String::new();

    for (page_idx, page) in pages.iter().enumerate() {
        if page.words.is_empty() {
            continue;
        }
        if pages.len() > 1 {
            out.push_str(&format!("--- Page {} ---\n", page_idx + 1));
        }

        let mut sorted_by_y: Vec<&WordBox> = page.words.iter().collect();
        sorted_by_y.sort_by(|a, b| {
            let ay = a.y + a.h * 0.5;
            let by = b.y + b.h * 0.5;
            ay.partial_cmp(&by).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut rows: Vec<Vec<&WordBox>> = Vec::new();
        for word in sorted_by_y {
            let word_cy = word.y + word.h * 0.5;
            let tolerance = (word.h * 0.6).max(12.0);
            if let Some(last_row) = rows.last_mut() {
                let avg_y: f32 =
                    last_row.iter().map(|w| w.y + w.h * 0.5).sum::<f32>() / (last_row.len() as f32);
                if (word_cy - avg_y).abs() <= tolerance {
                    last_row.push(word);
                    continue;
                }
            }
            rows.push(vec![word]);
        }

        for mut row in rows {
            row.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
            let mut cells: Vec<String> = Vec::new();
            let mut current_cell = String::new();
            let mut last_right: Option<f32> = None;

            for w in row {
                let space_threshold = (w.h * 1.35).max(22.0);
                if let Some(lr) = last_right {
                    if (w.x - lr) > space_threshold {
                        cells.push(escape_csv_cell(current_cell.trim()));
                        current_cell.clear();
                    } else if !current_cell.is_empty() {
                        current_cell.push(' ');
                    }
                }
                current_cell.push_str(&w.text);
                last_right = Some(w.x + w.w);
            }
            if !current_cell.is_empty() {
                cells.push(escape_csv_cell(current_cell.trim()));
            }
            out.push_str(&cells.join(","));
            out.push('\n');
        }
    }

    out.trim().to_string()
}

fn escape_csv_cell(val: &str) -> String {
    let escaped = val.replace('"', "\"\"");
    if escaped.contains(',') || escaped.contains('"') || escaped.contains('\n') {
        format!("\"{}\"", escaped)
    } else {
        escaped
    }
}

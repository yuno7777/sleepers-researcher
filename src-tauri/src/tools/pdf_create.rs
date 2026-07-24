//! PDF creation tool. Renders a title + body text into a paginated A4 PDF using
//! a self-contained Rust PDF engine (no external binaries). Writing a file is a
//! mutating action, so it goes through the confirmation modal.

use super::{activity, request_permission, Tool};
use anyhow::{anyhow, Result};
use printpdf::{BuiltinFont, Mm, PdfDocument};
use serde_json::{json, Value};
use std::fs::File;
use std::io::BufWriter;
use tauri::AppHandle;

pub struct CreatePdfTool;

#[async_trait::async_trait]
impl Tool for CreatePdfTool {
    fn name(&self) -> &'static str {
        "create_pdf"
    }
    fn description(&self) -> &'static str {
        "Create a PDF from a title and body text (plain text / simple markdown). \
Provide 'path' (…\\file.pdf), 'title', and 'content'. Requires confirmation."
    }
    fn args_hint(&self) -> Value {
        json!({ "path": "output .pdf path", "title": "document title", "content": "body text" })
    }
    fn mutating(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, app: &AppHandle) -> Result<String> {
        let path = args["path"].as_str().ok_or_else(|| anyhow!("missing 'path'"))?;
        let title = args["title"].as_str().unwrap_or("Document");
        let content = args["content"].as_str().ok_or_else(|| anyhow!("missing 'content'"))?;

        let preview = format!(
            "PDF -> {path}\ntitle: {title}\n\n{}",
            if content.len() > 1200 { &content[..1200] } else { content }
        );
        let approved =
            request_permission(app, "file_write", "Create PDF", path, &preview).await;
        if !approved {
            return Ok(format!("DENIED: user declined to create {path}."));
        }

        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        generate_pdf(path, title, content)?;
        activity(app, "pdf", path.to_string());
        Ok(format!("Created PDF at {path}."))
    }
}

fn sanitize(s: &str) -> String {
    // Builtin PDF fonts are Latin-1; map anything outside to a safe char.
    s.chars().map(|c| if (c as u32) < 256 { c } else { '?' }).collect()
}

fn wrap(line: &str, width: usize) -> Vec<String> {
    if line.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in line.split_whitespace() {
        if cur.len() + word.len() + 1 > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub(crate) fn generate_pdf(path: &str, title: &str, content: &str) -> Result<()> {
    let title = sanitize(title);
    let content = sanitize(content);

    let (doc, page1, layer1) = PdfDocument::new(title.as_str(), Mm(210.0), Mm(297.0), "Layer 1");
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;
    let mut layer = doc.get_page(page1).get_layer(layer1);

    let left = 20.0_f32;
    let top = 277.0_f32;
    let bottom = 20.0_f32;
    let mut y = top;

    layer.use_text(title.as_str(), 18.0, Mm(left), Mm(y), &bold);
    y -= 12.0;

    for raw in content.split('\n') {
        let wrapped = wrap(raw, 95);
        if wrapped.is_empty() {
            y -= 5.0; // blank line
            if y < bottom {
                let (p, l) = doc.add_page(Mm(210.0), Mm(297.0), "Layer");
                layer = doc.get_page(p).get_layer(l);
                y = top;
            }
            continue;
        }
        for w in wrapped {
            if y < bottom {
                let (p, l) = doc.add_page(Mm(210.0), Mm(297.0), "Layer");
                layer = doc.get_page(p).get_layer(l);
                y = top;
            }
            layer.use_text(w, 11.0, Mm(left), Mm(y), &font);
            y -= 5.5;
        }
    }

    doc.save(&mut BufWriter::new(File::create(path)?))?;
    Ok(())
}

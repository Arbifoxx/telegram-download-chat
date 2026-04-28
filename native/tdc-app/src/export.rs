use chrono::{DateTime, Datelike, FixedOffset, Local};
use html_escape::encode_safe;
use minijinja::{context, Environment};
use printpdf::{BuiltinFont, Mm, PdfDocument};
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

const AVATAR_COLORS: [&str; 6] = [
    "#c03d33",
    "#4fad2d",
    "#d09306",
    "#168acd",
    "#8544d6",
    "#cd4073",
];

const HTML_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{{ chat_title }}</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,'Helvetica Neue',Arial,sans-serif;background:#eae6da;color:#111;font-size:14px;line-height:1.45;min-height:100vh}
a{color:#168acd;text-decoration:none}
a:hover{text-decoration:underline}
.wrap{max-width:900px;margin:0 auto;display:flex;flex-direction:column;min-height:100vh;background:inherit}
.hdr{background:#fff;border-bottom:1px solid #ddd;padding:12px 16px;display:flex;align-items:center;gap:12px;position:sticky;top:0;z-index:10;box-shadow:0 1px 3px rgba(0,0,0,0.08)}
.hdr-av{width:42px;height:42px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-weight:700;font-size:18px;color:#fff;background:#168acd;flex-shrink:0}
.hdr-info .name{font-weight:600;font-size:16px}
.hdr-info .sub{font-size:12px;color:#999;margin-top:2px}
.msgs{flex:1;padding:16px 12px 32px;display:flex;flex-direction:column;gap:1px}
.datesep{text-align:center;margin:14px 0 10px;user-select:none}
.datesep span{background:rgba(0,0,0,0.22);color:#fff;border-radius:14px;padding:5px 14px;font-size:12px;font-weight:500}
.svc{text-align:center;margin:8px auto;user-select:none}
.svc span{background:rgba(0,0,0,0.16);color:#fff;border-radius:12px;padding:4px 12px;font-size:12px;display:inline-block}
.grp{display:flex;align-items:flex-end;gap:6px;margin-bottom:2px;max-width:100%}
.grp.out{flex-direction:row-reverse}
.av{width:34px;height:34px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-weight:700;font-size:13px;color:#fff;flex-shrink:0;align-self:flex-end}
.av-ph{width:34px;flex-shrink:0}
.bubbles{display:flex;flex-direction:column;gap:2px;max-width:72%;min-width:80px}
.grp.out .bubbles{align-items:flex-end}
.sname{font-size:13px;font-weight:600;margin-bottom:3px;padding-left:14px}
.bbl{background:#fff;border-radius:18px;padding:8px 12px 6px;box-shadow:0 1px 2px rgba(0,0,0,0.14);position:relative;word-break:break-word;max-width:100%}
.grp.out .bbl{background:#d9fdd3}
.grp:not(.out) .bbl{border-bottom-left-radius:5px}
.grp:not(.out) .bbl:first-child{border-top-left-radius:18px}
.grp:not(.out) .bbl:last-child{border-bottom-left-radius:18px}
.grp.out .bbl{border-bottom-right-radius:5px}
.grp.out .bbl:first-child{border-top-right-radius:18px}
.grp.out .bbl:last-child{border-bottom-right-radius:18px}
.grp:not(.out) .bbl:last-child::before{content:'';position:absolute;bottom:8px;left:-7px;border:7px solid transparent;border-right-color:#fff;border-left-width:0}
.grp.out .bbl:last-child::before{content:'';position:absolute;bottom:8px;right:-7px;border:7px solid transparent;border-left-color:#d9fdd3;border-right-width:0}
.fwd{border-left:3px solid #00a884;padding-left:8px;margin-bottom:6px;color:#00a884;font-size:13px;font-weight:600}
.rq{background:rgba(0,0,0,0.05);border-left:3px solid #00a884;border-radius:6px;padding:5px 8px;margin-bottom:6px;font-size:12.5px;color:#555;display:-webkit-box;-webkit-line-clamp:3;-webkit-box-orient:vertical;overflow:hidden;max-height:64px}
.media-img,.media-stk,.media-vid{display:block;border-radius:10px;margin-bottom:4px}
.media-img{max-width:100%;max-height:340px;object-fit:contain}
.media-stk{max-width:160px;max-height:160px}
.media-vid{max-width:100%;max-height:280px;width:100%}
.media-aud{width:260px;max-width:100%;margin-bottom:4px;display:block}
.media-file{display:flex;align-items:center;gap:10px;background:rgba(0,0,0,0.04);text-decoration:none;color:inherit;border-radius:10px;padding:8px 10px;margin-bottom:4px}
.media-file.missing{opacity:.9}
.media-file .ico{font-size:26px;line-height:1;flex-shrink:0}
.media-file .meta{display:flex;flex-direction:column;align-items:flex-start;gap:1px;flex:1;min-width:0;margin-top:0}
.media-file .fname{font-size:13px;font-weight:500;color:#333;word-break:break-all}
.media-file .sub{font-size:12px;color:#6d7b88}
.media-loc{margin-bottom:4px;font-size:13px}
.txt{white-space:pre-wrap;font-size:14px;color:#111}
.meta-line{display:flex;justify-content:flex-end;align-items:center;gap:5px;font-size:11px;color:#8a8a8a;margin-top:4px;user-select:none}
.meta-line .edited{font-style:italic}
.meta-line .tick{color:#53bdeb;font-size:13px}
</style>
</head>
<body>
<div class="wrap">
<div class="hdr">
  <div class="hdr-av">{{ header_initial }}</div>
  <div class="hdr-info">
    <div class="name">{{ chat_title }}</div>
    <div class="sub">{{ message_count }} messages &middot; Exported by Telegram Download Chat</div>
  </div>
</div>
<div class="msgs">
{% for item in items %}
{% if item.type == "date_sep" %}
<div class="datesep"><span>{{ item.label }}</span></div>
{% elif item.type == "service" %}
<div class="svc"><span>{{ item.text }}</span></div>
{% elif item.type == "group" %}
<div class="grp{% if item.is_outgoing %} out{% endif %}">
  {% if not item.is_outgoing %}
  <div class="av" style="background:{{ item.sender_color }}">{{ item.initials }}</div>
  {% else %}
  <div class="av-ph"></div>
  {% endif %}
  <div class="bubbles">
    {% if not item.is_outgoing %}
    <div class="sname" style="color:{{ item.sender_color }}">{{ item.sender_name }}</div>
    {% endif %}
    {% for msg in item.messages %}
    <div class="bbl">
      {% if msg.fwd_from_name %}
      <div class="fwd">&#8627; Forwarded from {{ msg.fwd_from_name }}</div>
      {% endif %}
      {% if msg.reply_text %}
      <div class="rq">{{ msg.reply_text }}</div>
      {% endif %}
      {% if msg.media_kind == "image" and msg.media_src %}
      <img class="media-img" src="{{ msg.media_src | urlencode_path }}" alt="" loading="lazy">
      {% elif msg.media_kind == "sticker" and msg.media_src %}
      <img class="media-stk" src="{{ msg.media_src | urlencode_path }}" alt="sticker" loading="lazy">
      {% elif msg.media_kind == "video" and msg.media_src %}
      <video class="media-vid" controls preload="none" src="{{ msg.media_src | urlencode_path }}"></video>
      {% elif msg.media_kind == "audio" and msg.media_src %}
      <audio class="media-aud" controls preload="none" src="{{ msg.media_src | urlencode_path }}"></audio>
      {% elif msg.media_kind == "location" and msg.location_url %}
      <div class="media-loc">&#128205; <a href="{{ msg.location_url }}" target="_blank" rel="noopener">View on map</a></div>
      {% elif msg.media_kind %}
      <div class="media-file{% if not msg.media_src %} missing{% endif %}">
        <div class="ico">{{ msg.media_icon }}</div>
        <div class="meta">
          <div class="fname">{{ msg.attachment_filename }}</div>
          <div class="sub">{{ msg.media_subtitle }}</div>
        </div>
      </div>
      {% endif %}
      {% if msg.text %}
      <div class="txt">{{ msg.text }}</div>
      {% endif %}
      <div class="meta-line">
        {% if msg.edited %}<span class="edited">edited</span>{% endif %}
        <span>{{ msg.time }}</span>
        {% if item.is_outgoing %}<span class="tick">&#10003;&#10003;</span>{% endif %}
      </div>
    </div>
    {% endfor %}
  </div>
</div>
{% endif %}
{% endfor %}
</div>
</div>
</body>
</html>
"#;

#[derive(Clone, Serialize)]
#[serde(tag = "type")]
enum HtmlItem {
    #[serde(rename = "date_sep")]
    DateSep { label: String },
    #[serde(rename = "service")]
    Service { text: String },
    #[serde(rename = "group")]
    Group {
        is_outgoing: bool,
        sender_name: String,
        sender_color: String,
        initials: String,
        messages: Vec<HtmlMessage>,
    },
}

#[derive(Clone, Serialize)]
struct HtmlMessage {
    text: String,
    time: String,
    edited: bool,
    reply_text: Option<String>,
    fwd_from_name: Option<String>,
    media_kind: Option<String>,
    media_src: Option<String>,
    media_icon: Option<String>,
    media_subtitle: Option<String>,
    attachment_filename: Option<String>,
    location_url: Option<String>,
}

pub async fn render_native_exports(
    messages: &[Value],
    attachments_dir: &Path,
    chat_title: &str,
    html_path: Option<&Path>,
    pdf_path: Option<&Path>,
) -> Result<(), String> {
    if html_path.is_none() && pdf_path.is_none() {
        return Ok(());
    }

    let messages = messages.to_vec();
    let attachments_dir = attachments_dir.to_path_buf();
    let chat_title = chat_title.to_string();
    let html_path = html_path.map(Path::to_path_buf);
    let pdf_path = pdf_path.map(Path::to_path_buf);

    tokio::task::spawn_blocking(move || {
        if let Some(path) = html_path.as_deref() {
            render_html(&messages, path, &attachments_dir, &chat_title)?;
        }
        if let Some(path) = pdf_path.as_deref() {
            render_pdf(&messages, path, &chat_title)?;
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| format!("Export task failed: {error}"))?
}

fn render_html(
    messages: &[Value],
    output_file: &Path,
    attachments_dir: &Path,
    chat_title: &str,
) -> Result<(), String> {
    let media_prefix = relative_media_prefix(output_file, attachments_dir);
    let items = preprocess_messages(messages, attachments_dir);
    let mut env = Environment::new();
    env.add_filter("urlencode_path", |value: String| encode_path(&value));
    let template = env
        .template_from_str(HTML_TEMPLATE)
        .map_err(|error| format!("Failed to build HTML template: {error}"))?;
    let header_initial = chat_title
        .chars()
        .next()
        .map(|ch| ch.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    let html = template
        .render(context! {
            chat_title => chat_title,
            header_initial => header_initial,
            message_count => messages.len(),
            items => items,
            media_prefix => media_prefix,
        })
        .map_err(|error| format!("Failed to render HTML: {error}"))?;
    std::fs::create_dir_all(
        output_file
            .parent()
            .ok_or_else(|| "Invalid HTML output path".to_string())?,
    )
    .map_err(|error| format!("Failed to prepare HTML output directory: {error}"))?;
    std::fs::write(output_file, html)
        .map_err(|error| format!("Failed to write {}: {error}", output_file.display()))
}

fn render_pdf(messages: &[Value], output_file: &Path, chat_title: &str) -> Result<(), String> {
    let sorted = sorted_messages(messages);
    let (doc, page1, layer1) =
        PdfDocument::new(chat_title, Mm(210.0), Mm(297.0), "Messages");
    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|error| format!("Failed to load PDF font: {error}"))?;
    let bold = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .map_err(|error| format!("Failed to load PDF bold font: {error}"))?;

    let mut current_page = page1;
    let mut current_layer = layer1;
    let mut y = 282.0f64;
    let left = 14.0f64;
    let line_height = 6.0f64;

    {
        let layer = doc.get_page(current_page).get_layer(current_layer);
        layer.use_text(chat_title, 20.0, Mm(left as f32), Mm(y as f32), &bold);
    }
    y -= 12.0;

    let mut last_date = None::<String>;
    for message in sorted {
        let date_raw = message.get("date").and_then(Value::as_str).unwrap_or_default();
        let date_sep = format_date_separator(date_raw);
        if last_date.as_deref() != Some(date_sep.as_str()) {
            ensure_pdf_space(&doc, &mut current_page, &mut current_layer, &mut y, 10.0);
            let layer = doc.get_page(current_page).get_layer(current_layer);
            layer.use_text(date_sep.clone(), 11.0, Mm(left as f32), Mm(y as f32), &bold);
            y -= 8.0;
            last_date = Some(date_sep);
        }

        let line = if let Some(service) = service_text(&message) {
            service
        } else {
            let date = format_time_only(date_raw);
            let sender = message
                .get("user_display_name")
                .and_then(Value::as_str)
                .unwrap_or("Unknown");
            let mut body = message
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if body.trim().is_empty() {
                body = media_placeholder(&message).unwrap_or_default();
            } else if let Some(placeholder) = media_placeholder(&message) {
                body.push(' ');
                body.push_str(&placeholder);
            }
            format!("[{date}] {sender}: {body}")
        };

        for wrapped in wrap_text(&line, 92) {
            ensure_pdf_space(&doc, &mut current_page, &mut current_layer, &mut y, line_height);
            let layer = doc.get_page(current_page).get_layer(current_layer);
            layer.use_text(wrapped, 10.5, Mm(left as f32), Mm(y as f32), &font);
            y -= line_height;
        }
        y -= 2.0;
    }

    std::fs::create_dir_all(
        output_file
            .parent()
            .ok_or_else(|| "Invalid PDF output path".to_string())?,
    )
    .map_err(|error| format!("Failed to prepare PDF output directory: {error}"))?;
    let file = File::create(output_file)
        .map_err(|error| format!("Failed to create {}: {error}", output_file.display()))?;
    doc.save(&mut BufWriter::new(file))
        .map_err(|error| format!("Failed to save PDF: {error}"))
}

fn ensure_pdf_space(
    doc: &printpdf::PdfDocumentReference,
    current_page: &mut printpdf::PdfPageIndex,
    current_layer: &mut printpdf::PdfLayerIndex,
    y: &mut f64,
    needed: f64,
) {
    if *y >= 18.0 + needed {
        return;
    }
    let (page, layer) = doc.add_page(Mm(210.0), Mm(297.0), "Messages");
    *current_page = page;
    *current_layer = layer;
    *y = 282.0;
}

fn relative_media_prefix(output_file: &Path, attachments_dir: &Path) -> String {
    match attachments_dir.strip_prefix(output_file.parent().unwrap_or(Path::new("."))) {
        Ok(path) => {
            let raw = path.to_string_lossy().replace('\\', "/");
            if raw.is_empty() {
                "attachments/".to_string()
            } else {
                format!("{raw}/")
            }
        }
        Err(_) => "attachments/".to_string(),
    }
}

fn preprocess_messages(messages: &[Value], attachments_dir: &Path) -> Vec<HtmlItem> {
    let mut items = Vec::new();
    let sorted = sorted_messages(messages);
    let mut current_group: Option<(bool, i64, String, String, String, Vec<HtmlMessage>)> = None;
    let mut prev_date: Option<String> = None;
    let mut prev_sender_id: Option<i64> = None;
    let mut prev_dt: Option<DateTime<FixedOffset>> = None;

    for message in sorted {
        let date_raw = message.get("date").and_then(Value::as_str).unwrap_or_default();
        let dt = parse_dt(date_raw);
        let local_day = dt
            .as_ref()
            .map(|value| value.with_timezone(&Local).format("%Y-%m-%d").to_string());

        if local_day != prev_date {
            if let Some(group) = current_group.take() {
                items.push(HtmlItem::Group {
                    is_outgoing: group.0,
                    sender_name: group.2,
                    sender_color: group.3,
                    initials: group.4,
                    messages: group.5,
                });
            }
            if let Some(day) = local_day.clone() {
                items.push(HtmlItem::DateSep {
                    label: format_date_separator(date_raw),
                });
                prev_date = Some(day);
            }
        }

        if let Some(service) = service_text(&message) {
            if let Some(group) = current_group.take() {
                items.push(HtmlItem::Group {
                    is_outgoing: group.0,
                    sender_name: group.2,
                    sender_color: group.3,
                    initials: group.4,
                    messages: group.5,
                });
            }
            items.push(HtmlItem::Service { text: service });
            prev_sender_id = None;
            prev_dt = None;
            continue;
        }

        let sender_id = sender_id(&message);
        let is_outgoing = message.get("out").and_then(Value::as_bool).unwrap_or(false);
        let sender_name = message
            .get("user_display_name")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_string();
        let sender_color = sender_color(&sender_name);
        let initials = sender_initials(&sender_name);
        let same_group = current_group.is_some()
            && prev_sender_id == Some(sender_id)
            && prev_dt.as_ref().zip(dt.as_ref()).map(|(a, b)| (b.timestamp() - a.timestamp()).abs() < 120).unwrap_or(false);

        if !same_group {
            if let Some(group) = current_group.take() {
                items.push(HtmlItem::Group {
                    is_outgoing: group.0,
                    sender_name: group.2,
                    sender_color: group.3,
                    initials: group.4,
                    messages: group.5,
                });
            }
            current_group = Some((
                is_outgoing,
                sender_id,
                sender_name.clone(),
                sender_color,
                initials,
                Vec::new(),
            ));
        }

        let entry = build_html_message(&message, attachments_dir);
        if let Some(group) = current_group.as_mut() {
            group.5.push(entry);
        }
        prev_sender_id = Some(sender_id);
        prev_dt = dt;
    }

    if let Some(group) = current_group.take() {
        items.push(HtmlItem::Group {
            is_outgoing: group.0,
            sender_name: group.2,
            sender_color: group.3,
            initials: group.4,
            messages: group.5,
        });
    }

    items
}

fn build_html_message(message: &Value, attachments_dir: &Path) -> HtmlMessage {
    let meta = attachment_meta(message, attachments_dir);
    let reply_text = message
        .get("reply_to")
        .and_then(Value::as_object)
        .and_then(|reply| reply.get("quote_text"))
        .and_then(Value::as_str)
        .map(|value| value.chars().take(150).collect::<String>());
    let fwd_from_name = message
        .get("fwd_from")
        .and_then(Value::as_object)
        .and_then(|forward| forward.get("from_name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    HtmlMessage {
        text: safe_text(
            message
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        time: format_time_only(
            message.get("date").and_then(Value::as_str).unwrap_or_default(),
        ),
        edited: message.get("edit_date").is_some() && !message.get("edit_date").unwrap().is_null(),
        reply_text,
        fwd_from_name,
        media_kind: meta.media_kind,
        media_src: meta.media_src,
        media_icon: meta.media_icon,
        media_subtitle: meta.media_subtitle,
        attachment_filename: meta.attachment_filename,
        location_url: meta.location_url,
    }
}

#[derive(Default)]
struct AttachmentMeta {
    media_kind: Option<String>,
    media_src: Option<String>,
    media_icon: Option<String>,
    media_subtitle: Option<String>,
    attachment_filename: Option<String>,
    location_url: Option<String>,
}

fn attachment_meta(message: &Value, attachments_dir: &Path) -> AttachmentMeta {
    let mut result = AttachmentMeta::default();
    let attachment_path = message
        .get("attachment_path")
        .and_then(Value::as_str)
        .map(str::to_string);
    let attachment_filename = attachment_path
        .as_deref()
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .map(str::to_string);
    result.attachment_filename = attachment_filename.clone();
    let downloaded = attachment_path
        .as_ref()
        .map(|path| attachments_dir.join(path).exists())
        .unwrap_or(false);
    if downloaded {
        result.media_src = attachment_path.clone();
    }

    let media = message.get("media").and_then(Value::as_object);
    let media_type = media
        .and_then(|media| media.get("_"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    match media_type {
        "MessageMediaPhoto" => {
            result.media_kind = Some("image".to_string());
            result.attachment_filename = result
                .attachment_filename
                .or_else(|| Some("Image".to_string()));
            result.media_icon = Some("🖼".to_string());
            result.media_subtitle = Some(if downloaded {
                "Image".to_string()
            } else {
                "Image not downloaded".to_string()
            });
        }
        "MessageMediaDocument" => {
            let (filename, category, size_label) = document_meta(message, media);
            result.attachment_filename = filename.clone().or(result.attachment_filename);
            result.media_subtitle = Some(match (downloaded, size_label.as_deref()) {
                (true, Some(size)) => format!("{size} · Download"),
                (true, None) => "Download".to_string(),
                (false, Some(size)) => format!("{size} · Attachment not downloaded"),
                (false, None) => "Attachment not downloaded".to_string(),
            });
            let (kind, icon) = match category.as_deref() {
                Some("stickers") => ("sticker", "🖼"),
                Some("videos") => ("video", "🎥"),
                Some("audio") => ("audio", "🎧"),
                Some("archives") => ("file", "🛠"),
                _ => ("file", "📄"),
            };
            result.media_kind = Some(kind.to_string());
            result.media_icon = Some(icon.to_string());
        }
        "MessageMediaContact" => {
            result.media_kind = Some("file".to_string());
            result.media_icon = Some("👤".to_string());
            result.media_subtitle = Some(if downloaded {
                "Contact".to_string()
            } else {
                "Contact not downloaded".to_string()
            });
        }
        "MessageMediaGeo" | "MessageMediaGeoLive" | "MessageMediaVenue" => {
            let filename = result.attachment_filename.clone().unwrap_or_default();
            let stem = filename.strip_suffix(".json").unwrap_or(&filename);
            let parts = stem.rsplitn(3, '_').collect::<Vec<_>>();
            if parts.len() >= 2 {
                let lng = parts[0];
                let lat = parts[1];
                result.media_kind = Some("location".to_string());
                result.location_url = Some(format!("https://maps.google.com/?q={lat},{lng}"));
            }
        }
        "MessageMediaPoll" => {
            result.media_kind = Some("file".to_string());
            result.media_icon = Some("📊".to_string());
            result.media_subtitle = Some("Poll".to_string());
        }
        _ => {}
    }

    result
}

fn document_meta(
    message: &Value,
    media: Option<&serde_json::Map<String, Value>>,
) -> (Option<String>, Option<String>, Option<String>) {
    let mut filename = message
        .get("attachment_path")
        .and_then(Value::as_str)
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .map(str::to_string);
    let mut category = None::<String>;
    let mut size = None::<u64>;

    if let Some(document) = media
        .and_then(|media| media.get("document"))
        .and_then(Value::as_object)
    {
        size = document.get("size").and_then(Value::as_u64);
        if let Some(attributes) = document.get("attributes").and_then(Value::as_array) {
            let mut has_sticker = false;
            let mut has_audio = false;
            let mut has_video = false;
            for attr in attributes {
                let Some(attr) = attr.as_object() else { continue };
                match attr.get("_").and_then(Value::as_str).unwrap_or_default() {
                    "DocumentAttributeFilename" => {
                        if filename.is_none() {
                            filename = attr
                                .get("file_name")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                    }
                    "DocumentAttributeSticker" => has_sticker = true,
                    "DocumentAttributeAudio" => has_audio = true,
                    "DocumentAttributeVideo" => has_video = true,
                    _ => {}
                }
            }
            category = Some(if has_sticker {
                "stickers".to_string()
            } else if has_audio {
                "audio".to_string()
            } else if has_video {
                "videos".to_string()
            } else {
                let ext = filename
                    .as_deref()
                    .and_then(|name| Path::new(name).extension())
                    .and_then(|ext| ext.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "ipsw"].contains(&ext.as_str()) {
                    "archives".to_string()
                } else {
                    "documents".to_string()
                }
            });
        }
    }

    (filename, category, format_size(size))
}

fn sorted_messages(messages: &[Value]) -> Vec<Value> {
    let mut values = messages.to_vec();
    values.sort_by(|a, b| compare_message_dates(a, b));
    values
}

fn compare_message_dates(a: &Value, b: &Value) -> Ordering {
    let a_dt = a.get("date").and_then(Value::as_str).and_then(parse_dt);
    let b_dt = b.get("date").and_then(Value::as_str).and_then(parse_dt);
    a_dt.cmp(&b_dt)
}

fn parse_dt(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

fn format_time_only(value: &str) -> String {
    parse_dt(value)
        .map(|dt| dt.with_timezone(&Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

fn format_date_separator(value: &str) -> String {
    parse_dt(value)
        .map(|dt| {
            let local = dt.with_timezone(&Local);
            format!("{} {}, {}", local.format("%B"), local.day(), local.format("%Y"))
        })
        .unwrap_or_else(|| value.to_string())
}

fn service_text(message: &Value) -> Option<String> {
    let action = message.get("action")?.as_object()?;
    let action_name = action.get("_")?.as_str()?;
    let sender = message
        .get("user_display_name")
        .and_then(Value::as_str)
        .unwrap_or("Someone");
    let label = match action_name {
        "MessageActionChatAddUser" => "joined the group".to_string(),
        "MessageActionChatDeleteUser" => "left the group".to_string(),
        "MessageActionChatJoinedByLink" => "joined via invite link".to_string(),
        "MessageActionChatEditTitle" => {
            if let Some(title) = action.get("title").and_then(Value::as_str) {
                format!("changed the group name to “{title}”")
            } else {
                "changed the group name".to_string()
            }
        }
        "MessageActionChatEditPhoto" => "updated the group photo".to_string(),
        "MessageActionChatCreate" => "created the group".to_string(),
        "MessageActionPinMessage" => "pinned a message".to_string(),
        "MessageActionChatMigrateTo" => "group was upgraded to a supergroup".to_string(),
        "MessageActionChannelCreate" => "created the channel".to_string(),
        "MessageActionPhoneCall" => "Phone call".to_string(),
        "MessageActionGroupCall" => "Group call".to_string(),
        "MessageActionInviteToGroupCall" => "was invited to a voice chat".to_string(),
        "MessageActionContactSignUp" => "joined Telegram".to_string(),
        "MessageActionHistoryClear" => "cleared the history".to_string(),
        "MessageActionSetMessagesTTL" => "changed the auto-delete timer".to_string(),
        "MessageActionScreenshotTaken" => "took a screenshot".to_string(),
        _ => return None,
    };
    Some(format!("{sender} {label}"))
}

fn media_placeholder(message: &Value) -> Option<String> {
    let media = message.get("media")?.as_object()?;
    match media.get("_").and_then(Value::as_str).unwrap_or_default() {
        "MessageMediaPhoto" => Some("[photo]".to_string()),
        "MessageMediaDocument" => {
            let filename = message
                .get("attachment_path")
                .and_then(Value::as_str)
                .map(Path::new)
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .or_else(|| {
                    media.get("document")
                        .and_then(Value::as_object)
                        .and_then(|document| document.get("attributes"))
                        .and_then(Value::as_array)
                        .and_then(|attrs| {
                            attrs.iter().find_map(|attr| {
                                let attr = attr.as_object()?;
                                if attr.get("_").and_then(Value::as_str)
                                    == Some("DocumentAttributeFilename")
                                {
                                    attr.get("file_name").and_then(Value::as_str)
                                } else {
                                    None
                                }
                            })
                        })
                })
                .unwrap_or("attachment");
            Some(format!("[file={filename}]"))
        }
        "MessageMediaContact" => Some("[contact]".to_string()),
        "MessageMediaGeo" | "MessageMediaGeoLive" | "MessageMediaVenue" => Some("[location]".to_string()),
        "MessageMediaPoll" => Some("[poll]".to_string()),
        _ => Some("[media]".to_string()),
    }
}

fn sender_id(message: &Value) -> i64 {
    let Some(from_id) = message.get("from_id") else {
        return 0;
    };
    if let Some(value) = from_id.get("user_id").and_then(Value::as_i64) {
        value
    } else if let Some(value) = from_id.get("channel_id").and_then(Value::as_i64) {
        value
    } else if let Some(value) = from_id.get("chat_id").and_then(Value::as_i64) {
        value
    } else {
        0
    }
}

fn sender_color(name: &str) -> String {
    let mut hash = 0u64;
    for byte in name.as_bytes() {
        hash = hash.wrapping_mul(16777619).wrapping_add(*byte as u64 + 2166136261);
    }
    AVATAR_COLORS[(hash as usize) % AVATAR_COLORS.len()].to_string()
}

fn sender_initials(name: &str) -> String {
    let parts = name.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() {
        return "?".to_string();
    }
    let first = parts.first().and_then(|part| part.chars().next()).unwrap_or('?');
    let last = if parts.len() > 1 {
        parts.last().and_then(|part| part.chars().next()).unwrap_or(first)
    } else {
        '\0'
    };
    if last == '\0' {
        first.to_uppercase().to_string()
    } else {
        format!("{}{}", first.to_ascii_uppercase(), last.to_ascii_uppercase())
    }
}

fn format_size(bytes: Option<u64>) -> Option<String> {
    let bytes = bytes?;
    if bytes == 0 {
        return None;
    }
    let size = bytes as f64;
    if size >= 1024.0 * 1024.0 * 1024.0 {
        Some(format!("{:.1} GB", size / (1024.0 * 1024.0 * 1024.0)))
    } else if size >= 1024.0 * 1024.0 {
        Some(format!("{:.1} MB", size / (1024.0 * 1024.0)))
    } else if size >= 1024.0 {
        Some(format!("{:.1} KB", size / 1024.0))
    } else {
        Some(format!("{bytes} B"))
    }
}

fn encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let next_len = if current.is_empty() {
                word.len()
            } else {
                current.len() + 1 + word.len()
            };
            if next_len > max_chars && !current.is_empty() {
                lines.push(current);
                current = word.to_string();
            } else if current.is_empty() {
                current = word.to_string();
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        } else {
            lines.push(String::new());
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn safe_text(value: &str) -> String {
    encode_safe(value).into_owned()
}

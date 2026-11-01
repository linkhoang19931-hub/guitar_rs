//! Đọc file MusicXML (score-partwise) thành các nốt tab theo thời gian tính bằng phách đen (quarter).
//!
//! Chỉ lấy những part có thông tin tab (`<technical><string>` + `<fret>`). Dấu nhắc lại (repeat) và khung
//! kết (ending / volta) được trải phẳng theo thứ tự chơi thật. Nốt nối (tie) được gộp thành một nốt dài.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use roxmltree::{Document, Node};

/// Lỗi khi đọc MusicXML: chỉ là một câu mô tả.
#[derive(Debug, Clone)]
pub struct XmlError(pub String);

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for XmlError {}

pub type Res<T> = Result<T, XmlError>;

fn err<T>(msg: impl Into<String>) -> Res<T> {
    Err(XmlError(msg.into()))
}

/// Các kỹ thuật chơi đọc được từ MusicXML (tên khoá giống feedpak).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marks {
    pub hammer: bool,
    pub slide: bool,
    pub bend: f64,
    pub vibrato: bool,
    pub palm_mute: bool,
    pub dead: bool,
    pub harmonic: bool,
    pub accent: bool,
    pub tap: bool,
    pub tremolo: bool,
}

/// Một nốt tab sau khi đã trải phẳng dấu nhắc lại. Thời gian tính bằng phách đen từ đầu bài (chưa có nhịp đếm vào).
#[derive(Clone, Debug, PartialEq)]
pub struct TabNote {
    pub q: f64,
    pub dur_q: f64,
    /// Số dây kiểu Rocksmith: 0 = dây trầm nhất.
    pub string: i32,
    pub fret: i32,
    /// Phím đích khi vuốt (slide), -1 nếu không vuốt.
    pub slide_to: i32,
    pub marks: Marks,
}

/// Một part (bè) có tab.
#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    pub is_bass: bool,
    /// Cao độ MIDI của từng dây buông, dây trầm trước.
    pub open_midi: Vec<i32>,
    pub capo: i32,
    pub notes: Vec<TabNote>,
    /// Tên hợp âm ghi trong bài (`<harmony>`), theo vị trí phách đen.
    pub chord_names: Vec<(f64, String)>,
}

/// Một ô nhịp đã trải phẳng.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub q: f64,
    pub len_q: f64,
    /// Số phách trong ô (theo số chỉ nhịp) và độ dài một phách tính bằng phách đen.
    pub beats: i32,
    pub beat_q: f64,
}

#[derive(Clone, Debug)]
pub struct Score {
    pub title: String,
    pub artist: String,
    pub parts: Vec<Part>,
    pub bars: Vec<Bar>,
    /// Đổi tốc độ: (vị trí phách đen, BPM theo phách đen).
    pub tempos: Vec<(f64, f64)>,
    /// Đánh dấu đoạn (rehearsal mark): (vị trí, tên).
    pub sections: Vec<(f64, String)>,
    pub end_q: f64,
}

// ---------------------------------------------------------------------------------------------------------------
// Mở file

/// Đọc văn bản XML từ `.musicxml` / `.xml`, hoặc từ gói nén `.mxl`.
pub fn read_text(path: &Path) -> Res<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let bytes = std::fs::read(path).map_err(|e| XmlError(format!("{}: {e}", path.display())))?;
    if ext == "mxl" || bytes.starts_with(b"PK") {
        return read_mxl(&bytes);
    }
    decode_text(bytes)
}

fn decode_text(bytes: Vec<u8>) -> Res<String> {
    // UTF-16 có BOM (một số phần mềm xuất như vậy)
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    let s = String::from_utf8_lossy(&bytes).into_owned();
    Ok(s.trim_start_matches('\u{feff}').to_string())
}

fn read_mxl(bytes: &[u8]) -> Res<String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| XmlError(format!("không mở được gói .mxl: {e}")))?;
    let mut read = |name: &str| -> Option<Vec<u8>> {
        let mut f = zip.by_name(name).ok()?;
        let mut v = Vec::new();
        f.read_to_end(&mut v).ok()?;
        Some(v)
    };
    // META-INF/container.xml chỉ ra file bản nhạc chính
    let mut main: Option<String> = None;
    if let Some(c) = read("META-INF/container.xml")
        && let Ok(text) = decode_text(c)
        && let Ok(doc) = Document::parse_with_options(&text, xml_options())
    {
        main = doc
            .descendants()
            .find(|n| n.has_tag_name("rootfile"))
            .and_then(|n| n.attribute("full-path"))
            .map(str::to_string);
    }
    if let Some(m) = main
        && let Some(b) = read(&m)
    {
        return decode_text(b);
    }
    let names: Vec<String> = zip
        .file_names()
        .filter_map(|r| r.ok().map(|c| c.into_owned()))
        .filter(|n| !n.starts_with("META-INF") && (n.ends_with(".xml") || n.ends_with(".musicxml")))
        .collect();
    for n in names {
        if let Some(b) = zip.by_name(&n).ok().and_then(|mut f| {
            let mut v = Vec::new();
            f.read_to_end(&mut v).ok().map(|_| v)
        }) {
            return decode_text(b);
        }
    }
    err("gói .mxl không có bản nhạc")
}

/// File `.xml` có phải MusicXML không (xem phần đầu file).
pub fn looks_like_musicxml(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = vec![0u8; 4096];
    let n = f.read(&mut head).unwrap_or(0);
    let text = String::from_utf8_lossy(&head[..n]);
    text.contains("<score-partwise")
}

// ---------------------------------------------------------------------------------------------------------------
// Tiện ích đọc cây XML

/// File MusicXML nào cũng có khai báo `<!DOCTYPE …>`: phải cho phép (roxmltree mặc định từ chối).
fn xml_options<'a>() -> roxmltree::ParsingOptions<'a> {
    roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    }
}

fn child<'a, 'i>(n: Node<'a, 'i>, tag: &str) -> Option<Node<'a, 'i>> {
    n.children().find(|c| c.has_tag_name(tag))
}

fn text_of(n: Option<Node>) -> String {
    n.and_then(|n| n.text()).unwrap_or("").trim().to_string()
}

fn num_of(n: Option<Node>) -> Option<f64> {
    text_of(n).parse::<f64>().ok().filter(|x| x.is_finite())
}

fn step_pc(step: &str) -> Option<i32> {
    Some(match step.trim().to_uppercase().as_str() {
        "C" => 0,
        "D" => 2,
        "E" => 4,
        "F" => 5,
        "G" => 7,
        "A" => 9,
        "B" => 11,
        _ => return None,
    })
}

const GUITAR_OPEN: [i32; 6] = [40, 45, 50, 55, 59, 64];
const BASS_OPEN: [i32; 4] = [28, 33, 38, 43];

// ---------------------------------------------------------------------------------------------------------------
// Đọc từng ô nhịp (chưa trải phẳng)

/// Nốt thô trong một ô: vị trí tính bằng phách đen kể từ đầu ô.
#[derive(Clone, Debug)]
struct RawNote {
    off_q: f64,
    dur_q: f64,
    /// Số dây MusicXML: 1 = dây cao nhất.
    xml_string: i32,
    fret: i32,
    tie_start: bool,
    tie_stop: bool,
    /// Bắt đầu một luyến (hammer-on / pull-off) sang nốt sau trên cùng dây.
    legato_start: bool,
    legato_stop: bool,
    slide_start: bool,
    marks: Marks,
}

#[derive(Clone, Debug, Default)]
struct RawBar {
    len_q: f64,
    beats: i32,
    beat_q: f64,
    notes: Vec<RawNote>,
    tempos: Vec<(f64, f64)>,
    sections: Vec<(f64, String)>,
    chord_names: Vec<(f64, String)>,
    // dấu nhắc lại
    forward: bool,
    backward: Option<u32>,
    /// Khung kết bắt đầu ở ô này (danh sách số lần chơi), và có kết thúc ở ô này không.
    ending_start: Option<Vec<u32>>,
    ending_stop: bool,
}

/// Trạng thái kéo dài qua các ô nhịp của một part.
struct PartState {
    divisions: f64,
    beats: i32,
    beat_type: i32,
    lines: i32,
    tuning: HashMap<i32, i32>,
    capo: i32,
}

fn ending_numbers(s: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for tok in s.split([',', ' ']) {
        let tok = tok.trim().trim_end_matches('.');
        if tok.is_empty() {
            continue;
        }
        if let Some((a, b)) = tok.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                out.extend(a..=b);
            }
        } else if let Ok(n) = tok.parse::<u32>() {
            out.push(n);
        }
    }
    out
}

fn tempo_from_metronome(m: Node) -> Option<f64> {
    let bpm = num_of(child(m, "per-minute"))?;
    let unit = text_of(child(m, "beat-unit"));
    let mut mul = match unit.as_str() {
        "whole" => 4.0,
        "half" => 2.0,
        "quarter" | "" => 1.0,
        "eighth" => 0.5,
        "16th" => 0.25,
        _ => 1.0,
    };
    if m.children().any(|c| c.has_tag_name("beat-unit-dot")) {
        mul *= 1.5;
    }
    Some(bpm * mul)
}

fn harmony_name(h: Node) -> Option<String> {
    let root = child(h, "root")?;
    let step = text_of(child(root, "root-step"));
    step_pc(&step)?;
    let alter = num_of(child(root, "root-alter")).unwrap_or(0.0) as i32;
    let acc = match alter {
        1 => "#",
        -1 => "b",
        _ => "",
    };
    let kind_node = child(h, "kind");
    let kind = text_of(kind_node);
    let suffix = match kind_node.and_then(|k| k.attribute("text")) {
        Some(t) => t.to_string(),
        None => match kind.as_str() {
            "major" | "" => "",
            "minor" => "m",
            "dominant" => "7",
            "major-seventh" => "maj7",
            "minor-seventh" => "m7",
            "power" => "5",
            "suspended-second" => "sus2",
            "suspended-fourth" => "sus4",
            "diminished" => "dim",
            "augmented" => "aug",
            "dominant-ninth" => "9",
            "major-sixth" => "6",
            "minor-sixth" => "m6",
            "half-diminished" => "m7b5",
            "diminished-seventh" => "dim7",
            _ => "",
        }
        .to_string(),
    };
    Some(format!("{}{acc}{suffix}", step.to_uppercase()))
}

fn read_marks(note: Node, marks: &mut Marks, legato: &mut (bool, bool), slide: &mut bool) {
    if let Some(nh) = child(note, "notehead")
        && nh.text().map(str::trim) == Some("x")
    {
        marks.dead = true;
    }
    let Some(nots) = child(note, "notations") else {
        return;
    };
    for n in nots.children().filter(|c| c.is_element()) {
        match n.tag_name().name() {
            "technical" => {
                for t in n.children().filter(|c| c.is_element()) {
                    match t.tag_name().name() {
                        "hammer-on" | "pull-off" => match t.attribute("type") {
                            Some("stop") => legato.1 = true,
                            _ => legato.0 = true,
                        },
                        "bend" => {
                            let a = num_of(child(t, "bend-alter")).unwrap_or(2.0).abs();
                            if child(t, "release").is_none() || a > 0.0 {
                                marks.bend = marks.bend.max(a);
                            }
                        }
                        "harmonic" => marks.harmonic = true,
                        "tap" => marks.tap = true,
                        "other-technical" => {
                            let w = t.text().unwrap_or("").to_lowercase();
                            if w.contains("p.m") || w.contains("palm") {
                                marks.palm_mute = true;
                            }
                            if w.contains("vib") {
                                marks.vibrato = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            "slide" | "glissando" => {
                if n.attribute("type") != Some("stop") {
                    *slide = true;
                }
            }
            "ornaments" => {
                for o in n.children().filter(|c| c.is_element()) {
                    match o.tag_name().name() {
                        "wavy-line" | "vibrato" => marks.vibrato = true,
                        "tremolo" => marks.tremolo = true,
                        _ => {}
                    }
                }
            }
            "articulations"
                if n.children()
                    .any(|a| a.has_tag_name("accent") || a.has_tag_name("strong-accent")) =>
            {
                marks.accent = true;
            }
            _ => {}
        }
    }
}

fn read_bar(m: Node, st: &mut PartState) -> RawBar {
    let mut bar = RawBar::default();
    let mut cursor = 0.0f64; // đơn vị divisions
    let mut max_cursor = 0.0f64;
    let mut last_start = 0.0f64;
    let implicit = m.attribute("implicit") == Some("yes");
    let q = |d: f64, st: &PartState| d / st.divisions.max(1.0);

    for c in m.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "attributes" => {
                if let Some(d) = num_of(child(c, "divisions"))
                    && d > 0.0
                {
                    st.divisions = d;
                }
                if let Some(t) = child(c, "time") {
                    // "3+2" → cộng lại
                    let b: i32 = text_of(child(t, "beats"))
                        .split('+')
                        .filter_map(|x| x.trim().parse::<i32>().ok())
                        .sum();
                    let bt = text_of(child(t, "beat-type")).parse::<i32>().unwrap_or(0);
                    if b > 0 && bt > 0 {
                        st.beats = b;
                        st.beat_type = bt;
                    }
                }
                if let Some(sd) = child(c, "staff-details") {
                    if let Some(l) = num_of(child(sd, "staff-lines")) {
                        st.lines = l as i32;
                    }
                    for tun in sd.children().filter(|x| x.has_tag_name("staff-tuning")) {
                        let line = tun
                            .attribute("line")
                            .and_then(|x| x.parse::<i32>().ok())
                            .unwrap_or(0);
                        let Some(pc) = step_pc(&text_of(child(tun, "tuning-step"))) else {
                            continue;
                        };
                        let alter = num_of(child(tun, "tuning-alter")).unwrap_or(0.0) as i32;
                        let oct = num_of(child(tun, "tuning-octave")).unwrap_or(3.0) as i32;
                        if line > 0 {
                            st.tuning.insert(line, (oct + 1) * 12 + pc + alter);
                        }
                    }
                    if let Some(cp) = num_of(child(sd, "capo")) {
                        st.capo = cp.max(0.0) as i32;
                    }
                }
            }
            "backup" => {
                cursor = (cursor - num_of(child(c, "duration")).unwrap_or(0.0)).max(0.0);
            }
            "forward" => {
                cursor += num_of(child(c, "duration")).unwrap_or(0.0);
                max_cursor = max_cursor.max(cursor);
            }
            "sound" => {
                if let Some(t) = c.attribute("tempo").and_then(|x| x.parse::<f64>().ok())
                    && t > 0.0
                {
                    bar.tempos.push((q(cursor, st), t));
                }
            }
            "direction" => {
                let off = num_of(child(c, "offset")).unwrap_or(0.0);
                let at = q((cursor + off).max(0.0), st);
                let mut tempo = child(c, "sound")
                    .and_then(|s| s.attribute("tempo"))
                    .and_then(|x| x.parse::<f64>().ok())
                    .filter(|t| *t > 0.0);
                for dt in c.children().filter(|x| x.has_tag_name("direction-type")) {
                    for x in dt.children().filter(|x| x.is_element()) {
                        match x.tag_name().name() {
                            "metronome" if tempo.is_none() => tempo = tempo_from_metronome(x),
                            "rehearsal" => {
                                let t = x.text().unwrap_or("").trim().to_string();
                                if !t.is_empty() {
                                    bar.sections.push((at, t));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(t) = tempo {
                    bar.tempos.push((at, t));
                }
            }
            "harmony" => {
                if let Some(name) = harmony_name(c) {
                    let off = num_of(child(c, "offset")).unwrap_or(0.0);
                    bar.chord_names.push((q((cursor + off).max(0.0), st), name));
                }
            }
            "barline" => {
                if let Some(r) = child(c, "repeat") {
                    match r.attribute("direction") {
                        Some("forward") => bar.forward = true,
                        Some("backward") => {
                            let times = r
                                .attribute("times")
                                .and_then(|x| x.parse::<u32>().ok())
                                .unwrap_or(2);
                            bar.backward = Some(times.max(1));
                        }
                        _ => {}
                    }
                }
                if let Some(e) = child(c, "ending") {
                    match e.attribute("type") {
                        Some("start") => {
                            bar.ending_start = Some(ending_numbers(e.attribute("number").unwrap_or("1")));
                        }
                        Some("stop") | Some("discontinue") => bar.ending_stop = true,
                        _ => {}
                    }
                }
            }
            "note" => {
                if child(c, "grace").is_some() || child(c, "cue").is_some() {
                    continue;
                }
                let dur = num_of(child(c, "duration")).unwrap_or(0.0);
                let is_chord = child(c, "chord").is_some();
                let start = if is_chord { last_start } else { cursor };
                if !is_chord {
                    last_start = cursor;
                    cursor += dur;
                    max_cursor = max_cursor.max(cursor);
                }
                if child(c, "rest").is_some() {
                    continue;
                }
                let tech = child(c, "notations").and_then(|n| child(n, "technical"));
                let (Some(s), Some(f)) = (
                    tech.and_then(|t| num_of(child(t, "string"))),
                    tech.and_then(|t| num_of(child(t, "fret"))),
                ) else {
                    continue;
                };
                let mut tie_start = false;
                let mut tie_stop = false;
                for t in c.children().filter(|x| x.has_tag_name("tie")) {
                    match t.attribute("type") {
                        Some("start") => tie_start = true,
                        Some("stop") => tie_stop = true,
                        _ => {}
                    }
                }
                if let Some(n) = child(c, "notations") {
                    for t in n.children().filter(|x| x.has_tag_name("tied")) {
                        match t.attribute("type") {
                            Some("start") => tie_start = true,
                            Some("stop") => tie_stop = true,
                            _ => {}
                        }
                    }
                }
                let mut marks = Marks::default();
                let mut legato = (false, false);
                let mut slide = false;
                read_marks(c, &mut marks, &mut legato, &mut slide);
                bar.notes.push(RawNote {
                    off_q: q(start, st),
                    dur_q: q(dur, st),
                    xml_string: s as i32,
                    fret: f.max(0.0) as i32,
                    tie_start,
                    tie_stop,
                    legato_start: legato.0,
                    legato_stop: legato.1,
                    slide_start: slide,
                    marks,
                });
            }
            _ => {}
        }
    }
    let beat_q = 4.0 / st.beat_type.max(1) as f64;
    let sig_len = st.beats.max(1) as f64 * beat_q;
    let used = q(max_cursor, st);
    // ô thiếu (anacrusis) hoặc ô dài hơn số chỉ nhịp: lấy độ dài thật
    bar.len_q = if (implicit && used > 0.0) || used > sig_len + 1e-6 {
        used
    } else {
        sig_len
    };
    bar.beat_q = beat_q;
    bar.beats = ((bar.len_q / beat_q).round() as i32).max(1);
    bar
}

// ---------------------------------------------------------------------------------------------------------------
// Trải phẳng dấu nhắc lại

/// Thứ tự chơi các ô nhịp (chỉ số trong danh sách gốc).
fn play_order(bars: &[RawBar]) -> Vec<usize> {
    // số lần chơi của khung kết mà mỗi ô thuộc về (rỗng = không thuộc khung nào)
    let mut endings: Vec<Vec<u32>> = vec![Vec::new(); bars.len()];
    let mut cur: Vec<u32> = Vec::new();
    for (i, b) in bars.iter().enumerate() {
        if let Some(n) = &b.ending_start {
            cur = n.clone();
        }
        endings[i] = cur.clone();
        if b.ending_stop {
            cur.clear();
        }
    }
    let mut order = Vec::new();
    let (mut i, mut start, mut pass) = (0usize, 0usize, 1u32);
    let mut jumped = false;
    let mut guard = 0usize;
    while i < bars.len() {
        guard += 1;
        if guard > 10_000 {
            break; // phòng file lỗi gây lặp vô hạn
        }
        let b = &bars[i];
        if b.forward && !jumped {
            start = i;
            pass = 1;
        }
        jumped = false;
        if !endings[i].is_empty() && !endings[i].contains(&pass) {
            i += 1;
            continue;
        }
        order.push(i);
        if let Some(times) = b.backward {
            // ở khung kết cuối cùng không quay lại nữa
            let last_ending = !endings[i].is_empty() && endings[i].iter().all(|&n| n >= times);
            if pass < times && !last_ending {
                pass += 1;
                i = start;
                jumped = true;
                continue;
            }
            pass = 1;
            start = i + 1;
        }
        i += 1;
    }
    order
}

// ---------------------------------------------------------------------------------------------------------------
// Toàn bộ bản nhạc

pub fn parse(text: &str, fallback_title: &str) -> Res<Score> {
    let doc = Document::parse_with_options(text, xml_options())
        .map_err(|e| XmlError(format!("XML không hợp lệ: {e}")))?;
    let root = doc.root_element();
    if root.has_tag_name("score-timewise") {
        return err("chưa hỗ trợ MusicXML dạng score-timewise; hãy xuất lại ở dạng partwise");
    }
    if !root.has_tag_name("score-partwise") {
        return err("không phải file MusicXML (thiếu <score-partwise>)");
    }

    let mut title = text_of(child(root, "work").and_then(|w| child(w, "work-title")));
    if title.is_empty() {
        title = text_of(child(root, "movement-title"));
    }
    if title.is_empty() {
        // tiêu đề trong <credit> (MuseScore hay để ở đây)
        title = root
            .children()
            .filter(|c| c.has_tag_name("credit"))
            .find(|c| child(*c, "credit-type").map(|t| text_of(Some(t))) == Some("title".into()))
            .and_then(|c| child(c, "credit-words"))
            .map(|w| text_of(Some(w)))
            .unwrap_or_default();
    }
    if title.is_empty() {
        title = fallback_title.to_string();
    }
    let ident = child(root, "identification");
    let creators: Vec<Node> = ident
        .map(|i| i.children().filter(|c| c.has_tag_name("creator")).collect())
        .unwrap_or_default();
    let artist = creators
        .iter()
        .find(|c| matches!(c.attribute("type"), Some("composer") | Some("artist")))
        .or_else(|| creators.first())
        .map(|c| text_of(Some(*c)))
        .unwrap_or_default();

    // tên và chương trình MIDI của từng part
    let mut part_info: HashMap<String, (String, i32)> = HashMap::new();
    if let Some(pl) = child(root, "part-list") {
        for sp in pl.children().filter(|c| c.has_tag_name("score-part")) {
            let id = sp.attribute("id").unwrap_or("").to_string();
            let name = text_of(child(sp, "part-name"));
            let prog = sp
                .descendants()
                .find(|d| d.has_tag_name("midi-program"))
                .and_then(|d| d.text())
                .and_then(|t| t.trim().parse::<i32>().ok())
                .unwrap_or(0);
            part_info.insert(id, (name, prog));
        }
    }

    let mut parts = Vec::new();
    let mut timeline: Option<(Vec<RawBar>, Vec<usize>)> = None;
    for p in root.children().filter(|c| c.has_tag_name("part")) {
        let mut st = PartState {
            divisions: 1.0,
            beats: 4,
            beat_type: 4,
            lines: 6,
            tuning: HashMap::new(),
            capo: 0,
        };
        let raw: Vec<RawBar> = p
            .children()
            .filter(|c| c.has_tag_name("measure"))
            .map(|m| read_bar(m, &mut st))
            .collect();
        if !raw.iter().any(|b| !b.notes.is_empty()) {
            continue; // part không có tab
        }
        let order = play_order(&raw);
        let id = p.attribute("id").unwrap_or("").to_string();
        let (pname, prog) = part_info.get(&id).cloned().unwrap_or_default();
        let lines = st.lines.clamp(1, 6);
        let is_bass = lines == 4 || (33..=40).contains(&prog) || pname.to_lowercase().contains("bass");
        // dây buông: theo <staff-tuning> nếu có, không thì E chuẩn
        let mut open_midi = Vec::new();
        for line in 1..=lines {
            let i = (line - 1) as usize;
            let std_midi = if is_bass {
                BASS_OPEN.get(i).copied().unwrap_or(43 + 5 * (i as i32 - 3))
            } else {
                GUITAR_OPEN.get(i).copied().unwrap_or(64)
            };
            open_midi.push(st.tuning.get(&line).copied().unwrap_or(std_midi));
        }
        let notes = unfold_notes(&raw, &order, lines);
        let chord_names = unfold_marks(&raw, &order, |b| &b.chord_names);
        parts.push(Part {
            name: pname,
            is_bass,
            open_midi,
            capo: st.capo,
            notes,
            chord_names,
        });
        if timeline.is_none() {
            timeline = Some((raw, order));
        }
    }
    let Some((raw, order)) = timeline else {
        return err("bài không có nốt tab nào (cần <technical><string> và <fret>)");
    };

    let mut bars = Vec::new();
    let mut q = 0.0;
    for &i in &order {
        let b = &raw[i];
        bars.push(Bar {
            q,
            len_q: b.len_q,
            beats: b.beats,
            beat_q: b.beat_q,
        });
        q += b.len_q;
    }
    let mut tempos = unfold_marks(&raw, &order, |b| &b.tempos);
    if tempos.first().is_none_or(|t| t.0 > 1e-9) {
        let first = tempos.first().map(|t| t.1).unwrap_or(120.0);
        tempos.insert(0, (0.0, first));
    }
    let sections = unfold_marks(&raw, &order, |b| &b.sections);
    Ok(Score {
        title,
        artist,
        parts,
        bars,
        tempos,
        sections,
        end_q: q,
    })
}

/// Lấy các mốc (tốc độ, tên đoạn…) của từng ô theo thứ tự chơi, đổi sang vị trí tuyệt đối.
fn unfold_marks<T: Clone>(
    raw: &[RawBar],
    order: &[usize],
    get: impl Fn(&RawBar) -> &Vec<(f64, T)>,
) -> Vec<(f64, T)> {
    let mut out = Vec::new();
    let mut q = 0.0;
    for &i in order {
        for (off, v) in get(&raw[i]) {
            out.push((q + off, v.clone()));
        }
        q += raw[i].len_q;
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

fn unfold_notes(raw: &[RawBar], order: &[usize], lines: i32) -> Vec<TabNote> {
    let mut out: Vec<TabNote> = Vec::new();
    // theo từng dây: nốt đang nối (tie), luyến và vuốt chờ nốt kế tiếp
    let mut open_tie: HashMap<i32, usize> = HashMap::new();
    let mut legato_from: HashMap<i32, usize> = HashMap::new();
    let mut slide_from: HashMap<i32, usize> = HashMap::new();
    let mut q = 0.0;
    for &i in order {
        let mut notes = raw[i].notes.clone();
        notes.sort_by(|a, b| a.off_q.total_cmp(&b.off_q));
        for n in notes {
            let string = lines - n.xml_string;
            if string < 0 || string >= lines {
                continue;
            }
            let at = q + n.off_q;
            if n.tie_stop
                && let Some(&k) = open_tie.get(&string)
                && out[k].fret == n.fret
            {
                out[k].dur_q = (at + n.dur_q - out[k].q).max(out[k].dur_q);
                if !n.tie_start {
                    open_tie.remove(&string);
                }
                continue;
            }
            let idx = out.len();
            let mut note = TabNote {
                q: at,
                dur_q: n.dur_q,
                string,
                fret: n.fret,
                slide_to: -1,
                marks: n.marks.clone(),
            };
            if let Some(k) = legato_from.remove(&string)
                && out[k].fret != n.fret
            {
                note.marks.hammer = true;
            } else if n.legato_stop {
                note.marks.hammer = true;
            }
            if let Some(k) = slide_from.remove(&string)
                && out[k].fret != n.fret
            {
                out[k].marks.slide = true;
                out[k].slide_to = n.fret;
            }
            out.push(note);
            if n.tie_start {
                open_tie.insert(string, idx);
            } else {
                open_tie.remove(&string);
            }
            if n.legato_start {
                legato_from.insert(string, idx);
            }
            if n.slide_start {
                slide_from.insert(string, idx);
            }
        }
        q += raw[i].len_q;
    }
    out.sort_by(|a, b| a.q.total_cmp(&b.q).then(a.string.cmp(&b.string)));
    out
}

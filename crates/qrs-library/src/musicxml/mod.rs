//! Nhập bài từ MusicXML (`.musicxml`, `.mxl`, hoặc `.xml` dạng MusicXML) thành feedpak để quickRS chơi được.
//!
//! Bài phải có tab (mỗi nốt có `<technical><string>` và `<fret>`), như file xuất từ Guitar Pro, MuseScore,
//! TuxGuitar hay các bài tập B01, B02… Kết quả là một `.feedpak` nằm cạnh file gốc:
//!
//!   manifest.yaml            tên bài, tác giả, độ dài, các bè (tuning, capo) và hai stem
//!   arrangements/<id>.json   nốt, hợp âm, vị trí tay (anchor), thế bấm (handshape) của từng bè
//!   song_timeline.json       phách, ô nhịp và các đoạn (để vòng lặp theo đoạn)
//!   stems/<bè>.wav           tiếng tổng hợp của từng bè (Lead, Rhythm, Bass…), mỗi bè một kênh riêng
//!   stems/click.wav          tiếng gõ nhịp, có một ô đếm vào trước khi bài bắt đầu
//!
//! Hai stem tách riêng nên trong bộ trộn (mixer) của quickRS có thể tắt tiếng guitar để tự chơi theo nhịp.

pub mod parse;
pub mod synth;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::feedpak_export::{r3, text::manifest_yaml, write_zip};
pub use parse::{Res, XmlError};
use parse::{Score, TabNote};

/// Đuôi file luôn được coi là MusicXML.
pub const SUFFIXES: [&str; 2] = ["musicxml", "mxl"];

/// Đổi khi cách chuyển thay đổi, để các feedpak cũ được tạo lại.
pub const IMPORTER_VERSION: &str = "2";

const GUITAR_STD: [i32; 6] = [40, 45, 50, 55, 59, 64];
const BASS_STD: [i32; 6] = [28, 33, 38, 43, 47, 52];

/// File này có phải MusicXML không: theo đuôi file, riêng `.xml` thì xem nội dung.
pub fn is_musicxml(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    SUFFIXES.contains(&ext.as_str()) || (ext == "xml" && parse::looks_like_musicxml(path))
}

/// Feedpak tương ứng: cùng thư mục, cùng tên, đuôi `.feedpak`.
pub fn feedpak_path_for(src: &Path) -> PathBuf {
    src.with_extension("feedpak")
}

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// Cần tạo (lại) feedpak không: chưa có, cũ hơn file MusicXML, hoặc do bản nhập cũ tạo ra.
pub fn needs_update(src: &Path, out: &Path) -> bool {
    match (mtime(src), mtime(out)) {
        (Some(s), Some(o)) if o >= s => !made_by_current_importer(out),
        (_, None) => true,
        _ => true,
    }
}

fn made_by_current_importer(out: &Path) -> bool {
    let Ok(f) = std::fs::File::open(out) else {
        return false;
    };
    let Ok(mut zip) = zip::ZipArchive::new(f) else {
        return false;
    };
    let Ok(mut m) = zip.by_name("manifest.yaml") else {
        return false;
    };
    let mut text = String::new();
    let _ = std::io::Read::read_to_string(&mut m, &mut text);
    text.contains(&format!("musicxml_importer: \"{IMPORTER_VERSION}\""))
        || text.contains(&format!("musicxml_importer: '{IMPORTER_VERSION}'"))
        || text.contains(&format!("musicxml_importer: {IMPORTER_VERSION}\n"))
}

/// Chuyển một file MusicXML thành feedpak tại `out`.
pub fn convert(src: &Path, out: &Path) -> Res<Summary> {
    let (files, summary) = build(src)?;
    write_zip(&files, out, None).map_err(|e| XmlError(e.to_string()))?;
    Ok(summary)
}

/// Tạo feedpak cạnh file gốc nếu cần; trả về đường dẫn feedpak (đã có hoặc vừa tạo).
pub fn ensure_feedpak(src: &Path) -> Res<PathBuf> {
    let out = feedpak_path_for(src);
    if needs_update(src, &out) {
        convert(src, &out)?;
    }
    Ok(out)
}

/// Tóm tắt kết quả chuyển, để in ra cho người dùng.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub title: String,
    pub parts: Vec<String>,
    pub notes: usize,
    pub chords: usize,
    pub bars: usize,
    pub duration: f64,
    pub bpm: f64,
}

// ---------------------------------------------------------------------------------------------------------------
// Bản đồ tốc độ: phách đen → giây

struct TempoMap {
    /// (vị trí phách đen, BPM, giây tại vị trí đó)
    points: Vec<(f64, f64, f64)>,
    offset: f64,
}

impl TempoMap {
    fn new(tempos: &[(f64, f64)], offset: f64) -> TempoMap {
        let mut points: Vec<(f64, f64, f64)> = Vec::new();
        for &(q, bpm) in tempos {
            let bpm = bpm.clamp(10.0, 400.0);
            let sec = match points.last() {
                Some(&(pq, pbpm, ps)) => ps + (q - pq) * 60.0 / pbpm,
                None => 0.0,
            };
            if let Some(last) = points.last_mut()
                && (last.0 - q).abs() < 1e-9
            {
                last.1 = bpm; // hai tốc độ cùng vị trí: lấy cái sau
                continue;
            }
            points.push((q, bpm, sec));
        }
        TempoMap { points, offset }
    }

    fn sec(&self, q: f64) -> f64 {
        let mut p = self.points[0];
        for &x in &self.points {
            if x.0 <= q + 1e-9 {
                p = x;
            } else {
                break;
            }
        }
        self.offset + p.2 + (q - p.0) * 60.0 / p.1
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tên hợp âm tự đoán (khi bài không ghi)

const NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

fn guess_chord_name(midis: &[i32]) -> String {
    if midis.len() < 2 {
        return String::new();
    }
    let bass = *midis.iter().min().unwrap();
    let pcs: Vec<i32> = {
        let mut v: Vec<i32> = midis.iter().map(|m| m.rem_euclid(12)).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let shapes: [(&[i32], &str); 12] = [
        (&[0, 4, 7], ""),
        (&[0, 3, 7], "m"),
        (&[0, 7], "5"),
        (&[0, 4, 7, 10], "7"),
        (&[0, 3, 7, 10], "m7"),
        (&[0, 4, 7, 11], "maj7"),
        (&[0, 2, 7], "sus2"),
        (&[0, 5, 7], "sus4"),
        (&[0, 3, 6], "dim"),
        (&[0, 4, 8], "aug"),
        (&[0, 4, 7, 9], "6"),
        (&[0, 2, 4, 7], "add9"),
    ];
    // thử nốt trầm làm gốc trước, rồi các nốt khác
    let mut roots = vec![bass.rem_euclid(12)];
    roots.extend(pcs.iter().copied().filter(|&p| p != bass.rem_euclid(12)));
    for root in roots {
        let mut rel: Vec<i32> = pcs.iter().map(|p| (p - root).rem_euclid(12)).collect();
        rel.sort_unstable();
        for (shape, suffix) in shapes {
            if rel == shape {
                let name = format!("{}{suffix}", NAMES[root as usize]);
                return if root != bass.rem_euclid(12) {
                    format!("{name}/{}", NAMES[bass.rem_euclid(12) as usize])
                } else {
                    name
                };
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------------------------------------------
// Dựng feedpak

fn note_json(n: &TabNote, t: f64, sus: f64, prev_fret: Option<i32>, with_time: bool) -> Map<String, Value> {
    let mut d = Map::new();
    if with_time {
        d.insert("t".into(), json!(r3(t)));
    }
    d.insert("s".into(), json!(n.string));
    d.insert("f".into(), json!(n.fret));
    if sus > 0.0 {
        d.insert("sus".into(), json!(r3(sus)));
    }
    let m = &n.marks;
    if m.hammer {
        // đi lên phím cao hơn là hammer-on, xuống thấp hơn là pull-off
        let up = prev_fret.is_none_or(|p| n.fret > p);
        d.insert(if up { "ho" } else { "po" }.into(), json!(true));
    }
    if n.slide_to >= 0 {
        d.insert("sl".into(), json!(n.slide_to));
    }
    if m.bend > 0.0 {
        d.insert("bn".into(), json!(r3(m.bend)));
    }
    for (on, key) in [
        (m.vibrato, "vb"),
        (m.palm_mute, "pm"),
        (m.dead, "mt"),
        (m.harmonic, "hm"),
        (m.accent, "ac"),
        (m.tap, "tp"),
        (m.tremolo, "tr"),
    ] {
        if on {
            d.insert(key.into(), json!(true));
        }
    }
    d
}

struct PartOut {
    id: String,
    name: String,
    json: Value,
    tuning: Vec<i32>,
    capo: i32,
    notes: usize,
    chords: usize,
}

fn build_part(
    p: &parse::Part,
    tm: &TempoMap,
    voices: &mut Vec<synth::Voice>,
    add_voices: bool,
) -> (Value, usize, usize) {
    let n_strings = p.open_midi.len();
    let midi_of = |n: &TabNote| p.open_midi.get(n.string as usize).copied().unwrap_or(40) + p.capo + n.fret;
    // gom nốt cùng thời điểm thành hợp âm
    let mut groups: Vec<Vec<&TabNote>> = Vec::new();
    for n in &p.notes {
        match groups.last_mut() {
            Some(g) if (g[0].q - n.q).abs() < 1e-6 => {
                if !g.iter().any(|x| x.string == n.string) {
                    g.push(n);
                }
            }
            _ => groups.push(vec![n]),
        }
    }
    let mut notes_out: Vec<Value> = Vec::new();
    let mut chords_out: Vec<Value> = Vec::new();
    let mut templates: Vec<Value> = Vec::new();
    let mut tpl_ids: HashMap<(Vec<i32>, String), usize> = HashMap::new();
    let mut shapes: Vec<Map<String, Value>> = Vec::new();
    let mut anchors: Vec<Value> = Vec::new();
    let mut anchor_fret = 0i32;
    let mut last_fret: HashMap<i32, i32> = HashMap::new();
    let mut last_chord: Option<(usize, f64)> = None;

    for g in &groups {
        let t = tm.sec(g[0].q);
        let dur_of = |n: &TabNote| (tm.sec(n.q + n.dur_q) - tm.sec(n.q)).max(0.0);
        // độ ngân (sustain): chỉ ghi cho nốt dài từ khoảng nửa giây trở lên
        let sus_of = |n: &TabNote| {
            let d = dur_of(n);
            if d >= 0.45 { d * 0.9 } else { 0.0 }
        };
        // vị trí tay: phím bấm thấp nhất / cao nhất của nhóm nốt
        let fretted: Vec<i32> = g.iter().map(|n| n.fret).filter(|&f| f > 0).collect();
        if let (Some(&lo), Some(&hi)) = (fretted.iter().min(), fretted.iter().max()) {
            let need = anchor_fret == 0 || lo < anchor_fret || hi > anchor_fret + 3;
            if need {
                let f = if anchor_fret == 0 || lo < anchor_fret {
                    lo
                } else {
                    (hi - 3).max(1)
                };
                let f = f.min(lo).max(1);
                if f != anchor_fret {
                    anchor_fret = f;
                    let width = (hi - f + 1).max(4);
                    anchors.push(json!({"time": r3(t), "fret": f, "width": width}));
                }
            }
        }
        if add_voices {
            for n in g {
                voices.push(synth::Voice {
                    time: t,
                    dur: dur_of(n),
                    midi: midi_of(n) as f64,
                    dead: n.marks.dead,
                    accent: n.marks.accent,
                });
            }
        }
        if g.len() == 1 {
            let n = g[0];
            let prev = last_fret.get(&n.string).copied();
            notes_out.push(Value::Object(note_json(n, t, sus_of(n), prev, true)));
            last_fret.insert(n.string, n.fret);
            continue;
        }
        // hợp âm
        let mut frets = vec![-1; 6];
        for n in g {
            if (n.string as usize) < 6 {
                frets[n.string as usize] = n.fret;
            }
        }
        let name = p
            .chord_names
            .iter()
            .rev()
            .find(|(q, _)| *q <= g[0].q + 1e-6 && g[0].q - *q < 1e-6)
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| guess_chord_name(&g.iter().map(|n| midi_of(n)).collect::<Vec<_>>()));
        let id = *tpl_ids.entry((frets.clone(), name.clone())).or_insert_with(|| {
            templates.push(json!({
                "name": name, "displayName": name, "arp": false,
                "fingers": vec![-1; 6], "frets": frets,
            }));
            templates.len() - 1
        });
        let end = g.iter().map(|n| t + dur_of(n)).fold(t + 0.1, f64::max);
        let mut c = Map::new();
        c.insert("t".into(), json!(r3(t)));
        c.insert("id".into(), json!(id));
        let notes: Vec<Value> = g
            .iter()
            .map(|n| {
                Value::Object(note_json(
                    n,
                    t,
                    sus_of(n),
                    last_fret.get(&n.string).copied(),
                    false,
                ))
            })
            .collect();
        c.insert("notes".into(), Value::Array(notes));
        // gảy lại cùng hợp âm liền nhau: vẽ gọn như Rocksmith, gộp thế bấm
        let repeat = matches!(last_chord, Some((pid, pend)) if pid == id && t - pend < 0.05);
        if repeat {
            c.insert("hd".into(), json!(true));
            if let Some(s) = shapes.last_mut() {
                s.insert("end_time".into(), json!(r3(end)));
            }
        } else {
            let mut s = Map::new();
            s.insert("chord_id".into(), json!(id));
            s.insert("start_time".into(), json!(r3(t)));
            s.insert("end_time".into(), json!(r3(end)));
            s.insert("arp".into(), json!(false));
            shapes.push(s);
        }
        last_chord = Some((id, end));
        for n in g {
            last_fret.insert(n.string, n.fret);
        }
        chords_out.push(Value::Object(c));
    }
    if anchors.is_empty() {
        anchors.push(json!({"time": 0.0, "fret": 1, "width": 4}));
    }
    let n_notes = notes_out.len();
    let n_chords = chords_out.len();
    let std = if p.is_bass { BASS_STD } else { GUITAR_STD };
    let tuning: Vec<i32> = (0..n_strings).map(|i| p.open_midi[i] - std[i.min(5)]).collect();
    let v = json!({
        "name": "",
        "tuning": tuning,
        "capo": p.capo,
        "notes": notes_out,
        "chords": chords_out,
        "anchors": anchors,
        "handshapes": shapes,
        "templates": templates,
    });
    (v, n_notes, n_chords)
}

fn part_names(score: &Score) -> Vec<(String, String)> {
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut guitars = 0;
    score
        .parts
        .iter()
        .map(|p| {
            let base = if p.is_bass {
                "Bass".to_string()
            } else {
                guitars += 1;
                match guitars {
                    1 => "Lead".to_string(),
                    2 => "Rhythm".to_string(),
                    _ if !p.name.trim().is_empty() => p.name.trim().to_string(),
                    k => format!("Guitar {k}"),
                }
            };
            let k = used.entry(base.clone()).or_insert(0);
            *k += 1;
            let name = if *k > 1 { format!("{base} {k}") } else { base };
            let id: String = name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            (id, name)
        })
        .collect()
}

/// Dựng các file của feedpak từ một file MusicXML.
pub fn build(src: &Path) -> Res<(BTreeMap<String, Vec<u8>>, Summary)> {
    let text = parse::read_text(src)?;
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "MusicXML".into());
    let score = parse::parse(&text, &stem)?;
    build_score(&score, &stem)
}

/// Dựng feedpak từ bản nhạc đã đọc (tách riêng để kiểm thử).
pub fn build_score(score: &Score, file_stem: &str) -> Res<(BTreeMap<String, Vec<u8>>, Summary)> {
    let first = score.bars.first().cloned().unwrap_or(parse::Bar {
        q: 0.0,
        len_q: 4.0,
        beats: 4,
        beat_q: 1.0,
    });
    let bpm0 = score
        .tempos
        .first()
        .map(|t| t.1)
        .unwrap_or(120.0)
        .clamp(10.0, 400.0);
    // một ô đếm vào trước bài
    let count_in = first.beats as f64 * first.beat_q * 60.0 / bpm0;
    let tm = TempoMap::new(&score.tempos, count_in);
    let song_end = tm.sec(score.end_q);
    let duration = song_end + 1.0;

    // phách và ô nhịp: các phách đếm vào không thuộc ô nào, để ô đầu tiên của bài vẫn là ô 1
    let mut beats: Vec<Value> = Vec::new();
    let mut clicks: Vec<(f64, bool)> = Vec::new();
    for k in 0..first.beats {
        let t = k as f64 * first.beat_q * 60.0 / bpm0;
        beats.push(json!({"time": r3(t), "measure": -1}));
        clicks.push((t, k == 0));
    }
    for (i, b) in score.bars.iter().enumerate() {
        for k in 0..b.beats {
            let t = tm.sec(b.q + k as f64 * b.beat_q);
            beats.push(json!({"time": r3(t), "measure": if k == 0 { i as i64 + 1 } else { -1 }}));
            clicks.push((t, k == 0));
        }
    }

    // các đoạn: theo dấu đoạn trong bài; không có thì mỗi 4 ô một đoạn
    let mut sections: Vec<(f64, String)> = vec![(0.0, "count in".into())];
    if !score.sections.is_empty() {
        if score.sections[0].0 > 1e-6 {
            sections.push((tm.sec(0.0), "intro".into()));
        }
        for (q, name) in &score.sections {
            sections.push((tm.sec(*q), name.clone()));
        }
    } else if score.bars.len() <= 4 {
        sections.push((tm.sec(0.0), "riff".into()));
    } else {
        for (i, b) in score.bars.iter().enumerate().step_by(4) {
            let last = (i + 4).min(score.bars.len());
            sections.push((tm.sec(b.q), format!("bars {}-{}", i + 1, last)));
        }
    }
    let mut counts: HashMap<String, i64> = HashMap::new();
    let sections_json: Vec<Value> = sections
        .iter()
        .map(|(t, name)| {
            let n = counts.entry(name.to_lowercase()).or_insert(0);
            *n += 1;
            json!({"name": name, "number": *n, "time": r3(*t)})
        })
        .collect();

    // từng bè
    let names = part_names(score);
    let mut parts_out: Vec<PartOut> = Vec::new();
    // tiếng tổng hợp của từng bè, để bộ trộn bật/tắt riêng (không trộn chung một kênh)
    let mut part_voices: Vec<Vec<synth::Voice>> = Vec::new();
    for (i, p) in score.parts.iter().enumerate() {
        let mut voices = Vec::new();
        let (mut v, n, c) = build_part(p, &tm, &mut voices, true);
        part_voices.push(voices);
        let (id, name) = names[i].clone();
        v["name"] = json!(name);
        let tuning = v["tuning"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_i64()).map(|x| x as i32).collect())
            .unwrap_or_default();
        parts_out.push(PartOut {
            id,
            name,
            json: v,
            tuning,
            capo: p.capo,
            notes: n,
            chords: c,
        });
    }

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut arrs: Vec<Value> = Vec::new();
    for p in &parts_out {
        let file = format!("arrangements/{}.json", p.id);
        files.insert(
            file.clone(),
            serde_json::to_vec_pretty(&p.json).map_err(|e| XmlError(e.to_string()))?,
        );
        arrs.push(json!({"id": p.id, "name": p.name, "file": file, "tuning": p.tuning, "capo": p.capo}));
    }
    files.insert(
        "song_timeline.json".into(),
        serde_json::to_vec_pretty(&json!({"beats": beats, "sections": sections_json}))
            .map_err(|e| XmlError(e.to_string()))?,
    );
    let mut stems: Vec<Value> = Vec::new();
    for (p, voices) in parts_out.iter().zip(&part_voices) {
        let file = format!("stems/{}.wav", p.id);
        files.insert(file.clone(), synth::wav(&synth::render_guitar(voices, duration)));
        stems.push(json!({"id": p.id, "file": file, "name": format!("{} (synth)", p.name), "default": true}));
    }
    stems.push(json!({"id": "click", "file": "stems/click.wav", "name": "Click", "default": true}));
    files.insert(
        "stems/click.wav".into(),
        synth::wav(&synth::render_click(&clicks, duration)),
    );

    let title = if score.title.trim().is_empty() {
        file_stem.to_string()
    } else {
        score.title.trim().to_string()
    };
    let artist = if score.artist.trim().is_empty() {
        "MusicXML".to_string()
    } else {
        score.artist.trim().to_string()
    };
    let mut manifest = Map::new();
    manifest.insert("feedpak_version".into(), json!("1.0.0"));
    manifest.insert("title".into(), json!(title));
    manifest.insert("artist".into(), json!(artist));
    manifest.insert("duration".into(), json!(r3(duration)));
    manifest.insert("arrangements".into(), Value::Array(arrs));
    manifest.insert(
        "stems".into(),
        Value::Array(stems),
    );
    manifest.insert("song_timeline".into(), json!("song_timeline.json"));
    manifest.insert("musicxml_importer".into(), json!(IMPORTER_VERSION));
    let yaml = manifest_yaml(
        &manifest,
        &format!("Made by quickRS from {file_stem} (MusicXML)."),
    )
    .map_err(XmlError)?;
    files.insert("manifest.yaml".into(), yaml.into_bytes());

    let summary = Summary {
        title,
        parts: parts_out.iter().map(|p| p.name.clone()).collect(),
        notes: parts_out.iter().map(|p| p.notes).sum(),
        chords: parts_out.iter().map(|p| p.chords).sum(),
        bars: score.bars.len(),
        duration,
        bpm: bpm0,
    };
    Ok((files, summary))
}

#[cfg(test)]
mod tests;

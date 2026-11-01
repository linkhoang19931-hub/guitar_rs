//! Kiểm thử bộ nhập MusicXML bằng file mẫu trong share/musicxml.

use super::*;
use qrs_formats::{Keys, Package};

const DEMO: &str = include_str!("../../../../share/musicxml/Demo - Power chord riff.musicxml");

fn demo_score() -> Score {
    parse::parse(DEMO, "demo").expect("đọc được file mẫu")
}

#[test]
fn repeats_are_unfolded() {
    let s = demo_score();
    // ô 1–2 nhắc lại 2 lần, rồi ô 3–4: tổng 6 ô, mỗi ô 4 phách đen
    assert_eq!(s.bars.len(), 6);
    assert!((s.end_q - 24.0).abs() < 1e-9);
    assert_eq!(s.tempos[0], (0.0, 70.0));
    let names: Vec<&str> = s.sections.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["riff", "riff", "lick"]);
}

#[test]
fn strings_tuning_ties_and_techniques() {
    let s = demo_score();
    let p = &s.parts[0];
    assert!(!p.is_bass);
    assert_eq!(p.open_midi, vec![40, 45, 50, 55, 59, 64]);
    // dây 6 trong MusicXML là dây trầm nhất = dây 0 kiểu Rocksmith
    let first = &p.notes[0];
    assert_eq!((first.string, first.fret, first.q), (0, 0, 0.0));
    // nốt nối ở ô 4 được gộp: dây 3 (Rocksmith) phím 2 dài 1 + 4 phách
    let tied: Vec<&TabNote> = p.notes.iter().filter(|n| n.q >= 22.0 - 1e-9).collect();
    assert!(tied.is_empty(), "không còn nốt riêng ở ô 4");
    let long = p.notes.iter().find(|n| (n.q - 19.0).abs() < 1e-9).unwrap();
    assert_eq!((long.string, long.fret), (2, 2));
    assert!((long.dur_q - 5.0).abs() < 1e-9);
    // hammer-on và slide
    let ho = p.notes.iter().find(|n| (n.q - 16.5).abs() < 1e-9).unwrap();
    assert!(ho.marks.hammer);
    let sl = p.notes.iter().find(|n| (n.q - 17.0).abs() < 1e-9).unwrap();
    assert_eq!(sl.slide_to, 4);
}

#[test]
fn chord_names_are_guessed() {
    assert_eq!(guess_chord_name(&[40, 47]), "E5");
    assert_eq!(guess_chord_name(&[45, 52, 57, 61, 64]), "A");
    assert_eq!(guess_chord_name(&[40, 47, 52, 55, 59, 64]), "Em");
    assert_eq!(guess_chord_name(&[48, 52, 55, 60, 64]), "C");
}

#[test]
fn tempo_map_counts_seconds() {
    let tm = TempoMap::new(&[(0.0, 60.0), (4.0, 120.0)], 2.0);
    assert!((tm.sec(0.0) - 2.0).abs() < 1e-9);
    assert!((tm.sec(4.0) - 6.0).abs() < 1e-9);
    assert!((tm.sec(8.0) - 8.0).abs() < 1e-9);
}

#[test]
fn demo_converts_and_quickrs_reads_it_back() {
    let dir = tempfile_dir();
    let src = dir.join("Demo.musicxml");
    std::fs::write(&src, DEMO).unwrap();
    let out = feedpak_path_for(&src);
    assert!(needs_update(&src, &out));
    let sum = convert(&src, &out).unwrap();
    assert_eq!(sum.parts, vec!["Lead".to_string()]);
    assert_eq!(sum.bars, 6);
    assert_eq!(sum.chords, 10);
    assert_eq!(sum.notes, 5);
    assert!(!needs_update(&src, &out), "lần sau không cần chuyển lại");

    // mở lại bằng chính bộ đọc feedpak của quickRS
    let mut pkg = Package::open(&out, &Keys::default(), None).expect("quickRS mở được feedpak");
    assert_eq!(pkg.title(), "Demo — Power chord riff");
    let arr = pkg.load(None, 1.0).unwrap();
    assert_eq!(arr.chords.len(), 10);
    assert_eq!(arr.notes.len(), 5);
    assert_eq!(arr.chords[0].name, "E5");
    // ô đếm vào 4 phách ở 70 BPM ≈ 3,43 giây
    assert!((arr.chords[0].time - 4.0 * 60.0 / 70.0).abs() < 2e-3);
    assert!(arr.notes.iter().any(|n| n.has(qrs_formats::Techs::HAMMER_ON)));
    assert!(arr.notes.iter().any(|n| n.slide_to == 4));
    let secs: Vec<String> = arr.sections.iter().map(|s| s.name.clone()).collect();
    assert_eq!(secs, ["count in", "riff", "riff", "lick"]);
    assert_eq!(arr.beats.iter().filter(|b| b.is_downbeat()).count(), 6);
    // hai stem cho bộ trộn
    let stems = qrs_formats::song_audio::song_stems(&mut pkg).unwrap().unwrap();
    assert_eq!(stems.0.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mxl_package_is_read() {
    use std::io::Write;
    let dir = tempfile_dir();
    let src = dir.join("Demo.mxl");
    let f = std::fs::File::create(&src).unwrap();
    let mut z = zip::ZipWriter::new(f);
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("META-INF/container.xml", o).unwrap();
    z.write_all(br#"<container><rootfiles><rootfile full-path="score.xml"/></rootfiles></container>"#)
        .unwrap();
    z.start_file("score.xml", o).unwrap();
    z.write_all(DEMO.as_bytes()).unwrap();
    z.finish().unwrap();
    assert!(is_musicxml(&src));
    let (files, sum) = build(&src).unwrap();
    assert!(files.contains_key("manifest.yaml"));
    assert_eq!(sum.bars, 6);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bass_part_and_drop_tuning() {
    let xml = r#"<?xml version="1.0"?>
<score-partwise version="4.0">
  <part-list><score-part id="P1"><part-name>Bass</part-name></score-part></part-list>
  <part id="P1"><measure number="1">
    <attributes><divisions>1</divisions><time><beats>4</beats><beat-type>4</beat-type></time>
      <staff-details><staff-lines>4</staff-lines>
        <staff-tuning line="1"><tuning-step>D</tuning-step><tuning-octave>1</tuning-octave></staff-tuning>
      </staff-details></attributes>
    <note><pitch><step>D</step><octave>1</octave></pitch><duration>4</duration><type>whole</type>
      <notations><technical><string>4</string><fret>0</fret></technical></notations></note>
  </measure></part>
</score-partwise>"#;
    let s = parse::parse(xml, "bass").unwrap();
    let p = &s.parts[0];
    assert!(p.is_bass);
    assert_eq!(p.open_midi, vec![26, 33, 38, 43]);
    assert_eq!(p.notes[0].string, 0);
    let (files, sum) = build_score(&s, "bass", None).unwrap();
    assert_eq!(sum.parts, vec!["Bass".to_string()]);
    let m = String::from_utf8(files["manifest.yaml"].clone()).unwrap();
    assert!(m.contains("tuning: [-2, 0, 0, 0]"), "{m}");
}

#[test]
fn not_tab_is_an_error() {
    let xml = r#"<score-partwise><part-list/><part id="P1"><measure number="1">
      <note><pitch><step>C</step><octave>4</octave></pitch><duration>1</duration></note>
    </measure></part></score-partwise>"#;
    assert!(parse::parse(xml, "x").is_err());
}

fn tempfile_dir() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "qrs-musicxml-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn library_scan_converts_musicxml_once() {
    let dir = tempfile_dir();
    let src = dir.join("B03.musicxml");
    std::fs::write(&src, DEMO).unwrap();
    let files = crate::scan::find_song_files(std::slice::from_ref(&dir));
    let fp = dir.join("B03.feedpak");
    assert_eq!(files, vec![fp.clone()]);
    // quét lần nữa: feedpak đã có, không bị liệt kê hai lần và không chuyển lại
    let before = std::fs::metadata(&fp).unwrap().modified().unwrap();
    let files = crate::scan::find_song_files(std::slice::from_ref(&dir));
    assert_eq!(files, vec![fp.clone()]);
    assert_eq!(std::fs::metadata(&fp).unwrap().modified().unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

//! Bản thu đi kèm một bài MusicXML: tìm file, giải mã, và tự căn cho tab khớp với bản thu.
//!
//! File âm thanh đặt cạnh file MusicXML, cùng tên (`Bài.musicxml` + `Bài.mp3` / `.ogg` / `.wav` / `.flac`).
//! Căn khớp: tìm độ lệch (bản thu bắt đầu ô 1 ở giây thứ mấy) và hệ số tốc độ (bản thu nhanh/chậm hơn tốc độ ghi
//! trong tab bao nhiêu) sao cho các nốt của tab rơi đúng vào các tiếng đánh (onset) trong bản thu nhiều nhất.
//! Muốn tự chỉnh tay thì tạo `Bài.audio.json`: `{"offset": 1.25, "tempo_scale": 1.0}` (offset = giây của ô 1 trong
//! bản thu; tempo_scale > 1 nghĩa là bản thu chậm hơn tab).

use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Đuôi file âm thanh nhận được (các định dạng quickRS phát được).
pub const AUDIO_EXTS: [&str; 5] = ["mp3", "ogg", "wav", "flac", "opus"];

/// File âm thanh cạnh file MusicXML, cùng tên.
pub fn find_audio(src: &Path) -> Option<PathBuf> {
    AUDIO_EXTS
        .iter()
        .map(|e| src.with_extension(e))
        .find(|p| p.is_file())
}

/// File căn tay (`Bài.audio.json`), nếu có.
pub fn align_file(src: &Path) -> PathBuf {
    src.with_extension("audio.json")
}

/// Căn tay: (offset, tempo_scale) từ `Bài.audio.json`.
pub fn manual_align(src: &Path) -> Option<(f64, f64)> {
    let text = std::fs::read_to_string(align_file(src)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let off = v.get("offset")?.as_f64()?;
    let scale = v.get("tempo_scale").and_then(|x| x.as_f64()).unwrap_or(1.0);
    (off.is_finite() && scale > 0.5 && scale < 2.0).then_some((off, scale))
}

/// Giải mã thành mono: (mẫu, tần số lấy mẫu).
pub fn decode_mono(data: &[u8], ext: &str) -> Result<(Vec<f32>, u32), String> {
    use symphonia::core::codecs::CodecParameters;
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let mss = MediaSourceStream::new(Box::new(Cursor::new(data.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(ext);
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("{ext}: {e}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| format!("{ext}: không có kênh âm thanh"))?
        .clone();
    let Some(CodecParameters::Audio(params)) = &track.codec_params else {
        return Err(format!("{ext}: không đọc được thông số âm thanh"));
    };
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| format!("{ext}: {e}"))?;
    let mut rate = params.sample_rate.unwrap_or(0);
    let mut mono: Vec<f32> = Vec::new();
    let mut chunk: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(_) if !mono.is_empty() => break,
            Err(e) => return Err(format!("{ext}: {e}")),
        };
        if packet.track_id != track.id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                rate = buf.spec().rate();
                let ch = buf.spec().channels().count().max(1);
                chunk.clear();
                buf.copy_to_vec_interleaved(&mut chunk);
                for f in chunk.chunks_exact(ch) {
                    mono.push(f.iter().sum::<f32>() / ch as f32);
                }
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("{ext}: {e}")),
        }
    }
    if rate == 0 || mono.is_empty() {
        return Err(format!("{ext}: file âm thanh rỗng"));
    }
    Ok((mono, rate))
}

/// Bước thời gian của đường onset (giây).
const HOP: f64 = 0.01;

/// Đường onset: mức tăng năng lượng theo từng 10 ms (chỉ phần tăng), đã chuẩn hoá về [0, 1].
pub fn onset_curve(mono: &[f32], rate: u32) -> Vec<f32> {
    let hop = ((rate as f64 * HOP) as usize).max(1);
    // lọc thông cao đơn giản (lấy hiệu), để tiếng đánh dây và trống nổi hơn tiếng ngân trầm
    let mut energy: Vec<f32> = Vec::with_capacity(mono.len() / hop + 1);
    let mut prev = 0.0f32;
    for frame in mono.chunks(hop) {
        let mut e = 0.0f32;
        for &x in frame {
            let d = x - prev;
            prev = x;
            e += d * d;
        }
        energy.push((e / frame.len() as f32 + 1e-9).ln());
    }
    let mut out: Vec<f32> = vec![0.0; energy.len()];
    for i in 1..energy.len() {
        out[i] = (energy[i] - energy[i - 1]).max(0.0);
    }
    // làm mượt một chút (±20 ms) để lệch vài ms vẫn ăn điểm
    let mut sm = vec![0.0f32; out.len()];
    for i in 0..out.len() {
        let lo = i.saturating_sub(2);
        let hi = (i + 3).min(out.len());
        sm[i] = out[lo..hi].iter().cloned().fold(0.0, f32::max);
    }
    let max = sm.iter().cloned().fold(0.0f32, f32::max);
    if max > 0.0 {
        for x in &mut sm {
            *x /= max;
        }
    }
    sm
}

/// Tìm (offset, tempo_scale) tốt nhất: thời điểm trong bản thu = offset + tempo_scale × thời điểm trong tab
/// (tính từ ô 1, theo tốc độ ghi trong tab).
pub fn align(note_times: &[f64], curve: &[f32]) -> (f64, f64) {
    if note_times.is_empty() || curve.is_empty() {
        return (0.0, 1.0);
    }
    // tối đa 600 nốt, rải đều trong bài (đủ để căn, nhanh)
    let step = (note_times.len() / 600).max(1);
    let notes: Vec<f64> = note_times.iter().step_by(step).cloned().collect();
    let audio_len = curve.len() as f64 * HOP;
    let score = |off: f64, scale: f64| -> f64 {
        let mut s = 0.0f64;
        for &t in &notes {
            let at = off + scale * t;
            let i = (at / HOP).round();
            if i >= 0.0 && (i as usize) < curve.len() {
                s += curve[i as usize] as f64;
            }
        }
        s
    };
    let max_off = (audio_len * 0.5).min(40.0);
    // dò thô rồi dò tinh
    let mut best = (0.0, 1.0, f64::MIN);
    let mut scale = 0.85;
    while scale <= 1.15 + 1e-9 {
        let mut off = 0.0;
        while off <= max_off {
            let s = score(off, scale);
            if s > best.2 {
                best = (off, scale, s);
            }
            off += 0.04;
        }
        scale += 0.005;
    }
    let (o0, s0) = (best.0, best.1);
    let mut scale = s0 - 0.006;
    while scale <= s0 + 0.006 {
        let mut off = (o0 - 0.06).max(0.0);
        while off <= o0 + 0.06 {
            let s = score(off, scale);
            if s > best.2 {
                best = (off, scale, s);
            }
            off += 0.005;
        }
        scale += 0.0005;
    }
    (best.0, best.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bản thu giả: tiếng gảy tổng hợp tại các nốt, lệch 2,5 giây và chậm hơn tab 4%.
    #[test]
    fn finds_offset_and_tempo() {
        let rate = 22_050;
        // nhịp không đều (như một riff thật) để chỉ có một cách khớp
        let pattern = [0.0, 0.75, 1.0, 1.5, 2.0, 2.25, 3.0, 3.5];
        let mut tab: Vec<f64> = Vec::new();
        for bar in 0..24 {
            for p in pattern {
                tab.push((bar as f64 * 4.0 + p) * 0.5);
            }
        }
        let (off, scale) = (2.5, 1.04);
        let voices: Vec<super::super::synth::Voice> = tab
            .iter()
            .enumerate()
            .map(|(i, &t)| super::super::synth::Voice {
                time: off + scale * t,
                dur: 0.2,
                midi: 40.0 + (i % 5) as f64 * 3.0,
                dead: false,
                accent: false,
            })
            .collect();
        let total = off + scale * tab.last().unwrap() + 1.0;
        let mono = super::super::synth::render_guitar(&voices, total);
        let curve = onset_curve(&mono, rate);
        let (o, s) = align(&tab, &curve);
        assert!((o - off).abs() < 0.03, "offset {o}");
        assert!((s - scale).abs() < 0.003, "scale {s}");
    }
}

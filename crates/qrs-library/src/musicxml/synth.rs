//! Tạo âm thanh cho bài MusicXML (bài không có bản thu): tiếng guitar tổng hợp và tiếng gõ nhịp, ra file WAV.
//!
//! Tiếng guitar dùng thuật toán Karplus–Strong (dây gảy): đủ để nghe đúng nốt, đúng nhịp khi tập.

/// 22,05 kHz: đủ cho tiếng guitar tập luyện, file nhỏ bằng nửa (mỗi bè một kênh nên quan trọng).
pub const RATE: u32 = 22_050;

/// Một nốt cần phát: thời điểm (giây), độ dài (giây), cao độ MIDI, có phải nốt câm (dead note) không.
#[derive(Clone, Debug)]
pub struct Voice {
    pub time: f64,
    pub dur: f64,
    pub midi: f64,
    pub dead: bool,
    pub accent: bool,
}

/// Bộ sinh số ngẫu nhiên đơn giản, cố định hạt giống để file ra giống nhau mỗi lần chuyển.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    }
}

/// Trộn tiếng guitar của mọi nốt vào một kênh mono dài `total` giây.
pub fn render_guitar(voices: &[Voice], total: f64) -> Vec<f32> {
    let n = (total * RATE as f64).ceil() as usize + 1;
    let mut out = vec![0.0f32; n];
    let mut rng = Lcg(0x5eed_1234);
    for v in voices {
        let start = (v.time * RATE as f64).round() as usize;
        if start >= n {
            continue;
        }
        let gain = if v.accent { 0.55 } else { 0.42 };
        if v.dead {
            // nốt câm: một tiếng "tạch" ngắn
            let len = (0.035 * RATE as f64) as usize;
            for k in 0..len.min(n - start) {
                let env = 1.0 - k as f32 / len as f32;
                out[start + k] += rng.next() * env * gain * 0.6;
            }
            continue;
        }
        let freq = 440.0 * 2f64.powf((v.midi - 69.0) / 12.0);
        let period = ((RATE as f64 / freq).round() as usize).max(2);
        // nốt kêu đến hết độ dài (tối đa 4 giây), rồi tắt nhanh trong 30 ms
        let ring = v.dur.clamp(0.08, 4.0);
        let len = ((ring + 0.03) * RATE as f64) as usize;
        let release = (0.03 * RATE as f64) as usize;
        // dây thấp tắt chậm hơn dây cao một chút
        let decay = (0.996 - (freq as f32 / 20_000.0)).clamp(0.985, 0.998);
        let mut buf: Vec<f32> = (0..period).map(|_| rng.next()).collect();
        // làm mềm tiếng gảy ban đầu
        for k in 1..period {
            buf[k] = 0.5 * (buf[k] + buf[k - 1]);
        }
        let mut idx = 0usize;
        for k in 0..len.min(n - start) {
            let a = buf[idx];
            let b = buf[(idx + 1) % period];
            buf[idx] = decay * 0.5 * (a + b);
            idx = (idx + 1) % period;
            let tail = len - k;
            let env = if tail < release {
                tail as f32 / release as f32
            } else {
                1.0
            };
            out[start + k] += a * env * gain;
        }
    }
    normalize(&mut out, 0.8);
    out
}

/// Tiếng gõ nhịp: phách mạnh (đầu ô) cao hơn và to hơn.
pub fn render_click(beats: &[(f64, bool)], total: f64) -> Vec<f32> {
    let n = (total * RATE as f64).ceil() as usize + 1;
    let mut out = vec![0.0f32; n];
    let len = (0.03 * RATE as f64) as usize;
    for &(t, down) in beats {
        let start = (t * RATE as f64).round() as usize;
        let (freq, gain) = if down {
            (1_600.0f32, 0.5f32)
        } else {
            (1_000.0, 0.32)
        };
        for k in 0..len.min(n.saturating_sub(start)) {
            let x = k as f32 / RATE as f32;
            let env = (-x * 160.0).exp();
            out[start + k] += (2.0 * std::f32::consts::PI * freq * x).sin() * env * gain;
        }
    }
    out
}

fn normalize(s: &mut [f32], peak: f32) {
    let max = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if max > peak {
        let g = peak / max;
        for x in s.iter_mut() {
            *x *= g;
        }
    }
}

/// Mã hoá mono 16 bit thành file WAV.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&RATE.to_le_bytes());
    v.extend_from_slice(&(RATE * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for &x in samples {
        let i = (x.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
        v.extend_from_slice(&i.to_le_bytes());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_valid() {
        let w = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        assert_eq!(w.len(), 44 + 6);
    }

    #[test]
    fn guitar_note_makes_sound_where_it_should() {
        let s = render_guitar(
            &[Voice {
                time: 0.5,
                dur: 0.5,
                midi: 64.0,
                dead: false,
                accent: false,
            }],
            1.5,
        );
        let before: f32 = s[..(0.45 * RATE as f64) as usize].iter().map(|x| x.abs()).sum();
        let during: f32 = s[(0.5 * RATE as f64) as usize..(0.9 * RATE as f64) as usize]
            .iter()
            .map(|x| x.abs())
            .sum();
        let after: f32 = s[(1.1 * RATE as f64) as usize..].iter().map(|x| x.abs()).sum();
        assert_eq!(before, 0.0);
        assert!(during > 100.0);
        assert_eq!(after, 0.0);
    }
}

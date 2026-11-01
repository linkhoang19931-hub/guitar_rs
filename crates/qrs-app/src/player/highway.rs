//! The 3D highway (owner request, D32): Rocksmith's view, notes flying at the neck on their strings over a floor of
//! fret lanes, with the hand's positions (anchors) lit on the floor as they come. Its camera is planned over the
//! whole part (`qrs_analysis::hwcamera`) so it holds still for long stretches and moves only in gaps, instead of the
//! Python highway's (rshighway.py, removed 2026-10-13) chase of every anchor. "Invert strings" puts the low E at the
//! bottom (Rocksmith's own option); Rocksmith's default has it on top. Notes take the scorer's judgements (the
//! tab's glyph groups): a hit fills and glows in its container, fires beams up the fret posts beside it and squashes
//! into a streak; a miss passes its container with "MISS" over it. The look follows Rocksmith's notehead and tail guide (Ubisoft) and RSMods' stock colours: 3D
//! gems with a rim, fret numbers on plates on the floor, note containers on the strings, chord frames, technique
//! marks and tails (slide, bend, vibrato, tremolo); the layout and motion from the owner's reference video (neck along
//! the bottom with tall fret posts, a narrow highway rising to a vanishing point that swings with the hand, notes
//! pitching forward off it onto their strings).

use std::sync::OnceLock;
use std::time::Instant;

/// The effects' clock (seconds): a frame-sequence render's own time while one runs (so effects play frame by frame),
/// else the wall clock.
fn clock() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    crate::shotclock::shot_now().unwrap_or_else(|| START.get_or_init(Instant::now).elapsed().as_secs_f64())
}

use eframe::egui;
use egui::{Align2, Color32, CursorIcon, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use qrs_analysis::Judgement;
use qrs_analysis::hwcamera::{self, CamOpts, CamPlan, RsCamera};
use qrs_formats::Techs;
use qrs_ui::icons::{self, Icon};
use qrs_ui::paint::{self, Fill};
use qrs_ui::text::{self, Style};
use qrs_ui::theme::t;
use qrs_ui::units::px;
use qrs_ui::widgets::interact;

use super::{Chart, ipos, mid, mid_y, snap};
use crate::library::txt;

/// Seconds of highway ahead of the now-line.
const LOOK: f64 = 3.8;
/// Note speeds (neck widths per second along the floor); `SPEED_DEFAULT` shows as 100%.
pub const SPEEDS: [f32; 7] = [0.4, 0.5, 0.65, 0.8, 1.0, 1.25, 1.6];
pub const SPEED_DEFAULT: usize = 3;
/// The camera's temper: how long it holds a view before moving.
/// The camera: Rocksmith's own (a slow spring after the hand's windows, fitted to the owner's video), or the planned
/// one (holds still, moves only in gaps; D32).
pub const CAMERAS: [&str; 2] = ["Rocksmith", "Steady"];
pub const CAMERA_DEFAULT: usize = 0;

/// What the player should do after a frame of the highway: actions, and a tooltip to show.
pub struct HwOut {
    pub acts: Vec<&'static str>,
    pub tip: Option<(String, String, Pos2, Align2)>,
}

/// A note as the highway draws it (from the tab's glyphs, so it shares the scorer's groups).
struct HwNote {
    t: f64,
    string: i32,
    fret: i32,
    sus: f64,
    group: Option<usize>,
    tech: Techs,
    slide_to: i32,
    bend: f64,
    /// Its chord in `Cache::chords`.
    chord: Option<usize>,
}

/// A chord's frame (or an arpeggio's shape).
struct HwChord {
    t: f64,
    lo: i32,
    hi: i32,
    name: String,
    /// The same chord again in the same hand shape: Rocksmith draws it as an empty frame.
    repeat: bool,
    /// Open strings in it.
    opens: Vec<i32>,
    /// Fingers and frets per string (-1 none), and the strings it sounds.
    fingers: Vec<i32>,
    frets: Vec<i32>,
    strings: Vec<i32>,
    /// The hand shape's end: a chord's frame waits at the strings until then.
    hold_end: f64,
}

struct Cache {
    id: u64,
    camera: usize,
    plan: CamPlan,
    /// Rocksmith's camera over the part.
    rs: RsCamera,
    /// Sorted by time; chord notes flattened.
    notes: Vec<HwNote>,
    chords: Vec<HwChord>,
    arps: Vec<HwChord>,
    /// (start, end, fret, width).
    anchors: Vec<(f64, f64, i32, i32)>,
    /// (time, is a bar line).
    beats: Vec<(f64, bool)>,
    /// Each scorer group's note time.
    group_t: Vec<f64>,
    /// Where sections start (an orange line across the highway).
    sections: Vec<f64>,
    /// The song's note speed at this difficulty (Rocksmith's DynamicVisualDensity; 1.3 at full difficulty by default,
    /// lower = faster).
    density: f32,
}

pub struct Highway {
    /// Low E at the bottom (Rocksmith's "invert strings"); off = low E on top as in Rocksmith.
    pub invert: bool,
    /// Index in `SPEEDS`.
    pub speed: usize,
    /// Index in `CAMERAS`.
    pub camera: usize,
    /// Chữ ghi tên kỹ thuật (BEND, HAMMER-ON, PALM MUTE…) phía trên nốt sắp tới, cho người mới học.
    pub labels: bool,
    cache: Option<Cache>,
    /// When each scorer group was judged (for the burst / cross), cleared when its judgement goes (a seek back).
    judged: Vec<Option<(Judgement, f64)>>,
    /// The toolbar's last hover or change (it's see-through at rest, solid while in use).
    tools_used: Option<Instant>,
}

/// The planned camera's options (the "Steady" camera).
fn cam_opts() -> CamOpts {
    CamOpts {
        max_frets: 12.0,
        comfort_frets: 8.0,
        min_hold: 10.0,
        move_cost: 1.6,
        ..CamOpts::default()
    }
}

impl Highway {
    pub fn new(invert: bool, speed: usize, camera: usize, labels: bool) -> Highway {
        Highway {
            invert,
            labels,
            speed: speed.min(SPEEDS.len() - 1),
            camera: camera.min(CAMERAS.len() - 1),
            cache: None,
            judged: Vec::new(),
            tools_used: None,
        }
    }

    /// One step faster (+1) or slower (-1); false at the end of the range.
    pub fn step_speed(&mut self, d: i32) -> bool {
        let n = (self.speed as i32 + d).clamp(0, SPEEDS.len() as i32 - 1) as usize;
        self.tools_used = Some(Instant::now());
        std::mem::replace(&mut self.speed, n) != n
    }

    /// One step livelier (+1) or steadier (-1); false at the end of the range.
    pub fn step_camera(&mut self, d: i32) -> bool {
        let n = (self.camera as i32 + d).clamp(0, CAMERAS.len() as i32 - 1) as usize;
        self.tools_used = Some(Instant::now());
        std::mem::replace(&mut self.camera, n) != n
    }

    /// Back to the default speed or camera.
    pub fn reset(&mut self, camera: bool) {
        self.tools_used = Some(Instant::now());
        if camera {
            self.camera = CAMERA_DEFAULT;
        } else {
            self.speed = SPEED_DEFAULT;
        }
    }

    /// The note speed as a percentage of the default.
    pub fn speed_pct(&self) -> i64 {
        (SPEEDS[self.speed] / SPEEDS[SPEED_DEFAULT] * 100.0).round() as i64
    }

    fn cache(&mut self, chart: &Chart) -> &Cache {
        if self
            .cache
            .as_ref()
            .is_none_or(|c| c.id != chart.id || c.camera != self.camera)
        {
            let shape_at = |t: f64| {
                let i = chart.shape_starts.partition_point(|&s| s <= t + 1e-3);
                (i > 0).then(|| &chart.shapes[i - 1]).filter(|s| t < s.end - 1e-3)
            };
            let any_hd = chart.chords.iter().any(|c| c.high_density);
            let mut chords: Vec<HwChord> = Vec::with_capacity(chart.chords.len());
            for (i, ch) in chart.chords.iter().enumerate() {
                let fretted = ch.notes.iter().map(|n| n.fret).filter(|&f| f > 0);
                // the chart's own flag (Rocksmith's high density); charts without any flags: the same chord again in the
                // same hand shape
                let repeat = if any_hd {
                    ch.high_density
                } else {
                    i > 0 && {
                        let pv = &chart.chords[i - 1];
                        pv.chord_id == ch.chord_id
                            && shape_at(ch.time).is_some_and(|s| pv.time >= s.start - 1e-3)
                    }
                };
                chords.push(HwChord {
                    t: ch.time,
                    lo: fretted.clone().min().unwrap_or(0),
                    hi: fretted.max().unwrap_or(0),
                    name: ch.name.clone(),
                    repeat,
                    opens: ch
                        .notes
                        .iter()
                        .filter(|n| n.fret == 0)
                        .map(|n| n.string)
                        .collect(),
                    fingers: ch.fingers.clone(),
                    frets: ch.frets.clone(),
                    strings: ch.notes.iter().map(|n| n.string).collect(),
                    hold_end: shape_at(ch.time).map_or(ch.time, |s| s.end),
                });
            }
            chords.sort_by(|a, b| a.t.total_cmp(&b.t));
            let arps = chart
                .shapes
                .iter()
                .filter(|s| s.arpeggio)
                .map(|s| {
                    let fretted = s.frets.iter().copied().filter(|&f| f > 0);
                    HwChord {
                        t: s.start,
                        lo: fretted.clone().min().unwrap_or(0),
                        hi: fretted.max().unwrap_or(0),
                        name: s.name.clone(),
                        repeat: false,
                        opens: Vec::new(),
                        fingers: Vec::new(),
                        frets: s.frets.clone(),
                        strings: Vec::new(),
                        hold_end: s.end,
                    }
                })
                .collect();
            let mut notes: Vec<HwNote> = chart
                .glyphs
                .iter()
                .map(|g| HwNote {
                    t: g.t,
                    string: g.string,
                    fret: g.fret,
                    sus: g.sus,
                    group: g.group,
                    tech: g.tech,
                    slide_to: g.slide_to,
                    bend: g.bend,
                    chord: g
                        .in_chord
                        .then(|| chords.partition_point(|c| c.t < g.t - 3e-3))
                        .filter(|&k| chords.get(k).is_some_and(|c| (c.t - g.t).abs() < 3e-3)),
                })
                .collect();
            notes.sort_by(|a, b| a.t.total_cmp(&b.t));
            let mut an: Vec<_> = chart
                .arr
                .anchors
                .iter()
                .map(|a| (a.start, a.end, a.fret, a.width))
                .collect();
            an.sort_by(|a, b| a.0.total_cmp(&b.0));
            // each window ends where the next starts (charts stitched from difficulty levels can run ends past their
            // successors)
            for i in 0..an.len() {
                let next = an.get(i + 1).map_or(f64::INFINITY, |a| a.0);
                an[i].1 = next.max(an[i].0);
            }
            let ev = hwcamera::events(
                &notes
                    .iter()
                    .map(|n| (n.t, n.fret, n.t + n.sus))
                    .collect::<Vec<_>>(),
                &an.iter().map(|a| (a.0, a.2, a.3)).collect::<Vec<_>>(),
            );
            let plan = CamPlan::new(&ev, chart.length, &cam_opts());
            // (charts without phrases: section starts)
            let phrases: Vec<f64> = if chart.arr.phrases.is_empty() {
                chart.sections.iter().map(|s| s.start).collect()
            } else {
                chart.arr.phrases.clone()
            };
            let rs = RsCamera::new(&an, &phrases, chart.length);
            self.cache = Some(Cache {
                id: chart.id,
                camera: self.camera,
                plan,
                rs,
                notes,
                chords,
                arps,
                anchors: an,
                beats: chart.beats.iter().map(|b| (b.time, b.measure >= 0)).collect(),
                sections: chart.sections.iter().map(|s| s.start).collect(),
                density: chart.arr.visual_density_at(chart.arr.mastery),
                group_t: chart.glyphs.iter().fold(Vec::new(), |mut v, g| {
                    if let Some(k) = g.group {
                        if v.len() <= k {
                            v.resize(k + 1, f64::NAN);
                        }
                        if v[k].is_nan() {
                            v[k] = g.t;
                        }
                    }
                    v
                }),
            });
        }
        self.cache.as_ref().expect("cached")
    }

    /// The camera's planned moves in this part (for the toast).
    pub fn moves(&mut self, chart: &Chart) -> usize {
        self.cache(chart).plan.moves()
    }

    /// Note when each group got its judgement, so the burst or cross plays once from that moment.
    fn track(&mut self, states: &[Option<Judgement>], now: f64) {
        let group_t = self.cache.as_ref().map(|c| c.group_t.clone()).unwrap_or_default();
        self.judged.resize(states.len(), None);
        // many at once is a load or a seek, not playing: those are dated by their notes' times (their effects are over,
        // or as far along as they'd be)
        let fresh = self
            .judged
            .iter()
            .zip(states)
            .filter(|(j, s)| s.is_some() && j.is_none_or(|(k, _)| Some(k) != **s))
            .count();
        let at = |i: usize| {
            if fresh <= 8 {
                return clock();
            }
            let ago = group_t.get(i).map_or(10.0, |t| {
                if t.is_nan() {
                    10.0
                } else {
                    (now - t).clamp(0.0, 10.0)
                }
            });
            clock() - ago
        };
        for (i, (j, s)) in self.judged.iter_mut().zip(states).enumerate() {
            match (*s, *j) {
                (None, _) => *j = None,
                (Some(k), None) => *j = Some((k, at(i))),
                (Some(k), Some((was, _))) if k != was => *j = Some((k, at(i))),
                _ => {}
            }
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut Ui,
        chart: &Chart,
        states: &[Option<Judgement>],
        now: f64,
        well: Rect,
    ) -> HwOut {
        self.cache(chart);
        self.track(states, now);
        let marks: Vec<Option<(Judgement, f32)>> = self
            .judged
            .iter()
            .map(|j| j.map(|(k, at)| (k, (clock() - at).max(0.0) as f32)))
            .collect();
        let invert = self.invert;
        let labels = self.labels;
        let speed = SPEEDS[self.speed] as f64;
        let n_str = chart.n_strings.clamp(4, 6);
        let camera = self.camera;
        let c = self.cache(chart);
        let p = ui.painter().with_clip_rect(well);
        // the camera as (fret wire at the screen's middle, frets across)
        let cam = if camera == 0 {
            c.rs.at(now)
        } else {
            let v = c.plan.at(now);
            let (fl, fr) = (hwcamera::fret_at(v.l), hwcamera::fret_at(v.r));
            let across = (fr - fl).clamp(10.55, 14.0);
            (((fl + fr) / 2.0 + (0.5 - 0.459) * across) as f32, across as f32)
        };
        draw(&p, well, c, cam, now, n_str, invert, speed, &marks, labels);
        let out = self.tools(ui, well);
        ui.ctx().request_repaint();
        out
    }

    /// The highway's toolbar, floating at its top right like the tab's: note speed and the camera's temper as sunken
    /// pills (− amount +), then invert strings as a round key (the RS Highway key, back to the tab, is the player's). See-through at
    /// rest, solid while the pointer is on it or just after a change.
    fn tools(&mut self, ui: &mut Ui, well: Rect) -> HwOut {
        let ctx = ui.ctx().clone();
        let th = t();
        let mut out = HwOut {
            acts: Vec::new(),
            tip: None,
        };
        let (key, pill, gap, pad, div_gap) = (px(26.0), px(112.0), px(6.0), px(5.0), px(8.0));
        let label_w: f32 = ["Speed", "Camera"]
            .iter()
            .map(|l| text::width(&ctx, l, Style::new(11.0, 600, th.caption)).ceil() + px(7.0))
            .sum();
        let rs_w = super::PlayerView::rs_highway_w(&ctx);
        let w = snap(
            2.0 * pad
                + rs_w
                + div_gap
                + label_w
                + 2.0 * pill
                + gap
                + 2.0 * div_gap
                + px(1.0)
                + 2.0 * key
                + px(8.0),
        );
        let h = snap(key + 2.0 * pad);
        let plate = Rect::from_min_size(ipos(well.max.x - px(16.0) - w, well.min.y + px(12.0)), vec2(w, h));
        let over = ctx
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|h| plate.contains(h));
        if over {
            self.tools_used = Some(Instant::now());
        }
        let busy = self.tools_used.is_some_and(|t| t.elapsed().as_secs_f32() < 1.4);
        let solid = ctx.animate_bool_with_time(egui::Id::new("hw-tools"), over || busy, 0.2);
        let mut tu = ui.new_child(egui::UiBuilder::new().max_rect(plate));
        tu.multiply_opacity(0.4 + 0.6 * solid);
        let p = tu.painter().clone();
        paint::floating(&p, plate, Some(12.0));
        let cy = mid_y(plate);
        let tip_at = |r: Rect| ipos(r.max.x, plate.max.y + px(8.0));
        let mut x = plate.min.x + pad;
        // the RS Highway key first, lit (back to the tab)
        {
            let r = Rect::from_min_size(ipos(x, cy - snap(key / 2.0)), vec2(rs_w, key));
            let resp = interact(&tu, r, ("hw-tool", "rs-highway"), Sense::click());
            super::PlayerView::draw_rs_highway(&p, r, true, resp.hovered());
            if resp.hovered() {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
                out.tip = Some(("Back to the tab".into(), "V".into(), tip_at(r), Align2::RIGHT_TOP));
            }
            if resp.clicked() {
                out.acts.push("view");
            }
            x = r.max.x + div_gap;
        }
        let steppers: [(&str, &str, String, [(&str, &'static str); 2], &'static str); 2] = [
            (
                "Speed",
                "Note speed",
                format!("{}%", self.speed_pct()),
                [("Slower notes", "hw_slower"), ("Faster notes", "hw_faster")],
                "hw_speed_reset",
            ),
            (
                "Camera",
                "Camera: Steady holds a view longest, Lively moves more to stay close",
                CAMERAS[self.camera].to_string(),
                [
                    ("Steadier camera", "hw_steadier"),
                    ("Livelier camera", "hw_livelier"),
                ],
                "hw_camera_reset",
            ),
        ];
        for (i, (label, what, value, keys, reset)) in steppers.iter().enumerate() {
            // what it sets: a plain label before the pill
            let lw = text::width(&ctx, label, Style::new(11.0, 600, th.caption)).ceil() + px(4.0);
            let lab = Rect::from_min_size(ipos(x, cy - snap(key / 2.0)), vec2(lw, key));
            txt(
                &p,
                label,
                ipos(lab.min.x, cy),
                11.0,
                th.caption,
                600,
                Align2::LEFT_CENTER,
                None,
            );
            let r = Rect::from_min_size(ipos(lab.max.x + px(3.0), lab.min.y), vec2(pill, key));
            paint::inset(&p, r, None, 3.0, Fill::Base);
            // − and + at the pill's ends, the amount between (a click on it puts it back)
            let minus = Rect::from_min_size(r.min, Vec2::splat(key));
            let plus = Rect::from_min_size(ipos(r.max.x - key, r.min.y), Vec2::splat(key));
            for (b, ic, (tip, act)) in [(minus, Icon::Minus, keys[0]), (plus, Icon::Plus, keys[1])] {
                let resp = interact(&tu, b, ("hw-tool", act), Sense::click());
                let ink = if resp.is_pointer_button_down_on() {
                    th.accent
                } else if resp.hovered() {
                    ctx.set_cursor_icon(CursorIcon::PointingHand);
                    p.circle_filled(mid(b), snap(key / 2.0 - px(3.0)), th.line);
                    th.text
                } else {
                    th.caption
                };
                icons::icon(&p, ic, mid(b), 12.0, ink);
                if resp.clicked() {
                    out.acts.push(act);
                }
                if resp.hovered() {
                    out.tip = Some((tip.to_string(), String::new(), tip_at(b), Align2::RIGHT_TOP));
                }
            }
            let mid_r = Rect::from_min_max(pos2(minus.max.x, r.min.y), pos2(plus.min.x, r.max.y));
            let resp = interact(&tu, mid_r, ("hw-tool-read", *label), Sense::click());
            let ink = if resp.hovered() { th.text } else { th.accent_ink };
            txt(&p, value, mid(mid_r), 12.0, ink, 800, Align2::CENTER_CENTER, None);
            if resp.hovered() {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
                let back = if i == 0 { "click: 100%" } else { "click: Normal" };
                out.tip = Some((what.to_string(), back.into(), tip_at(mid_r), Align2::RIGHT_TOP));
            }
            if resp.clicked() {
                out.acts.push(reset);
            }
            x = r.max.x + if i == 0 { gap } else { 0.0 };
        }
        x = snap(x + div_gap);
        p.rect_filled(
            Rect::from_min_size(ipos(x, plate.min.y + px(8.0)), vec2(px(1.0), snap(h - px(16.0)))),
            0.0,
            th.line,
        );
        x = snap(x + px(1.0) + div_gap);
        let toggles = [
            (Icon::Invert, self.invert, "Invert strings", "I", "hw_invert"),
            (
                Icon::Lesson,
                self.labels,
                "Technique names over the notes",
                "K",
                "hw_labels",
            ),
        ];
        for (icon, on, label, keycap, act) in toggles {
            let r = Rect::from_min_size(ipos(x, cy - snap(key / 2.0)), Vec2::splat(key));
            let resp = interact(&tu, r, ("hw-tool", act), Sense::click());
            let depth = if resp.is_pointer_button_down_on() {
                4.0
            } else {
                3.0
            };
            paint::inset(&p, r, None, depth, Fill::Base);
            if on {
                paint::ring(&p, r, None, 1.5, th.accent, true);
            }
            let ink = if on {
                th.accent_ink
            } else if resp.hovered() {
                th.text
            } else {
                th.caption
            };
            icons::icon(&p, icon, mid(r), 13.0, ink);
            if resp.hovered() {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
                out.tip = Some((label.into(), keycap.into(), tip_at(r), Align2::RIGHT_TOP));
            }
            if resp.clicked() {
                out.acts.push(act);
            }
            x = snap(r.max.x + px(4.0));
        }
        out
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

// Measured from the owner's reference video (Rocksmith 2014, "Take Me Out", 1280x720; scratch measure/ report,
// 2026-10-16). The layout below is in that frame's pixels and stretched onto the well.

/// The strings lit (RSMods' stock colours) and unlit (sampled), low E first.
const STRING_ON: [Color32; 6] = [
    rgb(0xff, 0x4f, 0x5a),
    rgb(0xe2, 0xc1, 0x02),
    rgb(0x1d, 0xac, 0xf9),
    rgb(0xff, 0x92, 0x16),
    rgb(0x3f, 0xcc, 0x0c),
    rgb(0xc8, 0x25, 0xed),
];
const LANE_LIT: Color32 = rgb(40, 230, 240);

const LANE_FAR: Color32 = rgb(30, 120, 160);
const LANE_DIM: Color32 = rgb(18, 34, 46);
const FLOOR_LIT: Color32 = rgb(16, 59, 93);

const BAR: Color32 = rgb(236, 175, 15);
const MEASURE: Color32 = rgb(70, 100, 120);

const INLAY_IN: Color32 = rgb(100, 72, 16);
const INLAY_OUT: Color32 = rgb(20, 50, 45);
const NUM_IN: Color32 = rgb(205, 158, 45);
const NUM_OUT: Color32 = rgb(23, 49, 46);
const TILE: Color32 = rgb(90, 170, 210);
const NUM_FLOOR_OUT: Color32 = rgb(40, 110, 120);
const BEAM_CORE: Color32 = rgb(255, 240, 230);

const LILAC: Color32 = rgb(0xb4, 0x8c, 0xff);
const CYAN: Color32 = rgb(70, 215, 215);

/// The vanishing point zoomed out (the camera pans and zooms; zooming in it rises and tilts a little, see Geo::vp).
const VP: (f32, f32) = (781.2, 184.7);
/// String rows fan out to the right: slope ROW_K0 + ROW_K1 * r (r = 0 the farthest row).
const ROW_K0: f32 = 0.0428;
const ROW_K1: f32 = 0.0045;
/// Where the floor's lane lines would meet the posts (below the strings).
const FLOOR_Y: f32 = 700.0;
/// Notes: y = vpy + (y_string - vpy) * T / (T + τ), T seconds at the default speed.
const T_NOTE: f64 = 0.6;
/// How far ahead notes show, and where the floor fades into the dark.
const FOG_Y: f32 = 230.0;
/// How long before its note a container shows, growing to the note's size.
const CONTAINER: f64 = 1.0;
/// The last stretch in which a note leaves its lane and drops onto its string (seconds).
const LAND: f64 = 0.3;
/// A hit (seconds since judged): lit and swelling, a bloom, a squash streak along the string, gone; the beams on the
/// fret's two wires meanwhile.
const HIT_BLOOM: f32 = 0.2;
const HIT_GONE: f32 = 0.3;
/// "MISS" / "LATE" / "EARLY": on the string at the fret, still, fading out over the end.
const WORD: f32 = 0.33;

/// A string colour's shade at level u (0..1): our own ramp, near black, through a deep tone and the saturated colour
/// (at 0.6, a touch brighter than the string), to a pale tint and near white.
fn lut(col: Color32, u: f32) -> Color32 {
    let u = u.clamp(0.0, 1.0);
    let sat = shade(col, 1.08);
    let keys = [
        (0.0, shade(col, 0.1)),
        (0.3, shade(col, 0.45)),
        (0.6, sat),
        (0.82, mix(sat, Color32::WHITE, 0.45)),
        (1.0, mix(sat, Color32::WHITE, 0.9)),
    ];
    for w in keys.windows(2) {
        let ((u0, c0), (u1, c1)) = (w[0], w[1]);
        if u <= u1 {
            return mix(c0, c1, (u - u0) / (u1 - u0));
        }
    }
    keys[4].1
}

fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// `c` scaled toward black (k < 1) or white (k > 1).
fn shade(c: Color32, k: f32) -> Color32 {
    let f = |v: u8| {
        if k <= 1.0 {
            (v as f32 * k) as u8
        } else {
            (v as f32 + (255.0 - v as f32) * (k - 1.0).min(1.0)) as u8
        }
    };
    rgb(f(c.r()), f(c.g()), f(c.b()))
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgba_unmultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

/// A quad with a colour per corner (a gradient).
fn quad_grad(p: &Painter, pts: [Pos2; 4], cols: [Color32; 4]) {
    let mut m = egui::Mesh::default();
    for (pt, c) in pts.iter().zip(cols) {
        m.colored_vertex(*pt, c);
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(Shape::mesh(m));
}

fn quad(p: &Painter, pts: [Pos2; 4], fill: Color32) {
    p.add(Shape::convex_polygon(pts.to_vec(), fill, Stroke::NONE));
}

/// A soft line: a core `w` px wide at `col`, feathered out by `feather` px each side to nothing (a smooth falloff,
/// not stacked strokes).
fn soft_line(p: &Painter, a: Pos2, b: Pos2, w: f32, feather: f32, col: Color32) {
    let d = b - a;
    let len = d.length();
    if len < 0.01 || col.a() == 0 {
        return;
    }
    let n = vec2(-d.y, d.x) / len;
    let (hw, fw) = (w / 2.0, w / 2.0 + feather.max(0.5));
    let mut m = egui::Mesh::default();
    for (off, c) in [
        (-fw, Color32::TRANSPARENT),
        (-hw, col),
        (hw, col),
        (fw, Color32::TRANSPARENT),
    ] {
        m.colored_vertex(a + n * off, c);
        m.colored_vertex(b + n * off, c);
    }
    for k in 0..3u32 {
        let (i0, i1, i2, i3) = (2 * k, 2 * k + 1, 2 * k + 2, 2 * k + 3);
        m.add_triangle(i0, i1, i3);
        m.add_triangle(i0, i3, i2);
    }
    p.add(Shape::mesh(m));
}

/// A soft halo `width` px wide around a convex quad, `col` at its edge fading to nothing.
fn feather_rect(p: &Painter, pts: [Pos2; 4], width: f32, col: Color32) {
    if width < 0.5 || col.a() == 0 {
        return;
    }
    let c = pos2(
        (pts[0].x + pts[1].x + pts[2].x + pts[3].x) / 4.0,
        (pts[0].y + pts[1].y + pts[2].y + pts[3].y) / 4.0,
    );
    let out: Vec<Pos2> = pts.iter().map(|q| *q + (*q - c).normalized() * width).collect();
    for k in 0..4 {
        let m = (k + 1) % 4;
        quad_grad(
            p,
            [pts[k], pts[m], out[m], out[k]],
            [col, col, Color32::TRANSPARENT, Color32::TRANSPARENT],
        );
    }
}

/// A line with a soft glow under it.
fn glow_line(p: &Painter, a: Pos2, b: Pos2, w: f32, c: Color32, a_core: f32) {
    p.line_segment([a, b], Stroke::new(w * 4.0, alpha(c, 0.1 * a_core)));
    p.line_segment([a, b], Stroke::new(w * 2.0, alpha(c, 0.22 * a_core)));
    p.line_segment([a, b], Stroke::new(w, alpha(c, a_core)));
}

fn has(t: Techs, f: Techs) -> bool {
    t.0 & f.0 != 0
}

fn smooth(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// The view (all in the reference frame's 1280x720 pixels, `to` maps onto the well): the neck's strings as rows
/// across the bottom, fanning out to the right; frets evenly spaced, the posts leaning a little; every note flying
/// straight from the vanishing point (which moves a little with the zoom) to its container on its string.
struct Geo {
    w: Rect,
    /// The fret wire at the screen's middle (the camera puts the hand's windows' middle at 0.459 of the width), and
    /// pixels per fret.
    fc: f64,
    pf: f32,
    n_str: usize,
    invert: bool,
    /// The highway's time scale (seconds; larger = notes spread out more).
    t: f64,
    now: f64,
}

impl Geo {
    fn to(&self, p: Pos2) -> Pos2 {
        pos2(
            self.w.min.x + p.x * self.w.width() / 1280.0,
            self.w.min.y + p.y * self.w.height() / 720.0,
        )
    }
    /// Pixels in the reference frame to pixels here (for sizes).
    fn sz(&self, v: f32) -> f32 {
        v * (self.w.width() / 1280.0).min(self.w.height() / 720.0)
    }
    /// The vanishing point: the camera rises and tilts as it zooms in, so it moves from (781, 185) zoomed out (82 px
    /// a fret) to about (765, 230) zoomed in (121 px a fret).
    fn vp(&self) -> Pos2 {
        let z = ((self.pf - 82.0) / (121.0 - 82.0)).clamp(0.0, 1.0);
        pos2(VP.0 - 16.0 * z, VP.1 + 45.0 * z)
    }
    /// A string's row, 0 = farthest (the low E unless inverted); a 4-string bass spreads over the same height.
    fn row(&self, s: i32) -> f32 {
        let n = self.n_str as i32;
        let s = s.clamp(0, n - 1);
        let r = if self.invert { n - 1 - s } else { s } as f32;
        r * 5.0 / (n as f32 - 1.0)
    }
    /// The gap between rows at the middle: it grows with the zoom (31.7 px at 103.5 px a fret, 38.5 at ~120).
    fn gap(&self) -> f32 {
        0.31 * self.pf
    }
    /// Row r's y at x: the nearest row stays put (668 at x=640), the others spread with the zoom; rows fan out to the
    /// right, the nearer ones more.
    fn row_y(&self, r: f32, x: f32) -> f32 {
        (if self.n_str <= 4 { 668.0 } else { 648.0 }) - (5.0 - r) * self.gap()
            + (ROW_K0 + ROW_K1 * r) * (x - 640.0)
    }
    /// The x where fret wire `f` (fractional) crosses the bottom of the posts.
    fn x_bot(&self, f: f64) -> f32 {
        640.0 + ((f - self.fc) as f32) * self.pf
    }
    /// The fret line `f` at height y: posts lean out a little toward their tops (0.027 of their offset from x=650).
    fn fret_x(&self, f: f64, y: f32) -> f32 {
        let xb = self.x_bot(f);
        let yb = self.row_y(5.3, xb);
        let yt = self.row_y(-0.45, xb);
        xb + 0.027 * (xb - 650.0) * (yb - y) / (yb - yt)
    }
    /// Fret position f on row r (fractional).
    fn on_row(&self, f: f64, r: f32) -> Pos2 {
        let mut x = self.x_bot(f);
        for _ in 0..3 {
            x = self.fret_x(f, self.row_y(r, x));
        }
        pos2(x, self.row_y(r, x))
    }
    /// The shrink toward the vanishing point `u` seconds ahead.
    fn g(&self, u: f64) -> f32 {
        (self.t / (self.t + u.max(0.0))) as f32
    }
    fn toward(&self, p: Pos2, g: f32) -> Pos2 {
        self.vp() + (p - self.vp()) * g
    }
    /// The floor under fret line f, `u` seconds ahead.
    fn floor(&self, f: f64, u: f64) -> Pos2 {
        let base = pos2(self.fret_x(f, FLOOR_Y), FLOOR_Y);
        self.toward(base, self.g(u))
    }
    /// The floor line f, cut where it meets the farthest string (it starts there) and where it fades.
    fn floor_line(&self, f: f64, u0: f64, u1: f64) -> (Pos2, Pos2) {
        let start_g = {
            let base = pos2(self.fret_x(f, FLOOR_Y), FLOOR_Y);
            let y0 = self.row_y(0.0, base.x) - 2.0;
            ((y0 - self.vp().y) / (FLOOR_Y - self.vp().y)).clamp(0.0, 1.0)
        };
        let ga = self.g(u0).min(start_g);
        let gb = self.g(u1).min(start_g);
        let base = pos2(self.fret_x(f, FLOOR_Y), FLOOR_Y);
        (self.toward(base, ga), self.toward(base, gb))
    }
    /// How far the floor is visible (its fade into the dark): 1 near, 0 at the fog.
    fn fog(&self, y: f32) -> f32 {
        // fades out over the last stretch before the floor's far end (never a hard edge there)
        let far = self.vp().y + (FLOOR_Y - self.vp().y) * self.g(LOOK);
        let u = ((y - far.max(FOG_Y)) / 110.0).clamp(0.0, 1.0);
        u * u * (3.0 - 2.0 * u)
    }
    /// A note's container on its string: 0.58 of the fret wide, 0.8 of the string gap tall.
    fn container(&self, s: i32, f: i32) -> Rect {
        let c = self.on_row(f as f64 - 0.5, self.row(s));
        let w = 0.55 * self.pf;
        let h = (self.gap() * 0.9).min(0.29 * self.pf);
        Rect::from_center_size(c, vec2(w, h))
    }
    /// An open string's bar on its string across frets lo..=hi.
    fn open_bar(&self, s: i32, lo: i32, hi: i32) -> Rect {
        let r = self.row(s);
        let (a, b) = (
            self.on_row((lo - 1) as f64 - 0.6, r),
            self.on_row(hi as f64 - 0.15, r),
        );
        let hh = if self.n_str <= 4 { 6.0 } else { 2.5 };
        Rect::from_x_y_ranges(a.x..=b.x, (a.y + b.y) / 2.0 - hh..=(a.y + b.y) / 2.0 + hh)
    }
    /// Where a note is `u` seconds ahead: riding its lane (over the floor line at fret position `f`, higher for the
    /// farther strings, as Rocksmith's notes hover on their stems), then in the last LAND seconds pitching forward and
    /// dropping onto its string at `home`.
    fn ride(&self, home: Pos2, f: f64, row: f32, u: f64) -> Pos2 {
        let lane = |u: f64| {
            let base = pos2(
                self.fret_x(f, FLOOR_Y),
                FLOOR_Y - (40.0 + 0.95 * self.gap() * (5.0 - row)),
            );
            self.toward(base, self.g(u))
        };
        if u >= LAND {
            return lane(u);
        }
        let t = (1.0 - u.max(0.0) / LAND) as f32;
        let from = lane(LAND);
        from + (home - from) * t
    }
    /// A note `u` seconds ahead: its box riding its lane in from the vanishing point and dropping into its container,
    /// and its roll (far notes lean right-end-up, flattening by 0.2 s out) and how much of its top shows.
    fn flying(&self, home: Rect, f: f64, row: f32, u: f64) -> (Rect, f32, f32) {
        let g = self.g(u);
        let c = self.ride(home.center(), f, row, u);
        let r = Rect::from_center_size(c, home.size() * g);
        let roll = 35.0 * (((u - 0.25) / 1.2).max(0.0) as f32).powf(1.6);
        // a lying box shows its top: as tall as the face far off, gone once it stands on the string
        let top = r.height() * (((u - 0.04) / 0.6).clamp(0.0, 1.0) as f32) * 0.15;
        (r, roll.min(60.0).to_radians(), top)
    }
}

/// A note's gem: a glowing frame in its string's colour, saturated, a darker outer edge and a bright top edge, around a
/// dark see-through window split by a bar along the string (目), a soft halo; a sliver of top while it lies on the
/// highway; rolled by `roll` (radians, right end up). `lit` (0..1): a hit, the frame hotter and the bar near white.
#[allow(clippy::too_many_arguments)]
fn block(p: &Painter, g: &Geo, r: Rect, roll: f32, top: f32, col: Color32, a: f32, lit: f32) {
    let c = r.center();
    let (sin, cos) = (-roll).sin_cos();
    let rot = |v: Vec2| g.to(c + vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos));
    let (hw, hh) = (r.width() / 2.0, r.height() / 2.0);
    let o = [
        rot(vec2(-hw, -hh)),
        rot(vec2(hw, -hh)),
        rot(vec2(hw, hh)),
        rot(vec2(-hw, hh)),
    ];
    let px_h = g.sz(r.height());
    feather_rect(
        p,
        o,
        px_h * (0.35 + 0.35 * lit),
        alpha(lut(col, 0.6), (0.16 + 0.34 * lit) * a),
    );
    // the top face, a little to the right: it reads as a solid block (taller while it lies on the highway)
    let tf = top.max(r.height() * 0.3);
    let sh = hw * 0.2;
    quad(
        p,
        [
            rot(vec2(-hw, -hh)),
            rot(vec2(hw, -hh)),
            rot(vec2(hw + sh, -hh - tf)),
            rot(vec2(-hw + sh, -hh - tf)),
        ],
        alpha(lut(col, 0.45 + 0.25 * lit), a),
    );
    let body = lut(col, 0.6 + 0.15 * lit);
    p.add(Shape::convex_polygon(o.to_vec(), alpha(body, a), Stroke::NONE));
    if px_h >= 7.0 {
        // one dark window (bright when lit, with a bar along the string)
        let (bx, by) = (hw * 0.2, hh * 0.38);
        let w = [
            rot(vec2(-hw + bx, -hh + by)),
            rot(vec2(hw - bx, -hh + by)),
            rot(vec2(hw - bx, hh - by)),
            rot(vec2(-hw + bx, hh - by)),
        ];
        p.add(Shape::convex_polygon(
            w.to_vec(),
            alpha(lut(col, 0.08 + 0.4 * lit), 0.9 * a),
            Stroke::NONE,
        ));
        if lit > 0.0 {
            p.line_segment(
                [rot(vec2(-hw + bx, 0.0)), rot(vec2(hw - bx, 0.0))],
                Stroke::new(g.sz(hh * 0.4), alpha(lut(col, 0.8), lit * a)),
            );
        }
    }
    p.add(Shape::closed_line(
        o.to_vec(),
        Stroke::new(g.sz(1.2).max(0.8), alpha(lut(col, 0.75), a)),
    ));
}

/// An open string's note: a tapered bar ("cigar") across the hand's window in its string's colour, a dark core, a
/// bright rim, a soft glow.
fn cigar(p: &Painter, g: &Geo, r: Rect, col: Color32, a: f32) {
    let sr = Rect::from_min_max(g.to(r.min), g.to(r.max));
    let (cy, hh, taper) = (sr.center().y, sr.height() / 2.0, sr.width() * 0.12);
    let top = vec![
        pos2(sr.min.x, cy),
        pos2(sr.min.x + taper, cy - hh),
        pos2(sr.max.x - taper, cy - hh),
        pos2(sr.max.x, cy),
    ];
    let bottom = vec![
        pos2(sr.min.x, cy),
        pos2(sr.max.x, cy),
        pos2(sr.max.x - taper, cy + hh),
        pos2(sr.min.x + taper, cy + hh),
    ];
    feather_rect(
        p,
        [top[1], top[2], bottom[2], bottom[3]],
        hh * 1.5,
        alpha(col, 0.5 * a),
    );
    p.add(Shape::convex_polygon(top, alpha(col, a), Stroke::NONE));
    p.add(Shape::convex_polygon(
        bottom,
        alpha(shade(col, 0.45), a),
        Stroke::NONE,
    ));
}

/// The technique's mark on a box (Rocksmith's notehead guide). `r` is in screen pixels.
fn face_icon(p: &Painter, r: Rect, tech: Techs, col: Color32, a: f32) {
    let c = r.center();
    let s = r.height() * 0.3;
    if s < 2.0 {
        return;
    }
    let white = alpha(Color32::WHITE, a);
    let (w, h) = (r.width(), r.height());
    // hammer-on ▼, pull-off ▲: on the gem's face, 0.6 of its width, 1.2 of its height, near white, a dark outline
    if has(tech, Techs::HAMMER_ON) || has(tech, Techs::PULL_OFF) {
        let hw = w * 0.3;
        let ink = alpha(rgb(245, 240, 225), a);
        let pts = if has(tech, Techs::HAMMER_ON) {
            vec![
                pos2(c.x - hw, r.min.y),
                pos2(c.x + hw, r.min.y),
                pos2(c.x, r.max.y + h * 0.2),
            ]
        } else {
            vec![
                pos2(c.x - hw, r.max.y),
                pos2(c.x, r.min.y - h * 0.2),
                pos2(c.x + hw, r.max.y),
            ]
        };
        p.add(Shape::convex_polygon(pts.clone(), ink, Stroke::NONE));
        p.add(Shape::closed_line(
            pts,
            Stroke::new(1.0, alpha(Color32::BLACK, 0.6 * a)),
        ));
    }
    if has(tech, Techs::FRET_HAND_MUTE) {
        let st = Stroke::new((s * 0.5).max(1.5), white);
        p.line_segment([c + vec2(-s, -s), c + vec2(s, s)], st);
        p.line_segment([c + vec2(-s, s), c + vec2(s, -s)], st);
    }
    if has(tech, Techs::PALM_MUTE) {
        let f = alpha(Color32::WHITE, 0.45 * a);
        p.add(Shape::convex_polygon(
            vec![c + vec2(-s * 1.3, -s), c, c + vec2(-s * 1.3, s)],
            f,
            Stroke::NONE,
        ));
        p.add(Shape::convex_polygon(
            vec![c + vec2(s * 1.3, -s), c + vec2(s * 1.3, s), c],
            f,
            Stroke::NONE,
        ));
    }
    if has(tech, Techs::PINCH_HARMONIC) {
        for (rad, k) in [(0.42, 1.0), (0.28, 0.8)] {
            p.circle_stroke(
                c,
                w * rad,
                Stroke::new((h * 0.12).max(1.0), alpha(lut(col, 0.8), k * a)),
            );
        }
    } else if has(tech, Techs::HARMONIC) {
        p.circle_stroke(c, w * 0.275, Stroke::new((h * 0.1).max(1.0), white));
        p.circle_filled(c, w * 0.1, white);
    }
    if has(tech, Techs::TAP) {
        // a big V on the gem: 1.5 of its width, arms 0.35 of its height thick, the tip 0.75 below the top
        let hw = w * 0.75;
        let tip = pos2(c.x, r.min.y + h * 0.75);
        let arm = h * 0.35;
        let ink = alpha(lut(col, 0.72), a);
        for side in [-1.0f32, 1.0] {
            let top = pos2(c.x + side * hw, r.min.y - h * 0.35);
            let q = vec![top, tip, tip + vec2(0.0, arm), top + vec2(0.0, arm)];
            p.add(Shape::convex_polygon(q.clone(), ink, Stroke::NONE));
            p.add(Shape::closed_line(
                q,
                Stroke::new(1.0, alpha(Color32::BLACK, 0.55 * a)),
            ));
        }
    }
    if has(tech, Techs::SLAP) {
        p.rect_filled(
            Rect::from_center_size(pos2(c.x, r.max.y + s * 0.6), vec2(r.width() * 0.5, s * 0.5)),
            1.0,
            white,
        );
    }
    if has(tech, Techs::POP) {
        let y = r.min.y - s * 0.5;
        let st = Stroke::new(2.0, white);
        p.line_segment([pos2(c.x - s, y), pos2(c.x, y - s * 0.8)], st);
        p.line_segment([pos2(c.x, y - s * 0.8), pos2(c.x + s, y)], st);
    }
    if has(tech, Techs::ACCENT) {
        // dấu nhấn: một hình > nhỏ, trắng, phía trên bên trái viên nốt
        let (x0, y) = (r.min.x - w * 0.05, r.min.y - h * 0.35);
        let st = Stroke::new((h * 0.12).max(1.2), white);
        p.line_segment([pos2(x0 - w * 0.25, y - h * 0.18), pos2(x0, y)], st);
        p.line_segment([pos2(x0, y), pos2(x0 - w * 0.25, y + h * 0.18)], st);
    }
}

/// Tên kỹ thuật của một nốt (hoặc của cả hợp âm khi `chord` cho các nốt của nó), ngắn gọn bằng chữ in hoa:
/// "BEND FULL", "HAMMER-ON", "SLIDE UP", "PALM MUTE"… Rỗng khi nốt không có kỹ thuật nào.
fn tech_label(notes: &[&HwNote]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut add = |s: String| {
        if !parts.contains(&s) {
            parts.push(s);
        }
    };
    for n in notes {
        if n.bend > 0.0 {
            // bend tính bằng nửa cung: 1 = ½, 2 = full, 3 = 1½…
            let half = n.bend.round().max(1.0) as i64;
            let amt = match half {
                1 => "½".to_string(),
                2 => "FULL".to_string(),
                h if h % 2 == 0 => format!("{}", h / 2),
                h => format!("{}½", h / 2),
            };
            add(format!("BEND {amt}"));
        }
        if n.slide_to > 0 {
            add(if n.slide_to > n.fret {
                "SLIDE UP"
            } else {
                "SLIDE DOWN"
            }
            .into());
        }
        for (t, name) in [
            (Techs::HAMMER_ON, "HAMMER-ON"),
            (Techs::PULL_OFF, "PULL-OFF"),
            (Techs::VIBRATO, "VIBRATO"),
            (Techs::PALM_MUTE, "PALM MUTE"),
            (Techs::FRET_HAND_MUTE, "MUTE"),
            (Techs::PINCH_HARMONIC, "PINCH HARMONIC"),
            (Techs::TAP, "TAP"),
            (Techs::TREMOLO, "TREMOLO"),
            (Techs::SLAP, "SLAP"),
            (Techs::POP, "POP"),
            (Techs::ACCENT, "ACCENT"),
        ] {
            if has(n.tech, t) {
                add(name.into());
            }
        }
        if has(n.tech, Techs::HARMONIC) && !has(n.tech, Techs::PINCH_HARMONIC) {
            add("HARMONIC".into());
        }
    }
    parts.join(" · ")
}

/// Vẽ tên kỹ thuật trên một tấm nền tối nhỏ, căn giữa phía trên `at` (toạ độ màn hình).
fn tech_tag(p: &Painter, at: Pos2, label: &str, size: f32, col: Color32, a: f32) {
    if label.is_empty() || a <= 0.02 {
        return;
    }
    let st = Style::new(size, 800, alpha(Color32::WHITE, a));
    let tw = text::width(p.ctx(), label, st);
    let (padx, pady) = (size * 0.45, size * 0.22);
    let bg = Rect::from_center_size(
        pos2(at.x, at.y - size * 0.6 - pady),
        vec2(tw + 2.0 * padx, size * 1.2 + 2.0 * pady),
    );
    p.rect_filled(bg, size * 0.35, alpha(Color32::from_rgb(12, 16, 24), 0.78 * a));
    p.rect_stroke(
        bg,
        size * 0.35,
        Stroke::new(1.0, alpha(lut(col, 0.6), 0.9 * a)),
        egui::StrokeKind::Inside,
    );
    text::draw(p, label, bg.center(), Align2::CENTER_CENTER, st, None);
}

/// Bend chevrons above a gem (screen pixels), between it and its tail, one per half step, pointing the way the string
/// goes: white with an outline in the string's colour.
fn chevrons(p: &Painter, r: Rect, n: usize, up: bool, col: Color32, a: f32) {
    let (hw, ch) = (r.width() * 0.4, r.height() * 0.3);
    if ch < 1.2 {
        return;
    }
    let c = r.center().x;
    for i in 0..n.min(4) {
        // above the gem, or below it when the bend goes down (the top string)
        let y = if up {
            r.min.y - r.height() * (0.15 + 0.32 * i as f32)
        } else {
            r.max.y + r.height() * (0.15 + 0.32 * i as f32) + ch
        };
        let (tip, back) = if up { (y - ch, y) } else { (y, y - ch) };
        let pts = [pos2(c - hw, back), pos2(c, tip), pos2(c + hw, back)];
        let wd = (ch * 0.5).max(1.2);
        p.add(Shape::line(
            pts.to_vec(),
            Stroke::new(wd + 1.5, alpha(Color32::BLACK, 0.4 * a)),
        ));
        p.add(Shape::line(
            pts.to_vec(),
            Stroke::new(wd, alpha(lut(col, 0.7), a)),
        ));
    }
}

/// A sustain's tail: a ribbon (a quarter of the note's width, its colour toward white, light edges) flying in along
/// the note's path (from the vanishing point to its container): a smooth S toward the slide's fret, rising toward the
/// next string for a bend, waving sideways for vibrato, a tight sawtooth for tremolo. An open string's sustain: two
/// bright rails along the hand's window with a see-through fill between.
fn tail(p: &Painter, g: &Geo, n: &HwNote, col: Color32, a: f32, window: (i32, i32)) {
    let u0 = (n.t - g.now).max(0.0);
    let u1 = (n.t + n.sus - g.now).min(LOOK);
    if u1 <= u0 {
        return;
    }
    let steps = ((u1 - u0) * 120.0).ceil().clamp(16.0, 90.0) as usize;
    let sample = |i: usize| u0 + (u1 - u0) * i as f64 / steps as f64;
    if n.fret <= 0 {
        let bar = g.open_bar(n.string, window.0, window.0 + window.1 - 1);
        let (mut l, mut r, mut f) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..=steps {
            let gg = g.g(sample(i));
            let (a0, b0) = (g.toward(bar.left_center(), gg), g.toward(bar.right_center(), gg));
            l.push(g.to(a0));
            r.push(g.to(b0));
            f.push(g.fog(a0.y) * a);
        }
        for i in 0..steps {
            quad(
                p,
                [l[i], r[i], r[i + 1], l[i + 1]],
                alpha(shade(col, 0.4), 0.45 * f[i + 1]),
            );
            let w = (g.sz(7.0) * g.g(sample(i))).max(1.5);
            glow_line(p, l[i], l[i + 1], w, col, f[i + 1]);
            glow_line(p, r[i], r[i + 1], w, col, f[i + 1]);
            let (m0, m1) = (
                pos2((l[i].x + r[i].x) / 2.0, l[i].y),
                pos2((l[i + 1].x + r[i + 1].x) / 2.0, l[i + 1].y),
            );
            p.line_segment([m0, m1], Stroke::new(1.0, alpha(col, 0.35 * f[i + 1])));
        }
        return;
    }
    let from = g.container(n.string, n.fret);
    let to = if n.slide_to > 0 {
        g.container(n.string, n.slide_to)
    } else {
        from
    };
    let bend_to = if g.row(n.string) > 0.0 { -1.0 } else { 1.0 };
    let rise = bend_to * g.gap() * (0.75 * n.bend as f32).min(2.2);
    let w = from.width();
    let ribbon = if has(n.tech, Techs::TREMOLO) {
        lut(col, 0.5)
    } else {
        shade(col, 0.7)
    };
    let edge = lut(col, 0.62);
    let mut left = Vec::with_capacity(steps + 1);
    let mut right = Vec::with_capacity(steps + 1);
    let mut fades = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let u = sample(i);
        let tt = g.now + u - n.t;
        let v = (tt / n.sus.max(1e-3)).clamp(0.0, 1.0);
        let along = from.center() + (to.center() - from.center()) * (v * v) as f32;
        let mut home = along;
        if n.bend > 0.0 {
            home.y += rise * smooth(v * 2.0) as f32;
        }
        if has(n.tech, Techs::VIBRATO) {
            home.x += 0.07 * w * (std::f64::consts::TAU * tt / 0.15).sin() as f32;
        }
        if has(n.tech, Techs::TREMOLO) {
            home.x += (((tt / 0.035).fract() - 0.5) * 2.0) as f32 * 0.2 * w;
        }
        let wd = w * 0.24;
        let gg = g.g(u);
        // the tail rides the note's lane like the note does (sliding across lanes toward the slide's fret)
        let fm = (n.fret as f64 - 0.5)
            + if n.slide_to > 0 {
                (n.slide_to - n.fret) as f64 * v * v
            } else {
                0.0
            };
        let c = g.ride(along, fm, g.row(n.string), u) + (home - along) * gg;
        left.push(g.to(c - vec2(wd * gg, 0.0)));
        right.push(g.to(c + vec2(wd * gg, 0.0)));
        fades.push(g.fog(c.y) * a);
    }
    let mut m = egui::Mesh::default();
    for i in 0..=steps {
        m.colored_vertex(left[i], alpha(ribbon, fades[i]));
        m.colored_vertex(right[i], alpha(ribbon, fades[i]));
    }
    for i in 0..steps as u32 {
        m.add_triangle(2 * i, 2 * i + 1, 2 * i + 3);
        m.add_triangle(2 * i, 2 * i + 3, 2 * i + 2);
    }
    p.add(Shape::mesh(m));
    for i in 0..steps {
        let f = fades[i + 1];
        p.line_segment(
            [left[i], left[i + 1]],
            Stroke::new(g.sz(3.0).max(1.0), alpha(edge, f)),
        );
        p.line_segment(
            [right[i], right[i + 1]],
            Stroke::new(g.sz(3.0).max(1.0), alpha(edge, f)),
        );
    }
}

/// A chord's frame at its depth: a dark glass box across the hand's window, from just below the nearest string to
/// just above the chord's highest note, thin cyan edges all round with a soft glow, a top and right side going back up
/// the highway; a repeated chord is a lower filled step with no name. A new chord's box waits at the strings through
/// its hand shape. An arpeggio's shape: lilac corner brackets. The name big and light at the box's upper left
/// (sharps, as Rocksmith spells them), shrinking with distance.
fn frame(p: &Painter, g: &Geo, ch: &HwChord, lo: i32, hi: i32, arpeggio: bool) {
    let mut u = ch.t - g.now;
    let mut a = 1.0f32;
    if u < 0.0 && !ch.repeat && g.now <= ch.hold_end + 0.1 {
        a = ((ch.hold_end + 0.1 - g.now) / 0.1).clamp(0.0, 1.0) as f32;
        u = 0.0;
    }
    if !(-0.05..=LOOK).contains(&u) {
        return;
    }
    let gg = g.g(u);
    let gb = g.g(u + 0.08);
    let (fl, fr) = ((lo - 1) as f64, hi as f64);
    let high = ch.strings.iter().map(|&s| g.row(s)).fold(5.0f32, f32::min);
    let top_row = if ch.repeat { 4.3 } else { (high - 0.6).min(4.0) };
    let corner = |f: f64, r: f32, k: f32| g.to(g.toward(g.on_row(f, r), k));
    let (bl, br, tl, tr) = (
        corner(fl, 5.6, gg),
        corner(fr, 5.6, gg),
        corner(fl, top_row, gg),
        corner(fr, top_row, gg),
    );
    let (tl2, tr2, br2) = (
        corner(fl, top_row, gb),
        corner(fr, top_row, gb),
        corner(fr, 5.6, gb),
    );
    a *= g.fog(g.toward(g.on_row(fl, top_row), gg).y);
    if a <= 0.0 {
        return;
    }
    if arpeggio {
        let w = (g.sz(2.0) * gg).max(1.0);
        let len = (tr.x - tl.x) * 0.12;
        for (c, dx, dy) in [(tl, 1.0, 1.0), (tr, -1.0, 1.0), (bl, 1.0, -1.0), (br, -1.0, -1.0)] {
            soft_line(p, c, c + vec2(dx * len, 0.0), w, w, alpha(LILAC, a));
            soft_line(p, c, c + vec2(0.0, dy * len), w, w, alpha(LILAC, a));
        }
        return;
    }
    let glass = rgb(5, 25, 30);
    // the glass fades as the box reaches the strings: there it's an open outline the strings show through
    let gf = smooth(u / 0.3) as f32;
    // the back: its top and right side going up the highway, then the front face
    quad(p, [tl, tr, tr2, tl2], alpha(glass, 0.35 * a * gf));
    quad(p, [tr, br, br2, tr2], alpha(glass, 0.35 * a * gf));
    let edge_w = (g.sz(1.8) * gg).max(0.8);
    for (p0, p1) in [(tl, tl2), (tr, tr2), (tl2, tr2), (br, br2), (tr2, br2)] {
        p.line_segment([p0, p1], Stroke::new(edge_w * 0.6, alpha(CYAN, 0.35 * a)));
    }
    quad(p, [bl, br, tr, tl], alpha(glass, 0.7 * a * gf));
    for (p0, p1) in [(bl, tl), (br, tr), (tl, tr), (bl, br)] {
        soft_line(p, p0, p1, edge_w, edge_w, alpha(CYAN, a));
    }
    if !ch.repeat {
        for &s in &ch.opens {
            let bar = g.open_bar(s, lo, hi);
            let (l, r) = (
                g.to(g.toward(bar.left_center(), gg)),
                g.to(g.toward(bar.right_center(), gg)),
            );
            let col = STRING_ON[s.clamp(0, 5) as usize];
            soft_line(
                p,
                l,
                r,
                (g.sz(6.0) * gg).max(2.0),
                g.sz(2.0),
                alpha(lut(col, 0.55), a),
            );
        }
    }
    if !ch.name.is_empty() && !ch.repeat {
        let name = sharp_name(&ch.name);
        let size = (g.sz(44.0) * gg).clamp(9.0, g.sz(44.0));
        let at = tl - vec2(0.5 * size, 0.6 * size);
        text::draw(
            p,
            &name,
            at + vec2(1.5, 1.5),
            Align2::RIGHT_BOTTOM,
            Style::new(size, 600, alpha(Color32::BLACK, 0.6 * a)),
            None,
        );
        text::draw(
            p,
            &name,
            at,
            Align2::RIGHT_BOTTOM,
            Style::new(size, 600, alpha(rgb(232, 232, 236), a)),
            None,
        );
    }
}

/// A chord name as Rocksmith spells it: flats as sharps (Gb5 -> F♯5), ♯ for #.
fn sharp_name(name: &str) -> String {
    let mut out = String::new();
    let cs: Vec<char> = name.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_ascii_uppercase() && cs.get(i + 1) == Some(&'b') && "ABCDEFG".contains(c) {
            let (l, sharp) = match c {
                'A' => ('G', true),
                'B' => ('A', true),
                'C' => ('B', false),
                'D' => ('C', true),
                'E' => ('D', true),
                'F' => ('E', false),
                _ => ('F', true),
            };
            out.push(l);
            if sharp {
                out.push('♯');
            }
            i += 2;
            continue;
        }
        out.push(if c == '#' { '♯' } else { c });
        i += 1;
    }
    out
}

/// The hand's window at time t (first fret, width).
fn window_at(c: &Cache, t: f64) -> Option<(i32, i32)> {
    c.anchors[..c.anchors.partition_point(|a| a.0 <= t + 1e-3)]
        .last()
        .map(|a| (a.2, a.3))
}

/// A chord's frame spans its frets and the hand's window at that time.
fn chord_span(c: &Cache, ch: &HwChord) -> (i32, i32) {
    let (mut lo, mut hi) = window_at(c, ch.t).map_or((ch.lo, ch.hi), |(f, w)| (f, f + w - 1));
    if ch.hi > 0 {
        lo = lo.min(ch.lo);
        hi = hi.max(ch.hi);
    }
    (lo.max(1), hi.max(lo.max(1)))
}

/// Draw a frame of the highway.
#[allow(clippy::too_many_arguments)]
fn draw(
    p: &Painter,
    w: Rect,
    c: &Cache,
    cam: (f32, f32),
    now: f64,
    n_str: usize,
    invert: bool,
    speed: f64,
    marks: &[Option<(Judgement, f32)>],
    labels: bool,
) {
    // the camera: the planned view's middle under the vanishing point, zoomed in as far as Rocksmith goes (11 frets
    // across) unless the view's right half needs more room
    let pf = 1280.0 / cam.1.max(4.0);
    let g = Geo {
        w,
        fc: cam.0 as f64,
        pf,
        n_str,
        invert,
        t: T_NOTE * speed / SPEEDS[SPEED_DEFAULT] as f64
            * (qrs_formats::model::VISUAL_DENSITY_DEFAULT / c.density.max(0.3)) as f64,
        now,
    };
    p.rect_filled(w, 14.0, rgb(3, 4, 8));
    {
        let y = |v: f32| w.min.y + v * w.height() / 720.0;
        let (top, haze, low) = (rgb(4, 5, 9), rgb(16, 22, 34), rgb(4, 5, 8));
        let hz = g.vp().y + 40.0;
        for (y0, y1, c0, c1) in [(0.0, hz, top, haze), (hz, 520.0, haze, low)] {
            quad_grad(
                p,
                [
                    pos2(w.min.x, y(y0)),
                    pos2(w.max.x, y(y0)),
                    pos2(w.max.x, y(y1)),
                    pos2(w.min.x, y(y1)),
                ],
                [c0, c0, c1, c1],
            );
        }
    }
    let f_lo = ((g.fc - (640.0 / pf) as f64).floor() as i32 - 1).max(0);
    let f_hi = ((g.fc + (640.0 / pf) as f64).ceil() as i32 + 1).min(24);
    let win = window_at(c, now);
    let in_win = |f: i32| win.is_some_and(|(a, w)| f >= a && f < a + w);

    // the highway: the hand's lanes lit as they come, each lane a soft hump (dark at its lines, navy in its middle,
    // fading toward a deeper blue far off); lane lines thin with a faint halo, the window's cyan, the rest dim; measure
    // lines grey-teal, orange where a section starts
    let depth = |y: f32| ((g.row_y(0.0, 640.0) - y) / (g.row_y(0.0, 640.0) - FOG_Y)).clamp(0.0, 1.0);
    let ai = c.anchors.partition_point(|a| a.1 < now - 0.2);
    let visible: Vec<_> = c.anchors[ai..]
        .iter()
        .take_while(|a| a.0 <= now + LOOK)
        .copied()
        .collect();
    // the board: barely there, a touch lighter through the middle strings (dark, light, dark)
    {
        let row = |r: f32| {
            (
                g.to(pos2(0.0, g.row_y(r, 0.0))),
                g.to(pos2(1280.0, g.row_y(r, 1280.0))),
            )
        };
        let dark = alpha(rgb(6, 7, 10), 0.5);
        let mid = alpha(rgb(12, 12, 15), 0.5);
        for (r0, r1, c0, c1) in [(-0.5, 2.5, dark, mid), (2.5, 5.6, mid, dark)] {
            let ((l0, rr0), (l1, rr1)) = (row(r0), row(r1));
            quad_grad(p, [l0, rr0, rr1, l1], [c0, c0, c1, c1]);
        }
    }
    // under the neck: the window's lanes run on faintly behind the strings
    if let Some((wf, ww)) = win {
        let y_of = |f: f64, r: f32| g.on_row(f, r);
        let (fa, fb) = ((wf - 1) as f64, (wf - 1 + ww) as f64);
        let (a0, b0, a1, b1) = (y_of(fa, 0.0), y_of(fb, 0.0), y_of(fa, 5.4), y_of(fb, 5.4));
        quad_grad(
            p,
            [g.to(a0), g.to(b0), g.to(b1), g.to(a1)],
            [
                alpha(FLOOR_LIT, 0.12),
                alpha(FLOOR_LIT, 0.12),
                Color32::TRANSPARENT,
                Color32::TRANSPARENT,
            ],
        );
        for f in (wf - 1)..=(wf - 1 + ww) {
            let (t, b) = (g.to(y_of(f as f64, 0.0)), g.to(y_of(f as f64, 5.4)));
            quad_grad(
                p,
                [
                    t - vec2(0.7, 0.0),
                    t + vec2(0.7, 0.0),
                    b + vec2(0.7, 0.0),
                    b - vec2(0.7, 0.0),
                ],
                [
                    alpha(LANE_LIT, 0.45),
                    alpha(LANE_LIT, 0.45),
                    Color32::TRANSPARENT,
                    Color32::TRANSPARENT,
                ],
            );
        }
    }
    let lane_edge = rgb(0, 4, 16);
    let lane_mid = rgb(8, 44, 80);
    let lane_far = rgb(6, 30, 60);
    for a in &visible {
        let (u0, u1) = ((a.0 - now).max(0.0), (a.1 - now).min(LOOK));
        if u1 <= u0 {
            continue;
        }
        let n = 10;
        for k in 0..n {
            let (ua, ub) = (
                u0 + (u1 - u0) * k as f64 / n as f64,
                u0 + (u1 - u0) * (k + 1) as f64 / n as f64,
            );
            for f in a.2..a.2 + a.3 {
                let (l0, l1) = g.floor_line((f - 1) as f64, ua, ub);
                let (m0, m1) = g.floor_line(f as f64 - 0.5, ua, ub);
                let (r0, r1) = g.floor_line(f as f64, ua, ub);
                if (l0.y - l1.y).abs() < 0.2 {
                    continue;
                }
                let (d0, d1) = (depth(l0.y), depth(l1.y));
                let (f0, f1) = (g.fog(l0.y), g.fog(l1.y));
                let mid0 = alpha(mix(lane_mid, lane_far, d0), f0);
                let mid1 = alpha(mix(lane_mid, lane_far, d1), f1);
                let (e0, e1) = (alpha(lane_edge, f0), alpha(lane_edge, f1));
                quad_grad(p, [g.to(l0), g.to(m0), g.to(m1), g.to(l1)], [e0, mid0, mid1, e1]);
                quad_grad(p, [g.to(m0), g.to(r0), g.to(r1), g.to(m1)], [mid0, e0, e1, mid1]);
            }
        }
    }
    let lane = |f: f64, u0: f64, u1: f64, near: Color32, far: Color32, w: f32| {
        let n = 6;
        for k in 0..n {
            let (ua, ub) = (
                u0 + (u1 - u0) * k as f64 / n as f64,
                u0 + (u1 - u0) * (k + 1) as f64 / n as f64,
            );
            let (s0, s1) = g.floor_line(f, ua, ub);
            if (s0.y - s1.y).abs() < 0.2 {
                continue;
            }
            let d = depth((s0.y + s1.y) / 2.0);
            let col = mix(near, far, d);
            let fa = g.fog(s1.y);
            let w = (w * (1.0 - 0.6 * d)).max(0.8);
            soft_line(p, g.to(s0), g.to(s1), w, w, alpha(col, fa));
        }
    };
    for f in f_lo - 1..=f_hi {
        for k in 0..6 {
            let (ua, ub) = (LOOK * k as f64 / 6.0, LOOK * (k + 1) as f64 / 6.0);
            let (a, b) = g.floor_line(f as f64, ua, ub);
            if (a.y - b.y).abs() > 0.2 {
                p.line_segment(
                    [g.to(a), g.to(b)],
                    Stroke::new(g.sz(1.0), alpha(LANE_DIM, g.fog(b.y))),
                );
            }
        }
    }
    for a in &visible {
        let (u0, u1) = ((a.0 - now).max(0.0), (a.1 - now).min(LOOK));
        if u1 <= u0 {
            continue;
        }
        for f in (a.2 - 1)..=(a.2 - 1 + a.3) {
            lane(f as f64, u0, u1, LANE_LIT, LANE_FAR, g.sz(1.3));
        }
    }
    let bi = c.beats.partition_point(|b| b.0 < now);
    for b in c.beats[bi..].iter().take_while(|b| b.0 <= now + LOOK) {
        let u = b.0 - now;
        let Some((wf, ww)) = window_at(c, b.0) else {
            continue;
        };
        let (l, r) = (g.floor((wf - 1) as f64, u), g.floor((wf - 1 + ww) as f64, u));
        // (only on the highway: not across the neck)
        if l.y > g.row_y(0.0, l.x) - 1.0 {
            continue;
        }
        let fade = g.fog(l.y);
        let section = c.sections.iter().any(|&t| (t - b.0).abs() < 0.03);
        if section {
            soft_line(
                p,
                g.to(l),
                g.to(r),
                (g.sz(2.5) * g.g(u)).clamp(1.0, 2.5),
                g.sz(2.0),
                alpha(BAR, 0.7 * fade),
            );
        } else if b.1 {
            soft_line(
                p,
                g.to(l),
                g.to(r),
                (g.sz(2.2) * g.g(u)).clamp(0.8, 2.2),
                g.sz(1.0),
                alpha(MEASURE, 0.8 * fade),
            );
        } else {
            p.line_segment([g.to(l), g.to(r)], Stroke::new(1.0, alpha(MEASURE, 0.3 * fade)));
        }
    }
    // where the hand moves: the inlay frets in a row on the floor as the new window comes (gold in it, teal outside)
    let mut shown_move = false;
    for (k, a) in visible.iter().enumerate() {
        let prev = if k == 0 {
            window_at(c, a.0 - 0.01)
        } else {
            Some((visible[k - 1].2, visible[k - 1].3))
        };
        let Some((pf0, pw0)) = prev else { continue };
        let moved = ((a.2 as f32 + a.3 as f32 / 2.0) - (pf0 as f32 + pw0 as f32 / 2.0)).abs();
        if moved < 2.0 || a.0 < now || a.0 > now + 2.5 || shown_move {
            continue;
        }
        let u = a.0 - now;
        shown_move = true;
        let gg = g.g(u);
        for f in [3, 5, 7, 9, 12, 15, 17, 19, 21, 24] {
            if f < f_lo || f > f_hi {
                continue;
            }
            let at = g.floor(f as f64 - 0.5, u);
            if at.y > g.row_y(0.0, at.x) {
                continue;
            }
            let col = if f >= a.2 && f < a.2 + a.3 {
                NUM_IN
            } else {
                NUM_FLOOR_OUT
            };
            let size = (g.sz(30.0) * gg).clamp(7.0, g.sz(16.0));
            text::draw(
                p,
                &f.to_string(),
                g.to(at),
                Align2::CENTER_BOTTOM,
                Style::new(size, 800, alpha(col, g.fog(at.y))),
                None,
            );
        }
    }

    // how lit each string is: a note coming at it lights it more as it nears (you can tell which string it will land
    // on), fully while it sounds
    let i0 = c.notes.partition_point(|n| n.t < now - 4.0);
    let i1 = c.notes.partition_point(|n| n.t <= now + LOOK);
    let mut live = [0.0f32; 6];
    for n in &c.notes[i0..i1] {
        let u = n.t - now;
        let k = if u <= 0.05 && now <= n.t + n.sus.max(0.1) {
            1.0
        } else if (0.0..1.2).contains(&u) {
            (1.0 - u / 1.2) as f32
        } else {
            0.0
        };
        let l = &mut live[n.string.clamp(0, 5) as usize];
        *l = l.max(k);
    }
    // which strings are vibrating (a sustain sounding on them)
    let mut vib = [false; 6];
    for n in &c.notes[i0..i1] {
        if n.sus > 0.05 && now >= n.t && now <= n.t + n.sus {
            vib[n.string.clamp(0, 5) as usize] = true;
        }
    }

    // the neck: soft inlay discs midway between the middle strings (gold in the window, teal outside), the strings, the
    // posts (dark metal, a warm highlight in the window), the fret numbers under them (gold with a glow in the window)
    for f in f_lo.max(1)..=f_hi {
        if matches!(f, 3 | 5 | 7 | 9 | 12 | 15 | 17 | 19 | 21 | 24) {
            let col = if in_win(f) { INLAY_IN } else { INLAY_OUT };
            let rows: &[f32] = if f % 12 == 0 { &[1.5, 3.5] } else { &[2.5] };
            for &r in rows {
                let at = g.to(g.on_row(f as f64 - 0.5, r));
                let rad = g.sz(0.115 * pf);
                p.circle_filled(at, rad + 1.0, alpha(col, 0.25));
                p.circle_filled(at, rad, alpha(col, 0.8));
            }
        }
    }
    let strings_k: [f32; 6] = if n_str <= 4 {
        [1.8, 1.7, 1.5, 1.4, 1.4, 1.4]
    } else {
        [1.35, 1.15, 1.0, 0.85, 0.75, 0.68]
    };
    for s in 0..n_str as i32 {
        let r = g.row(s);
        let k = live[s as usize];
        let on = STRING_ON[s as usize % 6];
        let th = g.sz((0.03 * pf).clamp(1.2, 3.0) * strings_k[s as usize]);
        let segs = 40;
        let at = |x: f32| g.to(pos2(x, g.row_y(r, x)));
        let fade = |x: f32| {
            let e = (x.min(1280.0 - x) / 120.0).clamp(0.0, 1.0);
            e * e * (3.0 - 2.0 * e)
        };
        let core = mix(shade(on, 0.6), on, k);
        for q in 0..segs {
            let (x0, x1) = (
                1280.0 * q as f32 / segs as f32,
                1280.0 * (q + 1) as f32 / segs as f32,
            );
            let (a0, a1) = (at(x0), at(x1));
            let f = fade((x0 + x1) / 2.0);
            if f <= 0.0 {
                continue;
            }
            // the glow of a lit string: one soft falloff in its hue
            if k > 0.0 {
                soft_line(p, a0, a1, 0.0, g.sz(0.18 * pf), alpha(on, 0.3 * k * f));
            }
            soft_line(p, a0, a1, th * 0.9, th * 0.5, alpha(core, f));
            if k > 0.3 {
                p.line_segment(
                    [a0, a1],
                    Stroke::new((th * 0.3).max(0.6), alpha(lut(on, 0.65), 0.3 * k * f)),
                );
            }
            if vib[s as usize] {
                // a sounding string shimmers along its length
                let ph = (now * 40.0) as f32 + q as f32 * 1.7;
                let off = vec2(0.0, ph.sin() * th * 0.8);
                soft_line(p, a0 + off, a1 - off, th * 0.6, th, alpha(lut(on, 0.8), 0.35 * f));
            }
        }
        // the wound strings (a guitar's lower three, every string of a bass): a faint fine winding, close up only
        let wound = if n_str <= 4 { true } else { s < 3 };
        if wound && pf > 70.0 && !vib[s as usize] {
            let (l, rt) = (at(0.0), at(1280.0));
            let len = (rt - l).length();
            let dir = (rt - l) / len.max(1.0);
            let step = g.sz(1.8).max(1.5);
            let across = vec2(-dir.y, dir.x) * th * 0.45;
            let mut d = 0.0;
            while d < len {
                let x = d / len * 1280.0;
                let c0 = l + dir * d;
                p.line_segment(
                    [c0 - across, c0 + across],
                    Stroke::new((step * 0.4).max(0.6), alpha(Color32::BLACK, 0.08 * fade(x))),
                );
                d += step;
            }
        }
    }
    let post = |f: i32| (g.to(g.on_row(f as f64, 5.3)), g.to(g.on_row(f as f64, -0.45)));
    for f in f_lo..=f_hi {
        let (b, t) = post(f);
        let inside = win.is_some_and(|(a, w)| f >= a - 1 && f <= a - 1 + w);
        let body = if inside { rgb(60, 44, 26) } else { rgb(34, 26, 18) };
        let w = g.sz(0.05 * pf).max(1.2);
        // slightly bowed: a polyline bending left toward the middle
        let left = (t - b).normalized().rot90() * -1.0;
        let pts: Vec<Pos2> = (0..=6)
            .map(|i| {
                let s = i as f32 / 6.0;
                b + (t - b) * s + left * (std::f32::consts::PI * s).sin() * g.sz(0.08 * pf) * 0.25
            })
            .collect();
        p.add(Shape::line(pts.clone(), Stroke::new(w, body)));
        if inside {
            let hl: Vec<Pos2> = pts.iter().map(|q| *q + left * w * 0.3).collect();
            p.add(Shape::line(
                hl,
                Stroke::new(g.sz(1.0).max(0.8), rgb(190, 140, 70)),
            ));
        }
        p.circle_filled(t, w * 0.5, body);
    }
    for f in f_lo.max(1)..=f_hi {
        let at = g.to(g.on_row(f as f64 - 0.5, 5.0) + vec2(0.0, 0.3 * pf));
        let inw = in_win(f);
        let col = if inw { NUM_IN } else { NUM_OUT };
        let size = g.sz(0.32 * pf);
        if inw {
            for off in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
                text::draw(
                    p,
                    &f.to_string(),
                    at + off,
                    Align2::CENTER_CENTER,
                    Style::new(size, 800, alpha(rgb(255, 120, 0), 0.2)),
                    None,
                );
            }
        }
        text::draw(
            p,
            &f.to_string(),
            at,
            Align2::CENTER_CENTER,
            Style::new(size, 800, col),
            None,
        );
    }
    // a chord's fingers: a white numeral in a thin ring on each fretted string at its fret, from just before the chord
    // through its hand shape
    for ch in c
        .chords
        .iter()
        .skip(c.chords.partition_point(|ch| ch.t < now - 8.0))
    {
        if ch.t > now + 0.5 {
            break;
        }
        if ch.repeat || now > ch.hold_end.max(ch.t) + 0.05 {
            continue;
        }
        for (s, (&fret, &finger)) in ch.frets.iter().zip(&ch.fingers).enumerate() {
            if fret <= 0 || finger <= 0 || s >= n_str {
                continue;
            }
            let at = g.to(g.on_row(fret as f64 - 0.5, g.row(s as i32)));
            let k = pf / 100.0;
            let rad = vec2(g.sz(12.0 * k), g.sz(14.0 * k));
            p.add(Shape::ellipse_filled(at, rad, alpha(rgb(8, 10, 14), 0.5)));
            p.add(Shape::ellipse_stroke(
                at,
                rad,
                Stroke::new(g.sz(1.8).max(1.0), alpha(Color32::WHITE, 0.85)),
            ));
            text::draw(
                p,
                &finger.to_string(),
                at,
                Align2::CENTER_CENTER,
                Style::new(g.sz(20.0 * k), 700, Color32::WHITE),
                None,
            );
        }
    }

    // containers: a hollow box on the string, growing to the note's size over the last 0.22 s (gone once the note is
    // nearly there, so there's no double image)
    for n in &c.notes[c.notes.partition_point(|n| n.t < now - 0.3)..i1] {
        let u = n.t - now;
        if !(0.1..=CONTAINER).contains(&u) || n.fret <= 0 || n.chord.is_some_and(|k| c.chords[k].repeat) {
            continue;
        }
        if n.group.and_then(|k| marks.get(k).copied().flatten()).is_some() {
            continue;
        }
        let home = g.container(n.string, n.fret);
        let r = Rect::from_center_size(home.center(), home.size() * vec2(0.92, 0.8));
        let col = STRING_ON[n.string.clamp(0, 5) as usize];
        let rr = Rect::from_min_max(g.to(r.min), g.to(r.max));
        let fade_in = ((CONTAINER - u) / 0.15).clamp(0.0, 1.0) as f32;
        p.rect_stroke(
            rr,
            2.0,
            Stroke::new(g.sz(2.0), alpha(lut(col, 0.45), 0.85 * fade_in)),
            egui::StrokeKind::Inside,
        );
        if n.bend > 0.0 {
            chevrons(p, rr, 1, g.row(n.string) > 0.0, col, fade_in);
        }
        if n.slide_to > 0 {
            let right = n.slide_to > n.fret;
            let (base, tip) = if right {
                (rr.max.x + 3.0, rr.max.x + 11.0)
            } else {
                (rr.min.x - 3.0, rr.min.x - 11.0)
            };
            let st = Stroke::new(2.0, alpha(lut(col, 0.6), 0.9));
            p.line_segment([pos2(base, rr.min.y), pos2(tip, rr.center().y)], st);
            p.line_segment([pos2(tip, rr.center().y), pos2(base, rr.max.y)], st);
        }
    }

    // chords and notes, far to near: a chord's frame, then each note's tail, stem and floor number, and its gem
    let ci0 = c.chords.partition_point(|ch| ch.t < now - 8.0);
    let ci1 = c.chords.partition_point(|ch| ch.t <= now + LOOK);
    let mut ci = ci1;
    let mut hits: Vec<(Rect, Color32, Judgement, f32, i32, bool, Option<usize>)> = Vec::new();
    for n in c.notes[i0..i1].iter().rev() {
        while ci > ci0 && c.chords[ci - 1].t >= n.t - 1e-3 {
            ci -= 1;
            let ch = &c.chords[ci];
            let (lo, hi) = chord_span(c, ch);
            frame(p, &g, ch, lo, hi, false);
        }
        let u = n.t - now;
        let mark = n.group.and_then(|k| marks.get(k).copied().flatten());
        let s = n.string.clamp(0, 5) as usize;
        let col = STRING_ON[s];
        let repeat = n.chord.is_some_and(|k| c.chords[k].repeat);
        let open = n.fret <= 0;
        let home = if open {
            let (wf, ww) = window_at(c, n.t).unwrap_or((1, 4));
            g.open_bar(n.string, wf, wf + ww - 1)
        } else {
            g.container(n.string, n.fret)
        };
        let sounding = n.sus > 0.05 && now >= n.t && now <= n.t + n.sus;
        if let Some((j, age)) = mark
            && age < WORD.max(HIT_GONE)
        {
            hits.push((home, col, j, age, n.fret, sounding, n.chord));
        }
        if n.t + n.sus < now - 0.5 {
            continue;
        }
        if n.sus > 0.05 && !repeat {
            tail(p, &g, n, col, 1.0, window_at(c, n.t).unwrap_or((1, 4)));
        }
        if repeat {
            continue;
        }
        match mark {
            // played: the hit plays out at the string (below); a held sustain stays lit there
            Some((j, _)) if j != Judgement::Miss => {
                if sounding {
                    let v = ((now - n.t) / n.sus.max(1e-3)).clamp(0.0, 1.0);
                    let mut at = home;
                    if n.slide_to > 0 && !open {
                        let to = g.container(n.string, n.slide_to);
                        at = at.translate((to.center() - home.center()) * (v * v) as f32);
                    }
                    if n.bend > 0.0 {
                        let up = if g.row(n.string) > 0.0 { -1.0 } else { 1.0 };
                        at = at.translate(vec2(
                            0.0,
                            up * g.gap() * (0.75 * n.bend as f32).min(2.2) * smooth(v * 2.0) as f32,
                        ));
                    }
                    if open {
                        cigar(p, &g, at, col, 1.0);
                    } else {
                        block(p, &g, at, 0.0, 0.0, col, 1.0, 0.85);
                    }
                }
                continue;
            }
            // missed: it fades where it is
            Some((Judgement::Miss, _)) if u < 0.0 => {
                let fade = 1.0 - (-u) as f32 / 0.05;
                if fade > 0.0 {
                    block(p, &g, home, 0.0, 0.0, col, fade * 0.6, 0.0);
                }
                continue;
            }
            _ => {}
        }
        if u < -0.03 {
            continue;
        }
        // a harmonic is a diamond: a small square turned 45 degrees
        let harmonic = has(n.tech, Techs::HARMONIC) || has(n.tech, Techs::PINCH_HARMONIC);
        let home_drawn = home;
        let _ = harmonic;
        let lane_f = if open {
            window_at(c, n.t).map_or(2.5, |(wf, ww)| (wf - 1) as f64 + ww as f64 / 2.0)
        } else {
            n.fret as f64 - 0.5
        };
        let (r, roll, top) = g.flying(home_drawn, lane_f, g.row(n.string), u.max(0.0));
        let (roll, top) = if open { (0.0, 0.0) } else { (roll, top) };
        let a = g.fog(r.center().y);
        if a <= 0.0 {
            continue;
        }
        // a single note: a stem in its colour down to the floor and its fret on a tile there; a chord's notes only
        // their fret on the floor (once per fret)
        let drop_fade = smooth((u - 0.12) / 0.2) as f32;
        if !open && u > 0.12 {
            let mut floor = g.floor(n.fret as f64 - 0.5, u);
            floor.y = floor.y.min(g.row_y(0.0, floor.x) - 2.0);
            let a = a * drop_fade;
            if floor.y > r.max.y + 2.0 {
                if n.chord.is_none() {
                    let sw = (g.sz(1.5) * g.g(u)).max(1.0);
                    p.line_segment(
                        [
                            g.to(pos2(r.center().x, r.max.y)),
                            g.to(pos2(r.center().x, floor.y)),
                        ],
                        Stroke::new(sw, alpha(lut(col, 0.6), 0.8 * a)),
                    );
                }
                let fresh = n.chord.is_none()
                    || !c.notes[c.notes.partition_point(|m| m.t < n.t - 3e-3)..]
                        .iter()
                        .take_while(|m| m.t < n.t + 3e-3)
                        .any(|m| m.fret == n.fret && m.string < n.string);
                if fresh {
                    let (tl, tr) = (g.floor((n.fret - 1) as f64, u), g.floor(n.fret as f64, u));
                    let th = (g.sz(7.0) * g.g(u)).max(2.0);
                    let tile = Rect::from_x_y_ranges(
                        g.to(tl).x..=g.to(tr).x,
                        g.to(floor).y - th / 2.0..=g.to(floor).y + th / 2.0,
                    );
                    if n.chord.is_none() {
                        p.rect_filled(tile, 1.0, alpha(TILE, 0.3 * a));
                    }
                    let size = (g.sz(20.0) * g.g(u) * 1.4).clamp(7.0, g.sz(16.0));
                    text::draw(
                        p,
                        &n.fret.to_string(),
                        tile.center_top(),
                        Align2::CENTER_BOTTOM,
                        Style::new(size, 800, alpha(NUM_IN, a)),
                        None,
                    );
                }
            }
        }
        if open {
            cigar(p, &g, r, col, a);
        } else {
            block(p, &g, r, roll, top, col, a, 0.0);
        }
        let mut sr = Rect::from_min_max(g.to(r.min), g.to(r.max));
        if open {
            let gw = g.sz(0.5 * pf * g.g(u.max(0.0)));
            sr = Rect::from_center_size(sr.center(), vec2(gw, gw * 0.5));
        }
        face_icon(p, sr, n.tech, col, a);
        if n.bend > 0.0 {
            chevrons(
                p,
                sr,
                n.bend.round().max(1.0) as usize,
                g.row(n.string) > 0.0,
                col,
                a,
            );
        }
        // tên kỹ thuật cho người mới: chỉ khi bật, chỉ cho nốt trong ~2,2 giây tới, mỗi hợp âm một nhãn (ở nốt dây
        // cao nhất của nó, nằm trên cùng)
        if labels && u < 2.2 && sr.height() >= 6.0 {
            let group: Vec<&HwNote> = match n.chord {
                Some(k) => {
                    let lo = c.notes.partition_point(|m| m.t < n.t - 3e-3);
                    let same: Vec<&HwNote> = c.notes[lo..]
                        .iter()
                        .take_while(|m| m.t < n.t + 3e-3)
                        .filter(|m| m.chord == Some(k))
                        .collect();
                    // nhãn chỉ vẽ một lần: ở nốt có dây lớn nhất trong hợp âm
                    if same.iter().any(|m| m.string > n.string) {
                        Vec::new()
                    } else {
                        same
                    }
                }
                None => vec![n],
            };
            let label = tech_label(&group);
            if !label.is_empty() {
                let up = g.row(n.string) > 0.0;
                let lift = if n.bend > 0.0 && up {
                    sr.height() * (0.35 + 0.32 * (n.bend.round().clamp(1.0, 4.0) as f32))
                } else {
                    sr.height() * 0.35
                };
                let size = (sr.height() * 0.45).clamp(9.0, 15.0);
                let fade = a * ((2.2 - u) / 0.4).clamp(0.0, 1.0) as f32;
                tech_tag(p, pos2(sr.center().x, sr.min.y - lift), &label, size, col, fade);
            }
        }
    }
    while ci > ci0 {
        ci -= 1;
        let ch = &c.chords[ci];
        let (lo, hi) = chord_span(c, ch);
        frame(p, &g, ch, lo, hi, false);
    }
    for arp in &c.arps {
        let (lo, hi) = chord_span(c, arp);
        frame(p, &g, arp, lo, hi, true);
    }

    // hits: the gem lit and its glow growing, then a soft smear along the string thinning to a streak, gone at 0.3 s;
    // the fret's two wires flash (a thin white core, a narrow pink halo) meanwhile; a chord flashes its frame's two outer
    // posts. MISS / LATE / EARLY sit on the string at the gem, a pale tint of its colour, briefly.
    let mut words: Vec<Pos2> = Vec::new();
    let flash = |f: i32, a: f32, halo: Color32| {
        let b = g.to(g.on_row(f as f64, 5.3));
        let t = g.to(g.on_row(f as f64, -0.45));
        soft_line(
            p,
            b,
            t,
            g.sz(0.05 * pf).max(1.5),
            g.sz(4.0),
            alpha(halo, 0.35 * a),
        );
        p.line_segment([b, t], Stroke::new(g.sz(0.05 * pf).max(1.5), alpha(BEAM_CORE, a)));
    };
    let mut chord_flash: Vec<(usize, f32)> = Vec::new();
    for &(_, _, j, age, _, _, k) in &hits {
        if let Some(k) = k
            && j != Judgement::Miss
            && age < HIT_GONE
            && !chord_flash.iter().any(|b| b.0 == k)
        {
            chord_flash.push((k, age));
        }
    }
    for (k, age) in chord_flash {
        let (lo, hi) = chord_span(c, &c.chords[k]);
        let a = if age < HIT_GONE - 0.07 {
            1.0
        } else {
            (HIT_GONE - age) / 0.07
        };
        for f in [lo - 1, hi] {
            flash(f, a, rgb(255, 90, 200));
        }
    }
    for (home, col, j, age, fret, sounding, chord) in hits {
        if j != Judgement::Miss {
            if fret > 0 && chord.is_none() && age < HIT_GONE {
                let a = if age < HIT_GONE - 0.07 {
                    1.0
                } else {
                    (HIT_GONE - age) / 0.07
                };
                for f in [fret - 1, fret] {
                    flash(f, a, rgb(255, 150, 170));
                }
            }
            let c0 = home.center();
            if age < HIT_BLOOM {
                let t = age / HIT_BLOOM;
                let r = Rect::from_center_size(c0, home.size() * vec2(1.0 + 0.08 * t, 1.0 + 0.1 * t));
                if fret > 0 {
                    block(p, &g, r, 0.0, 0.0, col, 1.0, 0.6 + 0.4 * t);
                }
            } else if age < HIT_GONE && !sounding {
                let t = (age - HIT_BLOOM) / (HIT_GONE - HIT_BLOOM);
                let r = Rect::from_center_size(c0, home.size() * vec2(1.3 + 0.2 * t, 0.5 - 0.38 * t));
                let rr = Rect::from_min_max(g.to(r.min), g.to(r.max));
                let a = 1.0 - t;
                feather_rect(
                    p,
                    [rr.left_top(), rr.right_top(), rr.right_bottom(), rr.left_bottom()],
                    rr.height() * 1.2,
                    alpha(lut(col, 0.65), 0.5 * a),
                );
                p.rect_filled(rr, rr.height() / 2.0, alpha(lut(col, 0.7), a));
                p.line_segment(
                    [rr.left_center(), rr.right_center()],
                    Stroke::new((rr.height() * 0.3).max(0.8), alpha(Color32::WHITE, 0.8 * a)),
                );
            }
        }
        if j != Judgement::Hit && age < WORD {
            let at = g.to(home.center());
            if words.iter().any(|w| (*w - at).length() < g.sz(40.0)) {
                continue;
            }
            words.push(at);
            let word = match j {
                Judgement::Miss => "MISS",
                Judgement::Late => "LATE",
                _ => "EARLY",
            };
            let a = ((WORD - age) / 0.1).min(1.0);
            let size = g.sz(0.27 * pf);
            let ink = mix(rgb(250, 245, 110), mix(col, Color32::WHITE, 0.6), 0.3);
            let at = at - vec2(0.0, size * 0.75);
            text::draw(
                p,
                word,
                at + vec2(0.0, g.sz(1.5)),
                Align2::CENTER_CENTER,
                Style::new(size, 900, alpha(Color32::BLACK, 0.5 * a)),
                None,
            );
            text::draw(
                p,
                word,
                at,
                Align2::CENTER_CENTER,
                Style::new(size, 900, alpha(ink, a)),
                None,
            );
        }
    }
}

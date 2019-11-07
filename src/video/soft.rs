use super::{QuadStrip, RgbColor, Vertex};
use crate::data;

pub const SCR_W: u16 = 320;
pub const SCR_H: u16 = 200;

const COL_ALPHA: u8 = 0x10;
const COL_PAGE: u8 = 0x11;

pub const FB_SIZE: usize = (SCR_W * SCR_H) as usize;

pub struct State {
    fb: [[u8; FB_SIZE]; 4],
    pal: [RgbColor; 16],
}

pub fn clear_fb(s: &mut State, fb: u8, color: u8) {
    for b in s.fb[usize::from(fb)].iter_mut() {
        *b = color;
    }
}

pub fn copy_fb(s: &mut State, dst_fb: u8, src_fb: u8, v_scroll: i32) {
    assert_ne!(dst_fb, src_fb);
    let dst = s.fb[usize::from(dst_fb)].as_mut_ptr();
    let src = s.fb[usize::from(src_fb)].as_ptr();
    let count = if -199 <= v_scroll && v_scroll <= 199 {
        if v_scroll < 0 {
            unsafe {
                src.add((-v_scroll as usize) * usize::from(SCR_W));
            }
            (i32::from(SCR_H) + v_scroll) * i32::from(SCR_W)
        } else if v_scroll > 0 {
            unsafe {
                dst.add((v_scroll as usize) * usize::from(SCR_W));
            }
            (i32::from(SCR_H) - v_scroll) * i32::from(SCR_W)
        } else {
            i32::from(SCR_W * SCR_H)
        }
    } else {
        0
    };

    unsafe {
        std::ptr::copy_nonoverlapping(src, dst, count as usize);
    }
}

pub fn draw_point(s: &mut State, fb: u8, x: u16, y: u16, color: u8) {
    let color = match color {
        COL_ALPHA => grab(s, fb, x, y) | 8,
        COL_PAGE => grab(s, 0, x, y),
        _ => color,
    };
    out(s, fb, x, y, color);
}

pub fn draw_polygon(s: &mut State, fb: u8, qs: &QuadStrip, color: u8) {
    let vs = qs.vertices();
    if vs.len() <= 2 {
        return;
    }

    let mut i = 0;
    let mut j = vs.len() - 1;

    let mut x2 = vs[i].x;
    let mut x1 = vs[j].x;
    let mut h_line_y = std::cmp::min(vs[i].y, vs[j].y);

    i += 1;
    j -= 1;

    let draw_h_line = match color {
        COL_ALPHA => draw_h_line_alpha,
        COL_PAGE => draw_h_line_page,
        _ => draw_h_line_color,
    };

    let mut cpt1 = (x1 as u32) << 16;
    let mut cpt2 = (x2 as u32) << 16;

    let mut count = vs.len();
    'top: while count > 2 {
        count -= 2;

        let (step1, _) = calc_step(vs[j + 1], vs[j]);
        let (step2, mut h) = calc_step(vs[i - 1], vs[i]);

        i += 1;
        j -= 1;

        cpt1 = (cpt1 & 0xFFFF0000) | 0x7FFF;
        cpt2 = (cpt2 & 0xFFFF0000) | 0x8000;

        if h == 0 {
            cpt1 = cpt1.wrapping_add(step1);
            cpt2 = cpt2.wrapping_add(step2);
        } else {
            while h > 0 {
                h -= 1;
                if h_line_y >= 0 {
                    x1 = (cpt1 >> 16) as i16;
                    x2 = (cpt2 >> 16) as i16;
                    if x1 < (SCR_W as i16) && x2 >= 0 {
                        if x1 < 0 {
                            x1 = 0;
                        }
                        if x2 >= (SCR_W as i16) {
                            x2 = (SCR_W as i16) - 1;
                        }

                        let x_max = std::cmp::max(x1, x2);
                        let x_min = std::cmp::min(x1, x2);
                        let w = x_max - x_min + 1;
                        let offset = i32::from(h_line_y) * i32::from(SCR_W) + i32::from(x_min);
                        draw_h_line(s, fb, offset as usize, w as u16, color);
                    }
                }
                cpt1 = cpt1.wrapping_add(step1);
                cpt2 = cpt2.wrapping_add(step2);
                h_line_y += 1;
                if h_line_y >= (SCR_H as i16) {
                    break 'top;
                }
            }
        }
    }
}

fn calc_step(v1: Vertex, v2: Vertex) -> (u32, u16) {
    let dy = (v2.y - v1.y) as u16;
    let delta = if dy == 0 { 1 } else { dy };
    let step = (i32::from(v2.x - v1.x) << 16) / i32::from(delta);
    (step as u32, dy)
}

fn draw_h_line_alpha(s: &mut State, fb: u8, offset: usize, w: u16, _color: u8) {
    let p = &mut s.fb[usize::from(fb)][offset..];
    for i in 0..usize::from(w) {
        p[i] |= 8;
    }
}

fn draw_h_line_page(s: &mut State, fb: u8, offset: usize, w: u16, _color: u8) {
    if fb != 0 {
        for i in 0..usize::from(w) {
            let src_color = s.fb[0][offset + i];
            s.fb[usize::from(fb)][offset + i] = src_color;
        }
    }
}

fn draw_h_line_color(s: &mut State, fb: u8, offset: usize, w: u16, color: u8) {
    let p = &mut s.fb[usize::from(fb)][offset..];
    for i in 0..usize::from(w) {
        p[i] = color;
    }
}

pub fn draw_char(s: &mut State, fb: u8, x: u16, y: u16, c: char, color: u8) {
    if x <= SCR_W - 8 && y <= SCR_H - 8 {
        let glyph = (u32::from(c) - 0x20) * 8;
        for j in 0..8 {
            let line = data::FONT[(glyph as usize) + usize::from(j)];
            for i in (0..8).filter(|i| pixel_in_font_line(line, *i)) {
                out(s, fb, x + u16::from(i), y + j, color);
            }
        }
    }
}

fn pixel_in_font_line(line: u8, pixel: u8) -> bool {
    (line & (1 << (7 - pixel))) != 0
}

pub fn draw_bitmap(s: &mut State, fb: u8, data: &[u8; FB_SIZE]) {
    s.fb[usize::from(fb)].copy_from_slice(data);
}

fn out(s: &mut State, fb: u8, x: u16, y: u16, color: u8) {
    assert!(x < SCR_W && y < SCR_H);
    s.fb[usize::from(fb)][usize::from(y * SCR_W + x)] = color;
}

fn grab(s: &mut State, fb: u8, x: u16, y: u16) -> u8 {
    s.fb[usize::from(fb)][usize::from(y * SCR_W + x)]
}

impl State {
    pub fn new() -> Self {
        let fb = [[0; FB_SIZE], [0; FB_SIZE], [0; FB_SIZE], [0; FB_SIZE]];
        Self {
            fb,
            pal: Default::default(),
        }
    }

    pub fn read_pixels(&self, fb: u8, out: &mut [u16]) {
        let src = &self.fb[usize::from(fb)];
        for (i, pixel) in src.iter().enumerate() {
            out[i] = self.pal[usize::from(*pixel)].as_rgb565();
        }
    }

    pub fn set_pal(&mut self, pal: [RgbColor; 16]) {
        self.pal = pal;
    }
}

impl RgbColor {
    fn as_rgb565(self) -> u16 {
        let r = (u16::from(self.r) & 0xF8) << 8;
        let g = (u16::from(self.g) & 0xFC) << 3;
        let b = u16::from(self.b) >> 3;
        r | g | b
    }
}

use super::data;
use super::Game;
use byteorder::{ByteOrder, BE};
use std::convert::TryFrom;

pub mod soft;

pub struct VideoContext {
    pub rndr: soft::State,
    fb_xlat: [u8; 3],
    // Data counter
    dc: u16,
    use_seg2: bool,
    // This can only be true for DOS data-set.
    use_ega_pal: bool,
    current_pal_num: Option<u8>,
    needs_pal_fixup: bool,
}

pub struct QuadStrip {
    vertices: [Vertex; 70],
    count: usize,
}

#[derive(Default, Clone, Copy)]
pub struct Vertex {
    pub x: i16,
    pub y: i16,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl QuadStrip {
    pub fn new() -> Self {
        Self {
            vertices: [Default::default(); 70],
            count: 0,
        }
    }

    pub fn push(&mut self, vertex: Vertex) {
        assert_ne!(self.count, self.vertices.len());
        self.vertices[self.count] = vertex;
        self.count += 1;
    }

    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices[0..self.count]
    }
}

pub fn select_page(v: &mut VideoContext, n: u8) {
    let n = translate_page(v, n);
    v.fb_xlat[0] = n;
}

pub fn fill_page(v: &mut VideoContext, n: u8, color: u8) {
    let n = translate_page(v, n);
    soft::clear_fb(&mut v.rndr, n, color)
}

pub fn copy_page(v: &mut VideoContext, src: u8, dst: u8, v_scroll: i16) {
    let dst = translate_page(v, dst);
    if src >= 0xFE {
        let src = translate_page(v, src);
        soft::copy_fb(&mut v.rndr, dst, src, 0);
    } else if (src & 0x80) == 0 {
        let src = translate_page(v, src & 0xBF);
        soft::copy_fb(&mut v.rndr, dst, src, 0);
    } else {
        let src = translate_page(v, src & 3);
        if src != dst && (-199..=199).contains(&v_scroll) {
            soft::copy_fb(&mut v.rndr, dst, src, i32::from(v_scroll));
        }
    }
}

pub fn swap_pages(v: &mut VideoContext, new_front_fb: u8) -> u8 {
    if new_front_fb != 0xFE {
        if new_front_fb == 0xFF {
            v.fb_xlat.swap(1, 2);
        } else {
            v.fb_xlat[1] = translate_page(v, new_front_fb);
        }
    }

    v.fb_xlat[1]
}

fn translate_page(v: &VideoContext, n: u8) -> u8 {
    match n {
        0..=3 => n,
        0xFE => v.fb_xlat[1],
        0xFF => v.fb_xlat[2],
        _ => {
            log::warn!("invalid page index {}", n);
            0
        }
    }
}

pub fn draw_shape(g: &mut Game, x: i16, y: i16, zoom: u16, color: u8) {
    let i = fetch_u8(g);
    if i >= 0xC0 {
        let color = if (color & 0x80) != 0 { i & 0x3F } else { color };

        let old_dc = g.video.dc;
        fill_polygon(g, x, y, zoom, color);
        g.video.dc = old_dc;
    } else {
        let i = i & 0x3F;
        if i == 2 {
            draw_shape_parts(g, x, y, zoom);
        } else {
            log::warn!("invalid video op {}", i);
        }
    }
}

fn fill_polygon(g: &mut Game, x: i16, y: i16, zoom: u16, color: u8) {
    let bbw = fetch_dim(g, zoom);
    let bbh = fetch_dim(g, zoom);

    let x1 = i16::try_from(i32::from(x) - i32::from(bbw / 2)).unwrap();
    let x2 = i16::try_from(i32::from(x) + i32::from(bbw / 2)).unwrap();
    let y1 = i16::try_from(i32::from(y) - i32::from(bbh / 2)).unwrap();
    let y2 = i16::try_from(i32::from(y) + i32::from(bbh / 2)).unwrap();

    if x1 > 319 || x2 < 0 || y1 > 199 || y2 < 0 {
        return;
    }

    let mut qs = QuadStrip::new();
    let num = fetch_u8(g);

    if (num & 1) != 0 {
        log::warn!("unexpected number of vertices {}", num);
        return;
    }

    for _ in 0..num {
        let x = x1 + fetch_dim(g, zoom);
        let y = y1 + fetch_dim(g, zoom);
        qs.push(Vertex { x, y })
    }

    let fb = g.video.fb_xlat[0];
    if num == 4 && bbw == 0 && bbh <= 1 {
        soft::draw_point(&mut g.video.rndr, fb, x as u16, y as u16, color);
    } else {
        soft::draw_polygon(&mut g.video.rndr, fb, &qs, color);
    }
}

fn fetch_dim(g: &mut Game, zoom: u16) -> i16 {
    i16::try_from(u32::from(fetch_u8(g)) * u32::from(zoom) / 64).unwrap()
}

fn draw_shape_parts(g: &mut Game, x: i16, y: i16, zoom: u16) {
    let x = x.wrapping_sub(fetch_dim(g, zoom));
    let y = y.wrapping_sub(fetch_dim(g, zoom));
    let n = fetch_u8(g);
    for _ in 0..=n {
        let offset = fetch_u16(g);
        let x = x.wrapping_add(fetch_dim(g, zoom));
        let y = y.wrapping_add(fetch_dim(g, zoom));

        let color = if (offset & 0x8000) != 0 {
            let hi = fetch_u8(g);
            let _lo = fetch_u8(g);
            hi & 0x7F
        } else {
            0xFF
        };

        let old_offset = std::mem::replace(&mut g.video.dc, offset << 1);
        draw_shape(g, x, y, zoom, color);
        g.video.dc = old_offset;
    }
}

pub fn draw_string(v: &mut VideoContext, mut xi: u16, mut ypos: u16, str_id: u16, color: u8) {
    let text = if let Some(s) = find_string(data::STRINGS_EN, str_id) {
        s
    } else {
        log::warn!("unknown string {}", str_id);
        return;
    };

    let left = xi;
    for c in text.chars() {
        if c == '\n' {
            xi = left;
            ypos += 8;
        } else {
            let next_xi = xi + 1;
            let xpos = std::mem::replace(&mut xi, next_xi) * 8;
            let fb = v.fb_xlat[0];
            soft::draw_char(&mut v.rndr, fb, xpos, ypos, c, color);
        }
    }
}

fn find_string(table: &[(u16, &'static str)], id: u16) -> Option<&'static str> {
    table.iter().find(|item| item.0 == id).map(|item| item.1)
}

#[allow(clippy::identity_op)]
#[allow(clippy::erasing_op)]
pub fn copy_bitmap(v: &mut VideoContext, mem: &[u8]) {
    let mut image = [0; 320 * 200];
    let mut di = 0;

    for y in 0..200 {
        for w in 0..40 {
            let n = y * 40 + w;
            let mut p = [
                mem[8000 * 3 + n],
                mem[8000 * 2 + n],
                mem[8000 * 1 + n],
                mem[8000 * 0 + n],
            ];

            for _ in 0..4 {
                let mut acc = 0;
                for i in 0..8 {
                    acc <<= 1;
                    acc |= p[i & 3] >> 7;
                    p[i & 3] <<= 1;
                }

                image[di] = acc >> 4;
                image[di + 1] = acc & 0x0F;
                di += 2;
            }
        }
    }

    soft::draw_bitmap(&mut v.rndr, 0, &image);
}

impl VideoContext {
    pub fn new() -> Self {
        Self {
            rndr: soft::State::new(),
            fb_xlat: [2, 2, 1],
            dc: 0,
            use_seg2: false,
            use_ega_pal: false,
            current_pal_num: None,
            needs_pal_fixup: true,
        }
    }

    pub fn needs_pal_fixup(&self) -> bool {
        self.needs_pal_fixup
    }

    pub fn invalidate_pal_num(&mut self) {
        self.current_pal_num = None;
    }

    pub fn set_dc(&mut self, new_dc: u16, use_seg2: bool) {
        self.dc = new_dc;
        self.use_seg2 = use_seg2;
    }

    pub fn set_use_ega_pal(&mut self, on: bool) {
        self.use_ega_pal = on;
    }
}

fn fetch_u8(g: &mut Game) -> u8 {
    let base = if g.video.use_seg2 {
        g.mem.seg_video2()
    } else {
        g.mem.seg_video1()
    };
    let b = g.mem.data[base + usize::from(g.video.dc)];
    g.video.dc += 1;
    b
}

fn fetch_u16(g: &mut Game) -> u16 {
    let hi = u16::from(fetch_u8(g));
    let lo = u16::from(fetch_u8(g));
    (hi << 8) | lo
}

pub fn load_pal_mem(g: &mut Game, num: u8) {
    let v = &mut g.video;
    if num < 32 && v.current_pal_num != Some(num) {
        let mem = &g.mem.data[g.mem.seg_video_pal()..];
        let pal = if v.use_ega_pal {
            read_ega_pal(mem, num)
        } else {
            read_vga_pal(mem, num)
        };
        v.rndr.set_pal(pal);
        v.current_pal_num = Some(num);
    }
}

const PAL_SIZE: usize = 16;

fn read_ega_pal(mem: &[u8], num: u8) -> [RgbColor; PAL_SIZE] {
    // EGA colors are stored after VGA.
    let begin = 1024 + usize::from(num) * PAL_SIZE * 2;
    let mut pal = [Default::default(); PAL_SIZE];
    for i in 0..PAL_SIZE {
        let color = BE::read_u16(&mem[begin + i * 2..]);
        let (r, g, b) = EGA_PAL[usize::from((color >> 12) & 0xF)];
        pal[i] = RgbColor { r, g, b };
    }
    pal
}

fn read_vga_pal(mem: &[u8], num: u8) -> [RgbColor; PAL_SIZE] {
    let begin = usize::from(num) * PAL_SIZE * 2;
    let mut pal = [Default::default(); PAL_SIZE];
    for i in 0..PAL_SIZE {
        let color = BE::read_u16(&mem[begin + i * 2..]);
        let extract_component = |shift: u16| {
            let component = ((color >> shift) & 0x0F) as u8;
            component | (component << 4)
        };
        pal[i] = RgbColor {
            r: extract_component(8),
            g: extract_component(4),
            b: extract_component(0),
        };
    }
    pal
}

// from https://en.wikipedia.org/wiki/Enhanced_Graphics_Adapter
const EGA_PAL: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00), // black #0
    (0x00, 0x00, 0xAA), // blue #1
    (0x00, 0xAA, 0x00), // green #2
    (0x00, 0xAA, 0xAA), // cyan #3
    (0xAA, 0x00, 0x00), // red #4
    (0xAA, 0x00, 0xAA), // magenta #5
    (0xAA, 0x55, 0x00), // yellow, brown #20
    (0xAA, 0xAA, 0xAA), // white, light gray #7
    (0x55, 0x55, 0x55), // dark gray, bright black #56
    (0x55, 0x55, 0xFF), // bright blue #57
    (0x55, 0xFF, 0x55), // bright green #58
    (0x55, 0xFF, 0xFF), // bright cyan #59
    (0xFF, 0x55, 0x55), // bright red #60
    (0xFF, 0x55, 0xFF), // bright magenta #61
    (0xFF, 0xFF, 0x55), // bright yellow #62
    (0xFF, 0xFF, 0xFF), // bright white #63
];

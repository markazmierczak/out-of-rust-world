use crate::video::soft::{FB_SIZE, SCR_H, SCR_W};
use crate::Game;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;

pub struct Host {
    sdl_context: sdl2::Sdl,
    video_subsystem: sdl2::VideoSubsystem,
    surface: sdl2::render::Texture,
    color_buffer: Vec<u16>,
    canvas: sdl2::render::Canvas<sdl2::video::Window>,
}

fn as_u8_slice(v: &[u16]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            v.as_ptr() as *const u8,
            v.len() * std::mem::size_of::<u16>(),
        )
    }
}

pub fn display_surface(g: &mut Game, fb: u8) {
    g.video.rndr.read_pixels(fb, &mut g.host.color_buffer);
    g.host
        .surface
        .update(
            None,
            as_u8_slice(&g.host.color_buffer),
            usize::from(SCR_W * 2),
        )
        .unwrap();
    g.host.canvas.copy(&g.host.surface, None, None).unwrap();
    g.host.canvas.present();
}

impl Host {
    pub fn new() -> Self {
        let sdl_context = sdl2::init().unwrap();
        let video_subsystem = sdl_context.video().unwrap();

        let window = video_subsystem
            .window("rust-sdl2 demo", 800, 600)
            .position_centered()
            .build()
            .unwrap();

        let mut canvas = window.into_canvas().build().unwrap();
        let texture_creator = canvas.texture_creator();
        let surface = texture_creator
            .create_texture_streaming(
                sdl2::pixels::PixelFormatEnum::RGB565,
                SCR_W.into(),
                SCR_H.into(),
            )
            .unwrap();

        canvas.set_draw_color(Color::RGB(0, 255, 255));
        canvas.clear();
        canvas.present();

        let _event_pump = sdl_context.event_pump().unwrap();

        Self {
            sdl_context,
            video_subsystem,
            canvas,
            surface,
            color_buffer: vec![0; FB_SIZE],
        }
    }
}

/* TODO:
for event in event_pump.poll_iter() {
    match event {
        Event::Quit { .. }
        | Event::KeyDown {
            keycode: Some(Keycode::Escape),
            ..
        } => break 'running,
        _ => {}
    }
}*/

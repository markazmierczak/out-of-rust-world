use crate::video::soft::{FB_SIZE, SCR_H, SCR_W};
use crate::{sfx, Game};
use sdl2::pixels::Color;

pub struct Host {
    #[allow(dead_code)]
    sdl_context: sdl2::Sdl,
    #[allow(dead_code)]
    video_subsystem: sdl2::VideoSubsystem,
    surface: sdl2::render::Texture,
    color_buffer: Vec<u16>,
    canvas: sdl2::render::Canvas<sdl2::video::Window>,
    event_pump: sdl2::EventPump,

    #[allow(dead_code)]
    mixer_context: sdl2::mixer::Sdl2MixerContext,
    audio_cvt: sdl2::audio::AudioCVT,
    audio_channels: [AudioChannel<u8>; 4],
    music_channel: AudioChannel<i16>,
    wants_quit: bool,
    wants_pause: bool,
}

#[derive(Default)]
struct AudioChannel<T> {
    chunk: Option<sdl2::mixer::Chunk>,
    samples: Vec<T>,
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

        // TODO: full-screen
        let window = video_subsystem
            .window("Out Of Rust World", 800, 600)
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

        let event_pump = sdl_context.event_pump().unwrap();

        use sdl2::audio::AudioFormat;
        let audio_cvt = sdl2::audio::AudioCVT::new(
            AudioFormat::S8,
            1,
            sfx::GAME_RATE.into(),
            AudioFormat::s16_sys(),
            2,
            sfx::HOST_RATE.into(),
        )
        .unwrap();

        let mixer_context = sdl2::mixer::init(sdl2::mixer::InitFlag::MID).unwrap();
        sdl2::mixer::open_audio(sfx::HOST_RATE.into(), sdl2::mixer::AUDIO_S16SYS, 2, 4096).unwrap();
        sdl2::mixer::allocate_channels(5);

        Self {
            sdl_context,
            video_subsystem,
            canvas,
            surface,
            color_buffer: vec![0; FB_SIZE],
            mixer_context,
            audio_channels: Default::default(),
            // FIXME: use frame rate constant
            music_channel: AudioChannel {
                chunk: None,
                samples: vec![0; usize::from(sfx::HOST_RATE) / 50],
            },
            audio_cvt,
            event_pump,
            wants_quit: false,
            wants_pause: false,
        }
    }

    pub fn wants_quit(&self) -> bool {
        self.wants_quit
    }

    pub fn wants_pause(&self) -> bool {
        self.wants_pause
    }
}

pub fn play_sound(
    h: &mut Host,
    channel: u8,
    freq: u16,
    volume: u8,
    data: &[u8],
    len: usize,
    loops: i32,
) {
    assert!(sfx::GAME_RATE / freq <= 4);
    stop_sound(h, channel);

    let ac = &mut h.audio_channels[usize::from(channel)];
    ac.samples.resize(h.audio_cvt.capacity(len * 4), 0);

    let mut pos = sfx::Frac::new(freq, sfx::GAME_RATE);
    let mut n = 0;
    while pos.int() < (len as u32) {
        ac.samples[n] = data[pos.int() as usize];
        n += 1;
        pos.inc();
    }
    ac.samples.truncate(n);
    ac.samples = h
        .audio_cvt
        .convert(std::mem::replace(&mut ac.samples, Vec::new()));

    ac.chunk = Some({
        let raw_chunk = unsafe {
            sdl2::sys::mixer::Mix_QuickLoad_RAW(ac.samples.as_mut_ptr(), ac.samples.len() as u32)
        };
        sdl2::mixer::Chunk {
            raw: raw_chunk,
            owned: true,
        }
    });

    let channel = sdl2::mixer::Channel(channel.into());
    channel.play(ac.chunk.as_ref().unwrap(), loops).unwrap();
    channel.set_volume(i32::from(volume) * sdl2::mixer::MAX_VOLUME / 63);
}

pub fn stop_sound(h: &mut Host, channel: u8) {
    sdl2::mixer::Channel(channel.into()).halt();
    h.audio_channels[usize::from(channel)].chunk = None;
}

pub fn push_music_frame(g: &mut Game) {
    if g.music.is_end_of_track() {
        return;
    }

    let mut samples = std::mem::replace(&mut g.host.music_channel.samples, Vec::new());
    sfx::mix_samples(g, &mut samples);
    let channel = &mut g.host.music_channel;
    channel.samples = samples;
    channel.chunk = Some({
        let raw_chunk = unsafe {
            sdl2::sys::mixer::Mix_QuickLoad_RAW(
                channel.samples.as_mut_ptr() as *mut u8,
                channel.samples.len() as u32,
            )
        };
        sdl2::mixer::Chunk {
            raw: raw_chunk,
            owned: true,
        }
    });
}

pub fn process_input(g: &mut Game) {
    use sdl2::event::Event;
    use sdl2::keyboard::Keycode;
    use std::convert::TryFrom;

    for event in g.host.event_pump.poll_iter() {
        match event {
            Event::Quit { .. }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            } => g.host.wants_quit = true,

            Event::KeyDown {
                keycode: Some(k), ..
            } => {
                match k {
                    Keycode::Left => g.input.left = true,
                    Keycode::Right => g.input.right = true,
                    Keycode::Up => g.input.up = true,
                    Keycode::Down => g.input.down = true,
                    Keycode::Space | Keycode::Return => g.input.button = true,
                    Keycode::P => g.host.wants_pause = !g.host.wants_pause,
                    _ => {}
                }
                g.input.last_char = u8::try_from(k as i32).ok();
            }

            Event::KeyUp {
                keycode: Some(k), ..
            } => match k {
                Keycode::Left => g.input.left = false,
                Keycode::Right => g.input.right = false,
                Keycode::Up => g.input.up = false,
                Keycode::Down => g.input.down = false,
                Keycode::Space | Keycode::Return => g.input.button = false,
                _ => {}
            },

            _ => {}
        }
    }
}

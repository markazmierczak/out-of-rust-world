use crate::video::soft::{FB_SIZE, SCR_H, SCR_W};
use crate::{sfx, Game};
use sdl2::pixels::Color;

const MUSIC_SAMPLES_PER_FRAME: usize = (sfx::HOST_RATE as usize) / 50 * 2;
const MUSIC_BUFFER_LEN: usize = MUSIC_SAMPLES_PER_FRAME * 8;

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
    music_chan: rb::SpscRb<i16>,
    music_chan_prod: rb::Producer<i16>,
    music_buf: std::rc::Rc<std::cell::RefCell<Vec<i16>>>,
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
    pub fn new(fullscreen: bool) -> Self {
        use rb::RB;

        let sdl_context = sdl2::init().unwrap();
        let video_subsystem = sdl_context.video().unwrap();

        let mut window = video_subsystem.window("Out Of Rust World", 800, 600);

        if fullscreen {
            window.fullscreen();
        } else {
            window.position_centered();
        }

        let window = window.build().unwrap();

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

        let mixer_context = init_mixer();
        sdl2::mixer::open_audio(sfx::HOST_RATE.into(), sdl2::mixer::AUDIO_S16SYS, 2, 4096).unwrap();
        sdl2::mixer::allocate_channels(4);

        let music_chan = rb::SpscRb::new(MUSIC_BUFFER_LEN);
        let (music_chan_prod, music_chan_cons) = (music_chan.producer(), music_chan.consumer());

        unsafe {
            sdl2::sys::mixer::Mix_HookMusic(
                Some(consume_music),
                Box::into_raw(Box::new(music_chan_cons)) as *mut libc::c_void,
            );
        }

        Self {
            sdl_context,
            video_subsystem,
            canvas,
            surface,
            color_buffer: vec![0; FB_SIZE],
            mixer_context,
            audio_channels: Default::default(),
            audio_cvt,
            music_chan,
            music_chan_prod,
            music_buf: std::cell::RefCell::new(Vec::new()).into(),
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

fn init_mixer() -> sdl2::mixer::Sdl2MixerContext {
    let ret = unsafe { sdl2::sys::mixer::Mix_Init(0) };
    assert_eq!(ret, 0);
    sdl2::mixer::Sdl2MixerContext
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
        .convert(std::mem::take(&mut ac.samples));

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

pub fn produce_music(g: &mut Game) {
    use rb::{RbInspector, RbProducer};

    if g.music.is_end_of_track() {
        return;
    }

    let buf = g.host.music_buf.clone();
    let mut buf = buf.borrow_mut();
    buf.resize(g.host.music_chan.slots_free(), 0);
    sfx::mix_samples(g, &mut *buf);
    g.host.music_chan_prod.write(&*buf).unwrap();
}

#[allow(clippy::cast_ptr_alignment)]
unsafe extern "C" fn consume_music(udata: *mut libc::c_void, stream: *mut u8, len: libc::c_int) {
    use rb::RbConsumer;
    let music_chan_cons = (udata as *mut rb::Consumer<i16>).as_ref().unwrap();
    let out = std::slice::from_raw_parts_mut(stream as *mut i16, (len as usize) / 2);
    let count = music_chan_cons.read(out).unwrap_or(0);
    for sample in &mut out[count..] {
        *sample = 0;
    }
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

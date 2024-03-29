use std::str::FromStr;

mod bytekiller;
mod data;
mod host;
mod mem;
#[allow(dead_code)]
mod pak;
mod script;
mod sfx;
mod video;

use host::Host;
use mem::Memory;
use script::Vm;
use video::VideoContext;

// FIXME: ability to resize a window during gameplay

pub struct Game {
    mem: Memory,
    vm: Vm,
    video: VideoContext,
    current_part: u16,
    next_part: Option<u16>,
    screen_num: Option<i16>,
    next_pal: Option<u8>,
    looping_gun_quirk: bool,
    bypass_protection: bool,

    music: sfx::Player,
    host: Host,
    input: script::Input,
}

pub fn run_frame(g: &mut Game) {
    script::stage_tasks(g);
    script::update_input(g);
    script::run_tasks(g);
}

pub fn main() {
    env_logger::init();

    let matches = clap::App::new("Another World in Rust")
        .version("1.0")
        .args_from_usage(
            "--fullscreen 'Display in fullscreen'
            --scene=[NUM] 'Start from given scene'
            --ega-pal 'Use EGA palette'",
        )
        .get_matches();

    let host = Host::new(matches.is_present("fullscreen"));

    let mut game = Game {
        host,
        video: VideoContext::new(),
        vm: Vm::new(),
        mem: Memory::new(),
        music: Default::default(),
        current_part: 0,
        next_part: None,
        screen_num: None,
        next_pal: None,
        looping_gun_quirk: false,
        bypass_protection: true,
        input: Default::default(),
    };

    game.video.set_use_ega_pal(matches.is_present("ega-pal"));

    let scene = matches
        .value_of("scene")
        .and_then(|s| u16::from_str(s).ok())
        .unwrap_or(16001);

    if scene < 36 {
        let (part, pos) = data::SCENE_POS[usize::from(scene)];
        script::restart_at(&mut game, part, pos);
    } else {
        script::restart_at(&mut game, scene, -1);
    }

    while !game.host.wants_quit() {
        if !game.host.wants_pause() {
            run_frame(&mut game);
        } else {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        host::process_input(&mut game);
    }
}

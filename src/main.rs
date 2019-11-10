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

    let host = Host::new();

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
    };

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

    loop {
        run_frame(&mut game);
    }
}

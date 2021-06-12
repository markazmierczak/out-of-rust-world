use super::{mem, sfx, video, Game};
use rand::Rng;
use std::time::{Duration, Instant};

const CALL_STACK_SIZE: u8 = 64;
const TASK_COUNT: usize = 64;

// Special program counter values to halt tasks.
const HALT_PC: u16 = 0xFFFF;
const PRE_HALT_PC: u16 = 0xFFFE;

mod reg_id {
    pub const RANDOM_SEED: usize = 0x3C;
    pub const SCREEN_NUM: usize = 0x67;
    pub const LAST_KEYCHAR: usize = 0xDA;
    pub const HERO_POS_UP_DOWN: usize = 0xE5;
    pub const MUSIC_SYNC: usize = 0xF4;
    pub const SCROLL_Y: usize = 0xF9;
    pub const HERO_ACTION: usize = 0xFA;
    pub const HERO_POS_JUMP_DOWN: usize = 0xFB;
    pub const HERO_POS_LEFT_RIGHT: usize = 0xFC;
    pub const HERO_POS_MASK: usize = 0xFD;
    pub const HERO_ACTION_POS_MASK: usize = 0xFE;
    pub const PAUSE_SLICES: usize = 0xFF;
}

#[derive(Debug, Clone, Copy)]
struct Task {
    pc: u16,
    frozen: bool,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            pc: HALT_PC,
            frozen: false,
        }
    }
}

pub struct Vm {
    regs: [i16; 256],
    call_stack: [u16; CALL_STACK_SIZE as usize],
    // Program counter of current task.
    pc: u16,
    // Call-stack pointer
    sp: u8,
    tasks: [Task; TASK_COUNT],
    pending_tasks: [Task; TASK_COUNT],
    needs_yield: bool,
    last_swap_time: Instant,
}

impl Vm {
    pub fn new() -> Self {
        let mut vm = Self {
            regs: [0; 256],
            call_stack: [0; CALL_STACK_SIZE as usize],
            pc: 0,
            sp: 0,
            tasks: [Default::default(); TASK_COUNT],
            pending_tasks: [Default::default(); TASK_COUNT],
            needs_yield: false,
            last_swap_time: Instant::now(),
        };

        vm.regs[reg_id::RANDOM_SEED] = rand::thread_rng().gen();
        // bypass the protection
        vm.regs[0xBC] = 0x10;
        vm.regs[0xC6] = 0x80;
        vm.regs[0xF2] = 4000;
        vm.regs[0xDC] = 33;

        vm
    }

    pub fn sync_music(&mut self, val: u16) {
        self.regs[reg_id::MUSIC_SYNC] = val as i16;
    }
}

#[derive(Default)]
pub struct Input {
    pub last_char: Option<u8>,
    pub right: bool,
    pub left: bool,
    pub down: bool,
    pub up: bool,
    pub button: bool,
}

fn is_valid_keychar(c: u8) -> bool {
    c == 0x08 || (b'a'..=b'z').contains(&c)
}

fn make_dir(ul: bool, rd: bool) -> i16 {
    match (ul, rd) {
        (false, false) => 0,
        (false, true) => 1,
        (true, _) => -1,
    }
}

pub fn update_input(g: &mut Game) {
    let regs = &mut g.vm.regs;
    let input = &mut g.input;

    if g.current_part == 16009 {
        regs[reg_id::LAST_KEYCHAR] = match input.last_char.take() {
            Some(c) if is_valid_keychar(c) => c & !0x20,
            _ => 0,
        }
        .into();
    }

    regs[reg_id::HERO_POS_LEFT_RIGHT] = make_dir(input.left, input.right);
    regs[reg_id::HERO_POS_UP_DOWN] = make_dir(input.up, input.down);
    regs[reg_id::HERO_POS_JUMP_DOWN] = make_dir(input.up, input.down);

    let mask = u8::from(input.right)
        | (u8::from(input.left) << 1)
        | (u8::from(input.down) << 2)
        | (u8::from(input.up) << 3);

    regs[reg_id::HERO_POS_MASK] = mask.into();
    regs[reg_id::HERO_ACTION] = input.button.into();
    regs[reg_id::HERO_ACTION_POS_MASK] = (mask | (u8::from(input.button) << 7)).into();
}

fn fetch_u8(g: &mut Game) -> u8 {
    let offset = usize::from(g.vm.pc) + g.mem.seg_code();
    let b = g.mem.data[offset];
    g.vm.pc += 1;
    b
}

fn fetch_u16(g: &mut Game) -> u16 {
    // Big endian
    let hi = u16::from(fetch_u8(g));
    let lo = u16::from(fetch_u8(g));
    (hi << 8) | lo
}

fn fetch_i16(g: &mut Game) -> i16 {
    fetch_u16(g) as i16
}

fn fetch_index8(g: &mut Game) -> usize {
    usize::from(fetch_u8(g))
}

fn op_mov_const(g: &mut Game) {
    let dst = fetch_index8(g);
    let val = fetch_i16(g);
    log::trace!("movi @{:02X}, {}", dst, val);
    g.vm.regs[dst] = val;
}

fn op_mov(g: &mut Game) {
    let dst = fetch_index8(g);
    let src = fetch_index8(g);
    log::trace!("mov @x{:02X}, @x{:02X}", dst, src);
    g.vm.regs[dst] = g.vm.regs[src];
}

fn op_add_const(g: &mut Game) {
    if g.vm.pc == 0x6D48 && g.current_part == 16006 && !g.looping_gun_quirk {
        log::warn!("hack for non-stop looping gun sound bug");
        // The script 0x27 slot 0x17 doesn't stop the gun sound from looping.
        // This is a bug in the original game code, confirmed by Eric Chahi and
        // addressed with the anniversary editions.
        // For older releases (DOS, Amiga), we play the 'stop' sound like it is
        // done in other part of the game code.
        //
        //  (0x6D43) jmp(0x6CE5)
        //  (0x6D46) break
        //  (0x6D47) VAR(0x06) += -50
        //
        play_sound_shim(g, 0x5B, 1, 64, 1);
    }

    let dst = fetch_index8(g);
    let val = fetch_i16(g);
    log::trace!("addi @x{:02X}, {}", dst, val);
    g.vm.regs[dst] = i16::wrapping_add(g.vm.regs[dst], val);
}

fn op_add(g: &mut Game) {
    let dst = fetch_index8(g);
    let src = fetch_index8(g);
    log::trace!("add @x{:02X}, @x{:02X}", dst, src);
    g.vm.regs[dst] = i16::wrapping_add(g.vm.regs[dst], g.vm.regs[src]);
}

fn op_sub(g: &mut Game) {
    let dst = fetch_index8(g);
    let src = fetch_index8(g);
    log::trace!("add @x{:02X}, @x{:02X}", dst, src);
    g.vm.regs[dst] = i16::wrapping_sub(g.vm.regs[dst], g.vm.regs[src]);
}

fn op_and_const(g: &mut Game) {
    let dst = fetch_index8(g);
    let val = fetch_i16(g);
    log::trace!("andi @x{:02X}, {}", dst, val);
    g.vm.regs[dst] &= val;
}

fn op_or_const(g: &mut Game) {
    let dst = fetch_index8(g);
    let val = fetch_i16(g);
    log::trace!("ori @x{:02X}, {}", dst, val);
    g.vm.regs[dst] |= val;
}

fn op_shl_const(g: &mut Game) {
    let dst = fetch_index8(g);
    let val = fetch_i16(g);
    log::trace!("shli @x{:02X}, {}", dst, val);
    g.vm.regs[dst] <<= val;
}

fn op_shr_const(g: &mut Game) {
    let dst = fetch_index8(g);
    let val = fetch_u16(g);
    log::trace!("shri @x{:02X}, {}", dst, val);
    g.vm.regs[dst] = ((g.vm.regs[dst] as u16) >> val) as i16;
}

fn op_call(g: &mut Game) {
    assert!(g.vm.sp < CALL_STACK_SIZE, "call-stack overflow");
    let new_pc = fetch_u16(g);
    log::trace!("br 0x{:04X}", new_pc);
    g.vm.call_stack[usize::from(g.vm.sp)] = g.vm.pc;
    g.vm.pc = new_pc;
    g.vm.sp += 1;
}

fn op_ret(g: &mut Game) {
    assert!(g.vm.sp > 0, "call-stack underflow");
    log::trace!("ret");
    g.vm.sp -= 1;
    g.vm.pc = g.vm.call_stack[usize::from(g.vm.sp)];
}

fn op_jmp(g: &mut Game) {
    let new_pc = fetch_u16(g);
    log::trace!("b 0x{:04X}", new_pc);
    g.vm.pc = new_pc;
}

fn op_jmp_if_var(g: &mut Game) {
    let i = fetch_index8(g);
    let new_pc = fetch_u16(g);
    log::trace!("bif 0x{:04X}, @x{:02X}", new_pc, i);
    g.vm.regs[i] = g.vm.regs[i].wrapping_sub(1);

    if g.vm.regs[i] != 0 {
        g.vm.pc = new_pc;
    }
}

fn op_cond_jmp(g: &mut Game) {
    let op = fetch_u8(g);

    let var_id = fetch_index8(g);
    let var = g.vm.regs[var_id];

    let arg = if (op & 0x80) != 0 {
        g.vm.regs[fetch_index8(g)]
    } else if (op & 0x40) != 0 {
        fetch_i16(g)
    } else {
        i16::from(fetch_u8(g))
    };

    let new_pc = fetch_u16(g);

    log::trace!(
        "b{} 0x{:04X}, @x{:02X}",
        match op & 7 {
            0 => "eq",
            1 => "ne",
            2 => "gt",
            3 => "ge",
            4 => "lt",
            5 => "le",
            _ => unreachable!(),
        },
        new_pc,
        var_id
    );

    let mut test = match op & 7 {
        0 => var == arg,
        1 => var != arg,
        2 => var > arg,
        3 => var >= arg,
        4 => var < arg,
        5 => var <= arg,
        _ => panic!("invalid condition in jump"),
    };

    if var_id == 0x29 && (op & 0x80) != 0 && g.current_part == 16000 && g.bypass_protection {
        log::info!("bypassing protection");
        test = true;
        // 4 symbols
        g.vm.regs[0x29] = g.vm.regs[0x1E];
        g.vm.regs[0x2A] = g.vm.regs[0x1F];
        g.vm.regs[0x2B] = g.vm.regs[0x20];
        g.vm.regs[0x2C] = g.vm.regs[0x21];
        // counters
        g.vm.regs[0x32] = 6;
        g.vm.regs[0x64] = 20;
    }

    if test {
        g.vm.pc = new_pc;

        if var_id == reg_id::SCREEN_NUM && g.screen_num != Some(var) {
            g.screen_num = Some(var);
            fixup_pal_after_change_screen(g, var);
        }
    }
}

fn op_install_task(g: &mut Game) {
    let id = check_task_id(fetch_u8(g));
    let pc = fetch_u16(g);
    log::trace!("task %{} 0x{:04X}", id, pc);
    g.vm.pending_tasks[id].pc = pc;
}

fn op_remove_task(g: &mut Game) {
    log::trace!("halt");
    g.vm.pc = HALT_PC;
    g.vm.needs_yield = true;
}

fn op_yield_task(g: &mut Game) {
    log::trace!("yield");
    g.vm.needs_yield = true;
}

fn op_change_tasks(g: &mut Game) {
    let begin = check_task_id(fetch_u8(g));
    let end = check_task_id(fetch_u8(g) & 0x3F);
    let action = fetch_u8(g);

    if begin > end {
        log::error!(
            "invalid task range in vec instruction %{}..=%{}",
            begin,
            end
        );
        return;
    }

    log::trace!("xtask %{}..=%{}, {}", begin, end, action);

    for task in &mut g.vm.pending_tasks[begin..=end] {
        if action == 2 {
            task.pc = PRE_HALT_PC;
        } else {
            task.frozen = action != 0;
        }
    }
}

fn check_task_id(id: impl Into<usize> + Copy) -> usize {
    assert!(id.into() < TASK_COUNT, "invalid task ID");
    id.into()
}

pub fn stage_tasks(g: &mut Game) {
    if let Some(part) = g.next_part.take() {
        restart_at(g, part, -1);
    }

    let vm = &mut g.vm;
    for (task, pending_task) in vm.tasks.iter_mut().zip(vm.pending_tasks.iter_mut()) {
        task.frozen = pending_task.frozen;

        // Pending task might have one of following values for program counter:
        //
        // * 0xFFFF - no change
        // * 0xFFFE - halt the task
        // * 0x???? - start task at given address

        if pending_task.pc != HALT_PC {
            task.pc = if pending_task.pc == PRE_HALT_PC {
                HALT_PC
            } else {
                pending_task.pc
            };
            pending_task.pc = HALT_PC;
        }
    }
}

pub fn restart_at(g: &mut Game, part: u16, pos: i16) {
    sfx::stop_sound_and_music(g);

    g.vm.regs[0xE4] = 20;
    if part == 16000 {
        g.vm.regs[0x54] = 0x81;
    }

    mem::setup_part(g, part);

    g.vm.tasks = [Task::default(); TASK_COUNT];
    g.vm.pending_tasks = [Task::default(); TASK_COUNT];

    g.vm.tasks[0].pc = 0;
    g.screen_num = None;

    if pos >= 0 {
        g.vm.regs[0] = pos;
    }

    if g.video.needs_pal_fixup() && part == 16009 {
        video::load_pal_mem(g, 5);
    }

    g.vm.last_swap_time = Instant::now();
}

pub fn run_tasks(g: &mut Game) {
    for id in 0..TASK_COUNT {
        if g.vm.tasks[id].pc == HALT_PC || g.vm.tasks[id].frozen {
            continue;
        }

        g.vm.pc = g.vm.tasks[id].pc;
        g.vm.sp = 0;
        g.vm.needs_yield = false;
        execute_task(g);
        g.vm.tasks[id].pc = g.vm.pc;
    }
}

fn execute_task(g: &mut Game) {
    while !g.vm.needs_yield {
        let opcode = fetch_u8(g);
        if (opcode & 0xC0) != 0 {
            op_draw_shape(g, opcode);
        } else {
            match opcode {
                0x00 => op_mov_const(g),
                0x01 => op_mov(g),
                0x02 => op_add(g),
                0x03 => op_add_const(g),
                0x04 => op_call(g),
                0x05 => op_ret(g),
                0x06 => op_yield_task(g),
                0x07 => op_jmp(g),
                0x08 => op_install_task(g),
                0x09 => op_jmp_if_var(g),
                0x0A => op_cond_jmp(g),
                0x0B => op_change_pal(g),
                0x0C => op_change_tasks(g),
                0x0D => op_select_page(g),
                0x0E => op_fill_page(g),
                0x0F => op_copy_page(g),
                0x10 => op_update_display(g),
                0x11 => op_remove_task(g),
                0x12 => op_draw_string(g),
                0x13 => op_sub(g),
                0x14 => op_and_const(g),
                0x15 => op_or_const(g),
                0x16 => op_shl_const(g),
                0x17 => op_shr_const(g),
                0x18 => op_play_sound(g),
                0x19 => op_update_resources(g),
                0x1A => op_play_music(g),
                _ => panic!("invalid opcode 0x{:02X}", opcode),
            }
        }
    }
}

fn op_select_page(g: &mut Game) {
    let n = fetch_u8(g);
    log::trace!("fb_sel {}", n);
    video::select_page(&mut g.video, n);
}

fn op_fill_page(g: &mut Game) {
    let n = fetch_u8(g);
    let color = fetch_u8(g);
    log::trace!("fb_fill {}, {}", n, color);
    video::fill_page(&mut g.video, n, color);
}

fn op_copy_page(g: &mut Game) {
    let src = fetch_u8(g);
    let dst = fetch_u8(g);
    log::trace!("fb_copy {}, {}", src, dst);
    video::copy_page(&mut g.video, src, dst, g.vm.regs[reg_id::SCROLL_Y]);
}

#[allow(clippy::collapsible_if)]
fn op_draw_shape(g: &mut Game, opcode: u8) {
    if (opcode & 0x80) != 0 {
        let offset = ((u16::from(opcode) << 8) | u16::from(fetch_u8(g))) << 1;

        let mut x = i16::from(fetch_u8(g));
        let mut y = i16::from(fetch_u8(g));

        let h = y - 199;
        if h > 0 {
            y = 199;
            x += h;
        }

        g.video.set_dc(offset, false);
        video::draw_shape(g, x, y, 0x40, 0xFF);
    } else {
        let offset = fetch_u16(g) << 1;
        let x = fetch_u8(g);
        let x = if (opcode & 0x20) == 0 {
            if (opcode & 0x10) == 0 {
                (i16::from(x) << 8) | i16::from(fetch_u8(g))
            } else {
                g.vm.regs[usize::from(x)]
            }
        } else {
            i16::from(x) | (i16::from(opcode & 0x10) << 4)
        };

        let y = fetch_u8(g);
        let y = if (opcode & 0x08) == 0 {
            if (opcode & 0x04) == 0 {
                (i16::from(y) << 8) | i16::from(fetch_u8(g))
            } else {
                g.vm.regs[usize::from(y)]
            }
        } else {
            i16::from(y)
        };

        let mut use_seg2 = false;

        let zoom = fetch_u8(g);
        let zoom = if (opcode & 0x02) == 0 {
            if (opcode & 0x01) == 0 {
                g.vm.pc -= 1;
                0x40
            } else {
                g.vm.regs[usize::from(zoom)] as u16
            }
        } else if (opcode & 0x01) != 0 {
            use_seg2 = true;
            g.vm.pc -= 1;
            0x40
        } else {
            u16::from(zoom)
        };

        g.video.set_dc(offset, use_seg2);
        video::draw_shape(g, x, y, zoom, 0xFF);
    }
}

fn op_draw_string(g: &mut Game) {
    let str_id = fetch_u16(g);
    let xi = u16::from(fetch_u8(g));
    let ypos = u16::from(fetch_u8(g));
    let color = fetch_u8(g);
    log::trace!("gstr {}, {}, {}, {}", str_id, xi, ypos, color);
    video::draw_string(&mut g.video, xi, ypos, str_id, color);
}

fn op_change_pal(g: &mut Game) {
    let num = fetch_u8(g);
    let _dummy = fetch_u8(g);

    log::trace!("gpal {}, {}", num, _dummy);

    let skip_change =
        g.video.needs_pal_fixup() && g.current_part == 16001 && (num == 10 || num == 16);

    if !skip_change {
        g.next_pal = Some(num);
    }
}

fn op_play_sound(g: &mut Game) {
    let resource = fetch_u16(g);
    let freq = fetch_u8(g);
    let volume = fetch_u8(g);
    let channel = fetch_u8(g);

    log::trace!("snd {}, {}, {}, {}", resource, freq, volume, channel);

    play_sound_shim(g, resource, freq, volume, channel);
}

fn play_sound_shim(g: &mut Game, resource: u16, freq: u8, volume: u8, channel: u8) {
    if volume == 0 {
        sfx::stop_sound(g, channel);
    } else {
        let volume = std::cmp::min(volume, 0x3F);
        if let Some(address) = mem::address_of_entry(&g.mem, resource) {
            let freq = crate::data::FREQUENCY_TABLE[usize::from(freq)];
            sfx::play_sound(g, channel & 3, address, freq, volume);
        }
    }
}

fn op_play_music(g: &mut Game) {
    let resource = fetch_u16(g);
    let delay = fetch_u16(g);
    let pos = fetch_u8(g);

    log::trace!("music {}, {}, {}", resource, delay, pos);

    if resource != 0 {
        sfx::seek(g, resource, delay, pos);
    } else {
        g.music.set_delay(delay);
    }
}

fn op_update_resources(g: &mut Game) {
    let num = fetch_u16(g);
    log::trace!("res {}", num);
    if num == 0 {
        sfx::stop_sound_and_music(g);
        mem::invalidate_res(&mut g.mem);
        g.video.invalidate_pal_num();
    } else if num >= 16000 {
        g.next_part = Some(num);
    } else {
        mem::load_entry(g, num);
    }
}

fn op_update_display(g: &mut Game) {
    let page = fetch_u8(g);
    log::trace!("swap {}", page);

    let fb = video::swap_pages(&mut g.video, page);

    if let Some(num) = g.next_pal.take() {
        video::load_pal_mem(g, num);
    }

    crate::host::display_surface(g, fb);

    const HZ: i32 = 50;
    let mut delay = g.vm.last_swap_time.elapsed().as_millis() as i32;
    for _ in 0..g.vm.regs[reg_id::PAUSE_SLICES] {
        crate::host::produce_music(g);
        delay -= 1000 / HZ;
        if delay < 0 {
            std::thread::sleep(Duration::from_millis(-delay as u64));
            delay = 0;
        }
    }

    g.vm.last_swap_time = Instant::now();
    g.vm.regs[0xF7] = 0;
}

fn fixup_pal_after_change_screen(g: &mut Game, screen: i16) {
    if let Some(pal) = match (g.current_part, screen) {
        (16004, 0x47) => Some(8),
        (16006, 0x4A) => Some(1),
        _ => None,
    } {
        video::load_pal_mem(g, pal);
    }
}

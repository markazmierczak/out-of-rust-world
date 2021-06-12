use crate::{mem, Game};
use byteorder::{ByteOrder, BE};

pub const HOST_RATE: u16 = 44100;
pub const GAME_RATE: u16 = 11025;

#[derive(Default)]
pub struct Player {
    delay: u16,
    samples_left: u16,
    channels: [Channel; 4],
    track: Track,
}

#[derive(Default)]
struct Channel {
    sample_address: usize,
    sample_len: u16,
    sample_loop_pos: u16,
    sample_loop_len: u16,
    volume: u16,
    pos: Frac,
}

#[derive(Default)]
struct Track {
    address: usize,
    cur_pos: u16,
    cur_order: u8,
    #[allow(dead_code)]
    num_order: u16,
    order_table: TrackOrderTable,
    samples: [Instrument; 15],
}

struct TrackOrderTable([u8; 0x80]);

impl Default for TrackOrderTable {
    fn default() -> Self {
        Self([0; 0x80])
    }
}

#[derive(Default, Clone, Copy)]
struct Instrument {
    address: usize,
    volume: u16,
}

pub fn seek(g: &mut Game, res_num: u16, delay: u16, cur_order: u8) {
    let address =
        match mem::address_of_entry_with_kind(&g.mem, res_num, crate::mem::entry_kind::MUSIC) {
            Some(a) => a,
            None => {
                log::warn!("unable to load music from {} resource", res_num);
                return;
            }
        };

    let data = &g.mem.data[address..];
    let num_order = BE::read_u16(&data[address + 0x3E..]);

    let mut order_table = TrackOrderTable::default();
    order_table.0[..0x80].clone_from_slice(&data[64..(0x80 + 64)]);

    g.music.delay = cvt_delay(if delay == 0 {
        BE::read_u16(data)
    } else {
        delay
    });

    let samples = prepare_instruments(g, &data[2..]);

    let address = address + 0xC0;
    g.music.track = Track {
        address,
        cur_pos: 0,
        cur_order,
        num_order,
        order_table,
        samples,
    };
    g.music.samples_left = 0;
    g.music.channels = Default::default();
}

fn prepare_instruments(g: &Game, data: &[u8]) -> [Instrument; 15] {
    let mut samples = [Instrument::default(); 15];
    for i in 0..15 {
        let ins = &mut samples[i];
        let res_num = BE::read_u16(&data[i * 4..]);
        if res_num != 0 {
            ins.volume = BE::read_u16(&data[i * 4 + 2..]);
            ins.address =
                mem::address_of_entry_with_kind(&g.mem, res_num, crate::mem::entry_kind::SOUND)
                    .expect("error loading instrument");
        }
    }
    samples
}

fn cvt_delay(delay: u16) -> u16 {
    (u32::from(delay) * 60 / 7050) as u16
}

pub fn mix_samples(g: &mut Game, mut out: &mut [i16]) {
    assert!(g.music.delay != 0);

    let mut len = (out.len() / 2) as u16;
    let samples_per_tick = HOST_RATE / (1000 / g.music.delay);
    while len != 0 {
        if g.music.samples_left == 0 {
            process_events(g);
            g.music.samples_left = samples_per_tick;
        }

        let count = std::cmp::min(g.music.samples_left, len);
        g.music.samples_left -= count;
        len -= count;

        for i in 0..count {
            let sample = mix_channel(g, 0, 0);
            let sample = mix_channel(g, 3, sample);
            out[usize::from(i * 2)] = i16::from(sample) * 256;

            let sample = mix_channel(g, 1, 0);
            let sample = mix_channel(g, 2, sample);
            out[usize::from(i * 2 + 1)] = i16::from(sample) * 256;
        }

        out = &mut out[usize::from(count * 2)..];
    }

    nr(out)
}

fn nr(out: &mut [i16]) {
    let mut prev_l = 0;
    let mut prev_r = 0;

    for pair in out.chunks_exact_mut(2) {
        let l = pair[0] >> 1;
        pair[0] = l.wrapping_add(prev_l);
        prev_l = l;

        let r = pair[1] >> 1;
        pair[0] = r.wrapping_add(prev_r);
        prev_r = r;
    }
}

#[allow(clippy::collapsible_if)]
fn mix_channel(g: &mut Game, ch: usize, in_sample: i8) -> i8 {
    let ch = &mut g.music.channels[ch];
    if ch.sample_len == 0 {
        return in_sample;
    }

    let pos1 = ch.pos.int();
    ch.pos.inc();
    let mut pos2 = pos1 + 1;

    if ch.sample_loop_len != 0 {
        if pos2 == u32::from(ch.sample_loop_pos) + u32::from(ch.sample_loop_len) {
            pos2 = u32::from(ch.sample_loop_pos);
            ch.pos.set_int(pos2);
        }
    } else if pos2 == u32::from(ch.sample_len) {
        ch.sample_len = 0;
        return in_sample;
    }

    let data = &g.mem.data[ch.sample_address..];
    let sample = ch
        .pos
        .interpolate(data[pos1 as usize] as i8, data[pos2 as usize] as i8);
    let sample = i16::from(in_sample) + sample * (ch.volume as i16) / 64;
    std::cmp::max(-128, std::cmp::min(sample, 127)) as i8
}

fn process_events(g: &mut Game) {
    let track = &g.music.track;
    let order = track.order_table.0[usize::from(track.cur_order)];
    let address = track.address + usize::from(track.cur_pos) + usize::from(order) * 1024;
    for ch in 0..4 {
        handle_pattern(g, ch, address + ch * 4);
    }

    let track = &mut g.music.track;
    track.cur_pos += 4 * 4;
    if track.cur_pos >= 1024 {
        track.cur_pos = 0;
        track.cur_order += 1;
    }
}

#[derive(Default)]
struct Pattern {
    sample_address: usize,
    sample_start: u16,
    sample_len: u16,
    sample_volume: u16,
    loop_pos: u16,
    loop_len: u16,
}

fn handle_pattern(g: &mut Game, channel: usize, address: usize) {
    let data = &g.mem.data[address..];
    let note1 = BE::read_u16(data);
    let note2 = BE::read_u16(&data[2..]);

    if note1 == 0xFFFD {
        g.vm.sync_music(note2);
        return;
    }

    let mut pattern = Pattern::default();
    let sample = note2 >> 12;
    if sample != 0 {
        let Instrument { address, volume } = g.music.track.samples[usize::from(sample - 1)];
        if address != 0 {
            let data = &g.mem.data[address..];
            pattern.sample_start = 8;
            pattern.sample_address = address;
            pattern.sample_len = BE::read_u16(data) * 2;
            let loop_len = BE::read_u16(&data[2..]) * 2;
            let (loop_pos, loop_len) = if loop_len != 0 {
                (pattern.sample_len, loop_len)
            } else {
                (0, 0)
            };
            pattern.loop_pos = loop_pos;
            pattern.loop_len = loop_len;

            const VOLUME_UP_EFFECT: u16 = 5;
            const VOLUME_DOWN_EFFECT: u16 = 6;

            let effect = (note2 >> 8) & 0xF;
            let amount = note2 & 0xFF;
            let volume = if effect == VOLUME_UP_EFFECT {
                std::cmp::min(volume + amount, 0x3F)
            } else if effect == VOLUME_DOWN_EFFECT {
                volume.saturating_sub(amount)
            } else {
                volume
            };
            pattern.sample_volume = volume;
            g.music.channels[channel].volume = volume;
        }
    }

    if note1 == 0xFFFE {
        g.music.channels[channel].sample_len = 0;
    } else if note1 != 0 && pattern.sample_address != 0 {
        assert!((0x37..0x1000).contains(&note1));
        // Convert Amiga period value to Hz.
        let freq = (7_159_092 / (u32::from(note1) * 2)) as u16;
        let ch = &mut g.music.channels[channel];
        ch.sample_address = pattern.sample_address + usize::from(pattern.sample_start);
        ch.sample_len = pattern.sample_len;
        ch.sample_loop_pos = pattern.loop_pos;
        ch.sample_loop_len = pattern.loop_len;
        ch.volume = pattern.sample_volume;
        ch.pos = Frac::new(freq, HOST_RATE);
    }
}

impl Player {
    pub fn set_delay(&mut self, delay: u16) {
        self.delay = cvt_delay(delay);
    }

    pub fn is_end_of_track(&self) -> bool {
        self.delay == 0
    }
}

pub fn play_sound(g: &mut Game, channel: u8, address: usize, freq: u16, volume: u8) {
    let data = &g.mem.data[address..];
    let len = BE::read_u16(data) * 2;
    let loop_len = BE::read_u16(&data[2..]) * 2;

    let (len, loops) = if loop_len != 0 {
        (loop_len, -1)
    } else {
        (len, 0)
    };

    crate::host::play_sound(
        &mut g.host,
        channel,
        freq,
        volume,
        &data[8..],
        len.into(),
        loops,
    );
}

pub fn stop_sound(g: &mut Game, channel: u8) {
    crate::host::stop_sound(&mut g.host, channel);
}

pub fn stop_sound_and_music(g: &mut Game) {
    for channel in 0..4 {
        stop_sound(g, channel);
    }
    g.music.set_delay(0);
}

#[derive(Default, Clone, Copy)]
pub struct Frac {
    inc: u32,
    offset: u64,
}

impl Frac {
    const BITS: u32 = 16;

    pub fn new(n: impl Into<u32>, d: impl Into<u32>) -> Self {
        Self {
            inc: (n.into() << Self::BITS) / d.into(),
            offset: 0,
        }
    }

    pub fn int(self) -> u32 {
        (self.offset >> Frac::BITS) as u32
    }

    pub fn set_int(&mut self, int: u32) {
        self.offset = u64::from(int) << Frac::BITS;
    }

    pub fn frac(self) -> u16 {
        self.offset as u16
    }

    pub fn inc(&mut self) {
        self.offset += u64::from(self.inc);
    }

    fn interpolate(self, sample1: i8, sample2: i8) -> i16 {
        let fp = self.frac();
        ((i32::from(sample1) * i32::from(0xFFFF - fp) + i32::from(sample2) * i32::from(fp))
            >> Frac::BITS) as i16
    }
}

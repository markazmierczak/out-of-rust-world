use super::{video, Game};
use byteorder::{ByteOrder, BE};
use std::io::{Read, Seek};

const STATUS_EMPTY: u8 = 0;
const STATUS_READY: u8 = 1;
const STATUS_PENDING: u8 = 2;

pub struct Memory {
    list: Vec<Entry>,
    pub data: Vec<u8>,

    data_bak: usize,
    data_cur: usize,

    seg_code: usize,
    seg_video_pal: usize,
    seg_video1: usize,
    seg_video2: usize,
}

#[derive(Debug)]
struct Entry {
    status: u8,
    kind: u8,
    address: usize,
    rank_num: u8,
    bank_num: u8,
    bank_pos: u32,
    packed_size: usize,
    unpacked_size: usize,
}

mod entry_kind {
    pub const BITMAP: u8 = 2;
}

const DATA_SIZE: usize = 1 * 1024 * 1024;
const DATA_BMP_OFFSET: usize = DATA_SIZE - 0x800 * 16;

impl Memory {
    pub fn new() -> Self {
        let list = read_entries();
        Self {
            list,
            data: vec![0; DATA_SIZE],
            data_bak: 0,
            data_cur: 0,

            seg_code: 0,
            seg_video_pal: 0,
            seg_video1: 0,
            seg_video2: 0,
        }
    }

    pub fn seg_code(&self) -> usize {
        self.seg_code
    }

    pub fn seg_video_pal(&self) -> usize {
        self.seg_video_pal
    }

    pub fn seg_video1(&self) -> usize {
        self.seg_video1
    }

    pub fn seg_video2(&self) -> usize {
        self.seg_video2
    }
}

fn read_entries() -> Vec<Entry> {
    let mut f = std::fs::File::open("memlist.bin").unwrap();
    let mut entries = Vec::new();
    let mut buf = [0; 20];
    loop {
        f.read_exact(&mut buf).unwrap();
        let status = buf[0];
        let kind = buf[1];
        let address = BE::read_u32(&buf[2..]) as usize;
        let rank_num = buf[6];
        let bank_num = buf[7];
        let bank_pos = BE::read_u32(&buf[8..]);
        let packed_size = BE::read_u32(&buf[12..]) as usize;
        let unpacked_size = BE::read_u32(&buf[16..]) as usize;

        if status == 0xFF {
            break;
        }

        entries.push(Entry {
            status,
            kind,
            address,
            rank_num,
            bank_num,
            bank_pos,
            packed_size,
            unpacked_size,
        })
    }
    entries
}

fn read_bank(entry: &Entry, dst: &mut [u8]) {
    let path = format!("bank{:02x}", entry.bank_num);
    log::debug!("reading entry {:?} from {}", entry, path);
    let mut f = std::fs::File::open(&path).unwrap();
    f.seek(std::io::SeekFrom::Start(entry.bank_pos.into()))
        .unwrap();
    f.read_exact(&mut dst[0..entry.packed_size]).unwrap();

    if entry.packed_size != entry.unpacked_size {
        crate::bytekiller::unpack(&mut dst[0..entry.unpacked_size], entry.packed_size);
    }
}

pub fn setup_part(g: &mut Game, part_id: u16) {
    let m = &mut g.mem;
    if g.current_part != part_id {
        assert!(
            16000 <= part_id && part_id <= 16009,
            "invalid part {}",
            part_id
        );

        let part_index = usize::from(part_id - 16000);
        let (ipal, icod, ivd1, ivd2) = MEM_LIST_PARTS[part_index];

        // invalidate all entries
        for entry in m.list.iter_mut() {
            entry.status = STATUS_EMPTY;
        }
        m.data_cur = 0;

        for i in [ipal, icod, ivd1, ivd2].iter().copied().filter(|x| *x != 0) {
            m.list[usize::from(i)].status = STATUS_PENDING;
        }

        load_entries(g);

        let m = &mut g.mem;
        m.seg_video_pal = address_of_entry(m, ipal);
        m.seg_code = address_of_entry(m, icod);
        m.seg_video1 = address_of_entry(m, ivd1);
        if ivd2 != 0 {
            m.seg_video2 = address_of_entry(m, ivd2);
        }

        g.current_part = part_id;
    }

    g.mem.data_bak = g.mem.data_cur;
}

fn address_of_entry(m: &Memory, index: u8) -> usize {
    m.list[usize::from(index)].address
}

pub fn invalidate_res(m: &mut Memory) {
    m.data_cur = m.data_bak;

    for entry in m.list.iter_mut().filter(|e| e.kind <= 2 || e.kind > 6) {
        entry.status = STATUS_EMPTY;
    }
}

pub fn load_entry(g: &mut Game, num: u16) {
    let entry = &mut g.mem.list[usize::from(num)];
    if entry.status == STATUS_EMPTY {
        entry.status = STATUS_PENDING;
        load_entries(g);
    }
}

fn load_entries(g: &mut Game) {
    let m = &mut g.mem;
    while let Some(entry) = m
        .list
        .iter_mut()
        .filter(|e| e.status == STATUS_PENDING)
        .max_by_key(|e| e.rank_num)
    {
        let address = if entry.kind == entry_kind::BITMAP {
            DATA_BMP_OFFSET
        } else {
            assert!(entry.unpacked_size <= DATA_BMP_OFFSET - m.data_cur);
            m.data_cur
        };

        if entry.bank_num == 0 {
            log::warn!("invalid load from bank 0");
            entry.status = STATUS_EMPTY;
        } else {
            read_bank(entry, &mut m.data[address..]);
            if entry.kind == entry_kind::BITMAP {
                video::copy_bitmap(&mut g.video, &m.data[address..]);
                entry.status = STATUS_EMPTY;
            } else {
                entry.address = address;
                entry.status = STATUS_READY;
                m.data_cur += entry.unpacked_size;
            }
        }
    }
}

const MEM_LIST_PARTS: [(u8, u8, u8, u8); 10] = [
    (0x14, 0x15, 0x16, 0x00), // 16000 - protection screens
    (0x17, 0x18, 0x19, 0x00), // 16001 - introduction
    (0x1A, 0x1B, 0x1C, 0x11), // 16002 - water
    (0x1D, 0x1E, 0x1F, 0x11), // 16003 - jail
    (0x20, 0x21, 0x22, 0x11), // 16004 - 'cite'
    (0x23, 0x24, 0x25, 0x00), // 16005 - 'arene'
    (0x26, 0x27, 0x28, 0x11), // 16006 - 'luxe'
    (0x29, 0x2A, 0x2B, 0x11), // 16007 - 'final'
    (0x7D, 0x7E, 0x7F, 0x00), // 16008 - password screen
    (0x7D, 0x7E, 0x7F, 0x00), // 16009 - password screen
];

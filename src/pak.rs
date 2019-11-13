use byteorder::{ByteOrder, LittleEndian};
use std::cell::RefCell;
use std::io::{self, Read, Seek};

const MAX_NAME_LEN: usize = 32;

pub struct Package {
    file: RefCell<std::fs::File>,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone)]
pub struct Entry {
    name: [u8; MAX_NAME_LEN],
    offset: u32,
    size: u32,
}

impl Package {
    // TODO: open

    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name_equals(name))
    }

    pub fn load(&self, entry: &Entry) -> io::Result<Vec<u8>> {
        let mut f = self.file.borrow_mut();
        f.seek(io::SeekFrom::Start(entry.offset.into()))?;
        let length = entry.size as usize;
        let mut data = vec![0; length];
        f.read_exact(&mut data)?;

        if data.starts_with(b"TooDC") {
            decode_toodc(&mut data[6..]);
            data.drain(0..10);
        }

        Ok(data)
    }
}

impl Entry {
    pub fn name(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.raw_name())
    }

    pub fn name_equals(&self, other: &str) -> bool {
        self.raw_name()
            .iter()
            .map(u8::to_ascii_lowercase)
            .eq(other.as_bytes().iter().map(u8::to_ascii_lowercase))
    }

    // Return name slice to first null-terminator (terminator is excluded).
    pub fn raw_name(&self) -> &[u8] {
        let null_pos = self
            .name
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(MAX_NAME_LEN);
        &self.name[0..null_pos]
    }
}

const CHECKSUM: u32 = 0x2020_2020;

fn decode_toodc(data: &mut [u8]) {
    assert!(
        data.len().trailing_zeros() >= 2,
        "invalid length for encoded TooDC data"
    );

    const XOR_KEY2: u32 = 0x2268_3297;

    let mut key = XOR_KEY2;
    let mut acc = 0;
    for q in data.chunks_exact_mut(4) {
        let word = LittleEndian::read_u32(q) ^ key;
        let r = (u32::from(q[2]) + u32::from(q[1]) + u32::from(q[0])) ^ u32::from(q[3]);
        key += r + acc;
        acc += 0x4D;
        LittleEndian::write_u32(q, word);
    }
}

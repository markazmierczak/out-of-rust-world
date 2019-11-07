use byteorder::{ByteOrder, BE};

struct Ctx<'a> {
    buf: &'a mut [u8],
    dst_pos: usize,
    src_pos: usize,
    len: usize,
    crc: u32,
    bits: u32,
}

impl<'a> Ctx<'a> {
    fn adjust_len(&mut self, count: usize) -> usize {
        if self.len >= count {
            self.len -= count;
            count
        } else {
            count - std::mem::replace(&mut self.len, 0)
        }
    }
}

pub fn unpack(buf: &mut [u8], packed_len: usize) {
    let mut src_pos = packed_len - 4;

    let len = BE::read_u32(&buf[src_pos..]) as usize;
    src_pos -= 4;

    assert!(len <= buf.len(), "output buffer too small");
    let dst_pos = len - 1;

    let mut crc = BE::read_u32(&buf[src_pos..]);
    src_pos -= 4;

    let bits = BE::read_u32(&buf[src_pos..]);
    src_pos -= 4;

    crc ^= bits;

    let mut ctx = Ctx {
        buf,
        dst_pos,
        src_pos,
        len,
        crc,
        bits,
    };

    while ctx.len > 0 {
        if !next_bit(&mut ctx) {
            if !next_bit(&mut ctx) {
                getd3chr(&mut ctx, 3, 0);
            } else {
                copyd3bytes(&mut ctx, 8, 2);
            }
        } else {
            let code = rdd1bits(&mut ctx, 2);
            match code {
                0 => copyd3bytes(&mut ctx, 9, 3),
                1 => copyd3bytes(&mut ctx, 10, 4),
                2 => {
                    let len = rdd1bits(&mut ctx, 8) + 1;
                    copyd3bytes(&mut ctx, 12, len as usize);
                }
                3 => getd3chr(&mut ctx, 8, 8),
                _ => unreachable!(),
            }
        }
    }

    assert!(ctx.len == 0 && ctx.crc == 0, "bytekiller failure");
}

fn rdd1bits(ctx: &mut Ctx, count: usize) -> i32 {
    let mut output = 0;
    for _ in 0..count {
        output = (output << 1) | i32::from(next_bit(ctx));
    }
    output
}

fn getd3chr(ctx: &mut Ctx, bits_count: usize, input_len: usize) {
    let count = (rdd1bits(ctx, bits_count) as usize) + input_len + 1;
    let count = ctx.adjust_len(count);

    for i in 0..count {
        ctx.buf[ctx.dst_pos - i] = rdd1bits(ctx, 8) as u8;
    }

    ctx.dst_pos = ctx.dst_pos.wrapping_sub(count);
}

fn copyd3bytes(ctx: &mut Ctx, bits_count: usize, count: usize) {
    let count = ctx.adjust_len(count);
    let offset = rdd1bits(ctx, bits_count);

    for i in 0..count {
        let output_pos = ctx.dst_pos - i;
        let input_pos = if offset >= 0 {
            output_pos + (offset as usize)
        } else {
            output_pos - (-offset as usize)
        };
        ctx.buf[output_pos] = ctx.buf[input_pos];
    }

    ctx.dst_pos = ctx.dst_pos.wrapping_sub(count);
}

fn next_bit(ctx: &mut Ctx) -> bool {
    let mut carry = (ctx.bits & 1) != 0;
    ctx.bits >>= 1;
    if ctx.bits == 0 {
        ctx.bits = BE::read_u32(&ctx.buf[ctx.src_pos..]);
        ctx.src_pos = ctx.src_pos.wrapping_sub(4);
        ctx.crc ^= ctx.bits;
        carry = (ctx.bits & 1) != 0;
        ctx.bits = (1 << 31) | (ctx.bits >> 1);
    }
    carry
}

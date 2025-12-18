use bitflags::bitflags;

pub const MAGIC: u32 = 0x5A525444; // 'DTRZ' in Little Endian
pub const CHUNK_LIST_TERMINATOR: u16 = 0xFFFF;

// [Refactor] Use bitflags! macro for type-safe flag handling
bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ChunkFlags: u16 {
        const COMBUF       = 0x1;
        const DZ_RANGE     = 0x4;
        const ZLIB         = 0x8;
        const BZIP         = 0x10;
        const MP3          = 0x20;
        const JPEG         = 0x40;
        const ZERO         = 0x80;
        const COPYCOMP     = 0x100;
        const LZMA         = 0x200;
        const RANDOMACCESS = 0x400;
    }
}
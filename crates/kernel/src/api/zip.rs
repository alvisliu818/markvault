use chrono::{Datelike, Timelike};

/// Minimal STORE-only ZIP writer (no compression).
/// Markdown exports are small; avoiding a compression crate keeps the
/// dependency tree lean. Filenames are written as UTF-8 with the
/// general-purpose bit 11 flag set so Chinese paths survive on Windows
/// Explorer / 7-Zip / unzip.

static CRC_TABLE: [u32; 256] = make_crc_table();

const fn make_crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn dos_datetime() -> (u16, u16) {
    let now = chrono::Local::now();
    let y = now.year().clamp(1980, 2107) as u16;
    let date = ((y - 1980) << 9) | ((now.month() as u16) << 5) | now.day() as u16;
    let time = ((now.hour() as u16) << 11)
        | ((now.minute() as u16) << 5)
        | (now.second() as u16 / 2);
    (time, date)
}

pub struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl Default for ZipWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ZipWriter {
    pub fn new() -> Self {
        ZipWriter { out: Vec::new(), central: Vec::new(), count: 0 }
    }

    /// `name` uses forward slashes; caller strips leading `/`.
    pub fn add(&mut self, name: &str, data: &[u8]) {
        let crc = crc32(data);
        let (dtime, ddate) = dos_datetime();
        let name_b = name.as_bytes();
        let off = self.out.len() as u32;

        // local file header
        self.out.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]);
        self.out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        self.out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 flag
        self.out.extend_from_slice(&0u16.to_le_bytes()); // method: store
        self.out.extend_from_slice(&dtime.to_le_bytes());
        self.out.extend_from_slice(&ddate.to_le_bytes());
        self.out.extend_from_slice(&crc.to_le_bytes());
        self.out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        self.out.extend_from_slice(name_b);
        self.out.extend_from_slice(data);

        // central directory entry
        self.central.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]);
        self.central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        self.central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        self.central.extend_from_slice(&0x0800u16.to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes()); // method
        self.central.extend_from_slice(&dtime.to_le_bytes());
        self.central.extend_from_slice(&ddate.to_le_bytes());
        self.central.extend_from_slice(&crc.to_le_bytes());
        self.central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.central.extend_from_slice(&(name_b.len() as u16).to_le_bytes());
        self.central.extend_from_slice(&0u16.to_le_bytes()); // extra
        self.central.extend_from_slice(&0u16.to_le_bytes()); // comment
        self.central.extend_from_slice(&0u16.to_le_bytes()); // disk start
        self.central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        self.central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        self.central.extend_from_slice(&off.to_le_bytes());
        self.central.extend_from_slice(name_b);
        self.count += 1;
    }

    pub fn finish(mut self) -> Vec<u8> {
        let cd_off = self.out.len() as u32;
        let cd_size = self.central.len() as u32;
        self.out.extend_from_slice(&self.central);
        // end of central directory
        self.out.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06]);
        self.out.extend_from_slice(&0u16.to_le_bytes()); // disk
        self.out.extend_from_slice(&0u16.to_le_bytes()); // cd disk
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&cd_size.to_le_bytes());
        self.out.extend_from_slice(&cd_off.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes()); // comment len
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walk the produced zip structure and verify EOCD/central/local headers.
    #[test]
    fn zip_structure_roundtrip() {
        let mut zw = ZipWriter::new();
        zw.add("笔记本A/首页.md", "# hello\n".as_bytes());
        zw.add("assets/pic.png", &[1, 2, 3, 4, 5]);
        let zip = zw.finish();

        // EOCD is the last 22 bytes (no comment)
        let eocd = &zip[zip.len() - 22..];
        assert_eq!(&eocd[..4], &[0x50, 0x4B, 0x05, 0x06]);
        let count = u16::from_le_bytes([eocd[10], eocd[11]]);
        assert_eq!(count, 2);
        let cd_size = u32::from_le_bytes([eocd[12], eocd[13], eocd[14], eocd[15]]);
        let cd_off = u32::from_le_bytes([eocd[16], eocd[17], eocd[18], eocd[19]]);
        let cd = &zip[cd_off as usize..(cd_off + cd_size) as usize];
        assert_eq!(&cd[..4], &[0x50, 0x4B, 0x01, 0x02]);

        // local header starts with signature, stored size matches
        assert_eq!(&zip[..4], &[0x50, 0x4B, 0x03, 0x04]);
        let size = u32::from_le_bytes([zip[22], zip[23], zip[24], zip[25]]);
        assert_eq!(size, "# hello\n".len() as u32);

        assert_eq!(crc32(b"hello"), 0x3610A686);
    }
}

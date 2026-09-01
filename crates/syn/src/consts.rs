/// True at a byte value that starts an identifier (a-z, A-Z, _, or >= 0x80).
pub(super) const IDENTIFIER_START_TABLE: [bool; 256] = {
    let mut table = [false; 256];
    let mut i = 0usize;
    while i < 256 {
        table[i] = matches!(i as u8, b'a'..=b'z' | b'A'..=b'Z' | b'_' | 0x80..=0xFF);
        i += 1;
    }

    table
};

/// True at a byte value that continues an identifier (a-z, A-Z, 0-9, _, or >= 0x80).
pub(super) const IDENTIFIER_PART_TABLE: [bool; 256] = {
    let mut table = [false; 256];
    let mut i = 0usize;
    while i < 256 {
        table[i] = matches!(i as u8, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | 0x80..=0xFF);
        i += 1;
    }

    table
};

/// True at a byte value that is ASCII whitespace (space, tab, `\n`, `\r`, `\x0B`, `\x0C`).
pub(super) const WHITESPACE_TABLE: [bool; 256] = {
    let mut table = [false; 256];
    table[b' ' as usize] = true;
    table[b'\t' as usize] = true;
    table[b'\n' as usize] = true;
    table[b'\r' as usize] = true;
    table[0x0C] = true;
    table[0x0B] = true;
    table
};

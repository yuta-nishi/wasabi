/// The characters a boot-protocol keyboard report carries (USB HID 1.11, B.1):
/// modifiers in the first byte, six usage codes from the third.
pub fn keys(report: &[u8]) -> [Option<char>; 6] {
    if report.len() < 8 {
        return [None; 6];
    }
    let shift = report[0] & 0b11 != 0;
    let mut keys = [None; 6];
    for (key, usage) in keys.iter_mut().zip(&report[2..8]) {
        *key = key_for(*usage, shift);
    }
    keys
}

/// The six usage codes a report carries, zero where no key is held.
pub fn usages(report: &[u8]) -> [u8; 6] {
    let mut usages = [0; 6];
    if report.len() >= 8 {
        usages.copy_from_slice(&report[2..8]);
    }
    usages
}

/// The characters of the usage codes that were not held in `previous`: a key the
/// keyboard keeps reporting while it is held is not typed twice.
pub fn newly_pressed(report: &[u8], previous: &[u8; 6]) -> [Option<char>; 6] {
    if report.len() < 8 {
        return [None; 6];
    }
    let shift = report[0] & 0b11 != 0;
    let mut keys = [None; 6];
    for (key, usage) in keys.iter_mut().zip(usages(report)) {
        if usage != 0 && !previous.contains(&usage) {
            *key = key_for(usage, shift);
        }
    }
    keys
}

/// The character one usage code means, with shift applied.
pub fn key_for(usage: u8, shift: bool) -> Option<char> {
    let character = match usage {
        0x04..=0x1d => char::from(b'a' + (usage - 0x04)),
        0x1e..=0x26 => char::from(b'1' + (usage - 0x1e)),
        0x27 => '0',
        0x28 => '\n',
        // Backspace.
        0x2a => '\u{8}',
        0x2c => ' ',
        _ => return None,
    };
    Some(if shift { shifted(character) } else { character })
}

fn shifted(character: char) -> char {
    match character {
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        other => other.to_ascii_uppercase(),
    }
}

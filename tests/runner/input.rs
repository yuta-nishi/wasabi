use wasabi::input::Line;

pub fn a_line_is_handed_over_when_enter_arrives() {
    let mut line = Line::new();
    for byte in b"hello" {
        assert!(line.push(*byte).is_none(), "the line is not finished yet");
    }
    assert_eq!(line.push(b'\r'), Some(&b"hello"[..]));
    // The buffer starts over, so the next Enter does not repeat the line.
    assert!(line.push(b'\r').is_none());
}

pub fn newline_ends_a_line_too() {
    let mut line = Line::new();
    for byte in b"abc" {
        line.push(*byte);
    }
    assert_eq!(line.push(b'\n'), Some(&b"abc"[..]));
}

pub fn backspace_takes_one_byte_off_the_end() {
    let mut line = Line::new();
    for byte in b"abx" {
        line.push(*byte);
    }
    line.push(0x7f);
    line.push(b'c');
    assert_eq!(line.push(b'\r'), Some(&b"abc"[..]));
}

pub fn backspace_on_an_empty_line_is_harmless() {
    let mut line = Line::new();
    line.push(0x08);
    line.push(0x7f);
    line.push(b'x');
    assert_eq!(line.push(b'\r'), Some(&b"x"[..]));
}

pub fn a_line_stops_growing_at_the_capacity() {
    let mut line = Line::new();
    for _ in 0..wasabi::input::LINE_CAPACITY + 10 {
        line.push(b'x');
    }
    assert_eq!(
        line.push(b'\r'),
        Some(&[b'x'; wasabi::input::LINE_CAPACITY][..])
    );
}

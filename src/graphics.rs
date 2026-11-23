use crate::result::Result;
use core::cmp::min;

pub trait FrameBuffer {
    fn bytes_per_pixel(&self) -> i64;
    fn pixels_per_line(&self) -> i64;
    fn width(&self) -> i64;
    fn height(&self) -> i64;
    fn buf_mut(&mut self) -> *mut u8;
    /// # Safety
    ///
    /// The returned pointer is valid as long as the given coordinates are valid,
    /// i.e. they pass the `is_in_*_range` tests.
    unsafe fn unchecked_pixel_at_mut(&mut self, x: i64, y: i64) -> *mut u32 {
        // SAFETY: the caller guarantees that (x, y) is in range.
        unsafe {
            self.buf_mut()
                .add(((y * self.pixels_per_line() + x) * self.bytes_per_pixel()) as usize)
                as *mut u32
        }
    }
    fn pixel_at_mut(&mut self, x: i64, y: i64) -> Option<&mut u32> {
        if self.is_in_x_range(x) && self.is_in_y_range(y) {
            // SAFETY: (x, y) was validated by the checks above.
            unsafe { Some(&mut *(self.unchecked_pixel_at_mut(x, y))) }
        } else {
            None
        }
    }
    fn is_in_x_range(&self, px: i64) -> bool {
        0 <= px && px < min(self.width(), self.pixels_per_line())
    }
    fn is_in_y_range(&self, py: i64) -> bool {
        0 <= py && py < self.height()
    }
}

/// # Safety
///
/// (x, y) must be a valid point in the buffer.
unsafe fn unchecked_draw_point<T: FrameBuffer>(buf: &mut T, color: u32, x: i64, y: i64) {
    // SAFETY: the caller guarantees that (x, y) is in range.
    unsafe {
        *buf.unchecked_pixel_at_mut(x, y) = color;
    }
}

fn draw_point<T: FrameBuffer>(buf: &mut T, color: u32, x: i64, y: i64) -> Result<()> {
    *(buf.pixel_at_mut(x, y).ok_or("Out of Range")?) = color;
    Ok(())
}

pub fn fill_rect<T: FrameBuffer>(
    buf: &mut T,
    color: u32,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
) -> Result<()> {
    if !buf.is_in_x_range(x)
        || !buf.is_in_y_range(y)
        || !buf.is_in_x_range(x + w - 1)
        || !buf.is_in_y_range(y + h - 1)
    {
        return Err("Out of Range");
    }
    for dy in 0..h {
        for dx in 0..w {
            // SAFETY: the rectangle bounds were checked above.
            unsafe {
                unchecked_draw_point(buf, color, x + dx, y + dy);
            }
        }
    }
    Ok(())
}

fn calc_slope_point(distance_a: i64, distance_b: i64, index: i64) -> Option<i64> {
    if distance_a < distance_b {
        None
    } else if distance_a == 0 {
        Some(0)
    } else if (0..=distance_a).contains(&index) {
        Some((2 * distance_b * index + distance_a) / distance_a / 2)
    } else {
        None
    }
}

fn draw_line<T: FrameBuffer>(
    buf: &mut T,
    color: u32,
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
) -> Result<()> {
    if !buf.is_in_x_range(x0)
        || !buf.is_in_x_range(x1)
        || !buf.is_in_y_range(y0)
        || !buf.is_in_y_range(y1)
    {
        return Err("Out of Range");
    }
    let dx = (x1 - x0).abs();
    let sx = (x1 - x0).signum();
    let dy = (y1 - y0).abs();
    let sy = (y1 - y0).signum();
    if dx >= dy {
        for (rx, ry) in
            (0..dx).flat_map(|rx| calc_slope_point(dx, dy, rx).map(|ry| (rx, ry)))
        {
            draw_point(buf, color, x0 + rx * sx, y0 + ry * sy)?;
        }
    } else {
        for (rx, ry) in
            (0..dy).flat_map(|ry| calc_slope_point(dy, dx, ry).map(|rx| (rx, ry)))
        {
            draw_point(buf, color, x0 + rx * sx, y0 + ry * sy)?;
        }
    }
    Ok(())
}

fn lookup_font(c: char) -> Option<[[char; 8]; 16]> {
    const FONT_SOURCE: &str = include_str!("./font.txt");
    if let Ok(character) = u8::try_from(c) {
        let mut fi = FONT_SOURCE.split('\n');
        while let Some(line) = fi.next() {
            if let Some(line) = line.strip_prefix("0x")
                && let Ok(idx) = u8::from_str_radix(line, 16)
            {
                if idx != character {
                    continue;
                }
                let mut font = [['*'; 8]; 16];
                for (y, line) in fi.clone().take(16).enumerate() {
                    for (x, character) in line.chars().enumerate() {
                        if let Some(pixel) = font[y].get_mut(x) {
                            *pixel = character;
                        }
                    }
                }
                return Some(font);
            }
        }
    }
    None
}

pub fn draw_font_fg<T: FrameBuffer>(buf: &mut T, x: i64, y: i64, color: u32, c: char) {
    if let Some(font) = lookup_font(c) {
        for (dy, row) in font.iter().enumerate() {
            for (dx, pixel) in row.iter().enumerate() {
                let color = match pixel {
                    '*' => color,
                    _ => continue,
                };
                let _ = draw_point(buf, color, x + dx as i64, y + dy as i64);
            }
        }
    }
}

pub fn draw_str_fg<T: FrameBuffer>(buf: &mut T, x: i64, y: i64, color: u32, s: &str) {
    for (i, c) in s.chars().enumerate() {
        draw_font_fg(buf, x + i as i64 * 8, y, color, c)
    }
}

pub fn draw_test_pattern<T: FrameBuffer>(buf: &mut T) {
    let width = 128;
    let left = buf.width() - width - 1;
    let colors = [0x000000, 0xff0000, 0x00ff00, 0x0000ff];
    let height = 64;
    for (index, color) in colors.iter().enumerate() {
        let y = index as i64 * height;
        fill_rect(buf, *color, left, y, height, height).expect("fill_rect failed");
        fill_rect(buf, !*color, left + height, y, height, height)
            .expect("fill_rect failed");
    }
    let points = [(0, 0), (0, width), (width, 0), (width, width)];
    for (x0, y0) in points.iter() {
        for (x1, y1) in points.iter() {
            let _ = draw_line(buf, 0xffffff, left + *x0, *y0, left + *x1, *y1);
        }
    }
    draw_str_fg(
        buf,
        left,
        height * colors.len() as i64,
        0x00ff00,
        "0123456789",
    );
    draw_str_fg(
        buf,
        left,
        height * colors.len() as i64 + 16,
        0x00ff00,
        "ABCDEF",
    );
}

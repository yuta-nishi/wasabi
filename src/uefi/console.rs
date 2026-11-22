use crate::graphics::Bitmap;
use crate::graphics::draw_font_fg;
use core::fmt;

/// Writes text onto any bitmap, one 8x16 cell per character.
pub struct BitmapTextWriter<'a, B: Bitmap> {
    bitmap: &'a mut B,
    cursor_x: i64,
    cursor_y: i64,
}
impl<'a, B: Bitmap> BitmapTextWriter<'a, B> {
    pub fn new(bitmap: &'a mut B) -> Self {
        Self::resume_at(bitmap, 0, 0)
    }

    /// Continues writing at a cursor saved by [`Self::cursor`].
    pub fn resume_at(bitmap: &'a mut B, cursor_x: i64, cursor_y: i64) -> Self {
        Self {
            bitmap,
            cursor_x,
            cursor_y,
        }
    }

    /// Where the next character goes, to be handed back to [`Self::resume_at`].
    pub fn cursor(&self) -> (i64, i64) {
        (self.cursor_x, self.cursor_y)
    }
}
// SAFETY: the writer only holds a `&mut` to the framebuffer, which is a fixed
// address the CPU writes through; moving it between contexts on this single-CPU
// OS moves that borrow and nothing else.
unsafe impl<B: Bitmap> Send for BitmapTextWriter<'_, B> {}

impl<B: Bitmap> fmt::Write for BitmapTextWriter<'_, B> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            if c == '\n' {
                self.cursor_y += 16;
                self.cursor_x = 0;
                continue;
            }
            draw_font_fg(self.bitmap, self.cursor_x, self.cursor_y, 0xffffff, c);
            self.cursor_x += 8;
        }
        Ok(())
    }
}

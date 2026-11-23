use crate::graphics::FrameBuffer;
use crate::graphics::draw_font_fg;
use core::fmt;

/// Writes text onto any frame buffer, one 8x16 cell per character.
pub struct FrameBufferTextWriter<'a, B: FrameBuffer> {
    frame_buffer: &'a mut B,
    cursor_x: i64,
    cursor_y: i64,
}
impl<'a, B: FrameBuffer> FrameBufferTextWriter<'a, B> {
    pub fn new(frame_buffer: &'a mut B) -> Self {
        Self::resume_at(frame_buffer, 0, 0)
    }

    /// Continues writing at a cursor saved by [`Self::cursor`].
    pub fn resume_at(frame_buffer: &'a mut B, cursor_x: i64, cursor_y: i64) -> Self {
        Self {
            frame_buffer,
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
unsafe impl<B: FrameBuffer> Send for FrameBufferTextWriter<'_, B> {}

impl<B: FrameBuffer> fmt::Write for FrameBufferTextWriter<'_, B> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            if c == '\n' {
                self.cursor_y += 16;
                self.cursor_x = 0;
                continue;
            }
            draw_font_fg(self.frame_buffer, self.cursor_x, self.cursor_y, 0xffffff, c);
            self.cursor_x += 8;
        }
        Ok(())
    }
}

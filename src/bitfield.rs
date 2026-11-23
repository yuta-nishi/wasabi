//! Named fields over a register word.

/// Names the bit fields of a 32-bit register word.
///
/// Each field becomes a getter named after the field, so call sites never carry
/// shift counts; writes are left to methods that say what they do, since a write
/// often means "clear this change" or "start that operation" rather than "put
/// this number there".
#[macro_export]
macro_rules! bitfield {
    ($(#[$meta:meta])* $name:ident { $($field:ident : $hi:expr, $lo:expr),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub struct $name(u32);

        impl $name {
            pub const fn new(raw: u32) -> Self {
                Self(raw)
            }

            pub const fn raw(self) -> u32 {
                self.0
            }

            pub fn set_raw(&mut self, raw: u32) {
                self.0 = raw;
            }

            /// The bits of one field, in place.
            pub const fn mask(hi: u32, lo: u32) -> u32 {
                (((1u32 << (hi - lo + 1)) - 1) << lo)
            }

            $(
                #[allow(dead_code)]
                pub const fn $field(self) -> u32 {
                    (self.0 >> $lo) & ((1u32 << ($hi - $lo + 1)) - 1)
                }
            )*
        }
    };
}

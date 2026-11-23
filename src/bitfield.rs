/// Names the bit fields of a register word.
///
/// Each field becomes a getter named after the field, so call sites never carry
/// shift counts; writes are left to methods that say what they do, since a write
/// often means "clear this change" or "start that operation" rather than "put
/// this number there".
#[macro_export]
macro_rules! bitfield {
    ($(#[$meta:meta])* $name:ident : $raw:ty { $($field:ident : $hi:literal, $lo:literal $(-> $setter:ident)? ),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub struct $name($raw);

        impl $name {
            pub const fn new(raw: $raw) -> Self {
                Self(raw)
            }

            pub const fn raw(self) -> $raw {
                self.0
            }

            #[allow(dead_code)]
            pub fn set_raw(&mut self, raw: $raw) {
                self.0 = raw;
            }

            /// The bits of one field, in place.
            pub const fn mask(hi: u32, lo: u32) -> $raw {
                ((((1 as $raw) << (hi - lo + 1)) - 1) << lo)
            }

            $(
                #[allow(dead_code)]
                pub const fn $field(self) -> $raw {
                    (self.0 >> $lo) & (((1 as $raw) << ($hi - $lo + 1)) - 1)
                }

                $(
                    #[allow(dead_code)]
                    pub fn $setter(&mut self, value: $raw) {
                        let mask = Self::mask($hi, $lo);
                        self.0 = (self.0 & !mask) | ((value << $lo) & mask);
                    }
                )?
            )*
        }
    };
}

use crate::x86::hlt;
use crate::x86::write_io_port_u8;

/// The statuses we hand to QEMU through the `isa-debug-exit` device.
///
/// QEMU exits with `(code << 1) | 1`, so `Success` (0x1) makes QEMU exit with 3,
/// which `scripts/launch_qemu.sh` maps back to a passing `cargo test`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x1,
    Fail = 0x2,
}

pub fn exit_qemu(exit_code: QemuExitCode) -> ! {
    // https://github.com/qemu/qemu/blob/master/hw/misc/debugexit.c
    write_io_port_u8(0xf4, exit_code as u8);
    loop {
        hlt()
    }
}

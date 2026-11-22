use crate::executor::Executor;
use crate::info;
use crate::input::Line;
use crate::println;
use crate::serial::Serial;
use crate::timer;
use core::time::Duration;

/// Spawns the demo tasks and hands the CPU to the executor forever.
pub fn run() -> ! {
    EXECUTOR.spawn(async {
        for step in 0..3 {
            info!(
                "task a: step {step} at {} ms",
                timer::nanoseconds() / 1_000_000
            );
            timer::sleep(Duration::from_millis(50)).await;
        }
        info!("task a: done");
    });
    EXECUTOR.spawn(async {
        for step in 0..3 {
            info!(
                "task b: step {step} at {} ms",
                timer::nanoseconds() / 1_000_000
            );
            timer::sleep(Duration::from_millis(30)).await;
        }
        info!("task b: done");
    });
    spawn_serial_console();
    EXECUTOR.run()
}

/// Polls the serial port and logs each line once Enter arrives.
///
/// `Serial::try_read` only touches the data register when the line status says a
/// byte is waiting, so this task polls with `sleep` and the other tasks keep
/// their turns.
fn spawn_serial_console() {
    EXECUTOR.spawn(async {
        let mut serial = Serial;
        let mut line = Line::new();
        // QEMU's window and the terminal both take keys: only the terminal's
        // reach COM1, and only while no key is held by the window.
        info!("serial console: type in the terminal running QEMU, then press Enter");
        loop {
            while let Some(byte) = serial.try_read() {
                match line.push(byte) {
                    Some(text) => {
                        let text = core::str::from_utf8(text).unwrap_or("<not utf-8>");
                        info!("input: \"{text}\"");
                    }
                    None if byte == b'\r' || byte == b'\n' => println!(),
                    None => {}
                }
            }
            timer::sleep(Duration::from_millis(10)).await;
        }
    });
}

/// The executor the OS runs its tasks on.
static EXECUTOR: Executor = Executor::new();

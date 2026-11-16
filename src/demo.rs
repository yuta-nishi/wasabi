//! The tasks the OS runs after boot, so the timer and the executor can be
//! watched while reading the code.

use crate::executor::Executor;
use crate::info;
use crate::interrupts;
use crate::timer;
use core::sync::atomic::Ordering;
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
    EXECUTOR.spawn(async {
        let mut last = 0;
        loop {
            let now = interrupts::TICKS.load(Ordering::Relaxed);
            if now >= last + 1_000 {
                info!("timer ticks: {now}");
                last = now;
            }
            timer::sleep(Duration::from_millis(100)).await;
        }
    });
    EXECUTOR.run()
}

/// The executor the OS runs its tasks on.
static EXECUTOR: Executor = Executor::new();

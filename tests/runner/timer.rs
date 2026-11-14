use core::pin::pin;
use core::task::Context;
use core::task::Waker;
use wasabi::timer;

pub fn sleep_waits_for_its_deadline() {
    let start = timer::nanoseconds();
    let mut sleep = pin!(timer::sleep(1_000));
    let mut context = Context::from_waker(Waker::noop());

    while sleep.as_mut().poll(&mut context).is_pending() {}

    let elapsed = timer::nanoseconds() - start;
    assert!(elapsed >= 1_000_000, "slept for only {elapsed} ns");
}

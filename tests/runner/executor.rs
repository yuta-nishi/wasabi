use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;
use wasabi::executor::Executor;
use wasabi::executor::yield_now;

static COMPLETED: AtomicU64 = AtomicU64::new(0);
static ORDER: AtomicU64 = AtomicU64::new(0);

pub fn executor_runs_every_task_to_completion() {
    COMPLETED.store(0, Ordering::Relaxed);
    let executor = Executor::new();
    for _ in 0..3 {
        executor.spawn(async {
            COMPLETED.fetch_add(1, Ordering::Relaxed);
        });
    }

    while executor.run_ready() > 0 {}

    assert_eq!(COMPLETED.load(Ordering::Relaxed), 3);
}

pub fn yielding_lets_another_task_run() {
    ORDER.store(0, Ordering::Relaxed);
    let executor = Executor::new();
    executor.spawn(async {
        push(1);
        yield_now().await;
        push(1);
    });
    executor.spawn(async {
        push(2);
    });

    while executor.run_ready() > 0 {}

    // The first task runs up to its `await`, the second runs, then the first
    // finishes: 1, 2, 1.
    assert_eq!(ORDER.load(Ordering::Relaxed), 121);
}

fn push(digit: u64) {
    ORDER.store(
        ORDER.load(Ordering::Relaxed) * 10 + digit,
        Ordering::Relaxed,
    );
}

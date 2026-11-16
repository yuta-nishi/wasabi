//! A cooperative executor for `async` tasks.
//!
//! A task is a future boxed and pinned on the heap, plus the flag that says it
//! wants to be polled again. Waking works through [`alloc::task::Wake`], so the
//! `Waker` a future receives can be cloned and kept: waking simply sets the
//! flag, and the executor polls the task on its next round.
//!
//! Nothing here is preemptive. A future must return `Pending` for other tasks to
//! get a turn, which is what [`yield_now`] is for.

use crate::interrupts::TICKS;
use crate::mutex::Mutex;
use crate::x86::hlt;
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;
use core::task::Context;
use core::task::Poll;
use core::task::Waker;

/// One future the executor can poll.
pub struct Task {
    future: UnsafeCell<Pin<Box<dyn Future<Output = ()>>>>,
    woken: AtomicBool,
    /// Set once the future returned `Ready`; the executor must not poll it again.
    done: AtomicBool,
}

impl Task {
    fn new(future: impl Future<Output = ()> + 'static) -> Self {
        Self {
            future: UnsafeCell::new(Box::pin(future)),
            woken: AtomicBool::new(true),
            done: AtomicBool::new(false),
        }
    }

    /// Polls the future once and reports whether the task ran. A finished task
    /// is never polled again: the state machine must not be resumed after it
    /// returned `Ready`.
    fn poll(self: &Arc<Self>) -> bool {
        if self.done.load(Ordering::Acquire) {
            return false;
        }
        let waker = Waker::from(self.clone());
        let mut context = Context::from_waker(&waker);
        // SAFETY: the executor polls a task from one place at a time, and the
        // future is pinned, so handing out `&mut` here cannot alias a poll.
        let future = unsafe { &mut *self.future.get() };
        if future.as_mut().poll(&mut context).is_ready() {
            self.done.store(true, Ordering::Release);
        }
        true
    }
}

// SAFETY: `future` is only touched by `poll`, which the executor calls from one
// thread at a time; everything else is atomic.
unsafe impl Send for Task {}
// SAFETY: as above; there is no OS thread that could race with the executor.
unsafe impl Sync for Task {}

impl alloc::task::Wake for Task {
    fn wake(self: Arc<Self>) {
        self.woken.store(true, Ordering::Release);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::Release);
    }
}

/// Runs `async` tasks one at a time, in spawn order, whenever they are woken.
pub struct Executor {
    tasks: Mutex<Vec<Arc<Task>>>,
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor {
    pub const fn new() -> Self {
        Self {
            tasks: Mutex::new(Vec::new()),
        }
    }

    /// Adds a future and returns the task that owns it.
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) -> Arc<Task> {
        let task = Arc::new(Task::new(future));
        self.tasks.lock().push(task.clone());
        task
    }

    /// Marks every task as woken, so the next round polls all of them. The timer
    /// interrupt calls this: time has moved on, so a `sleep` may have expired
    /// even though nobody woke it.
    pub fn wake_all(&self) {
        for task in self.tasks.lock().iter() {
            task.woken.store(true, Ordering::Release);
        }
    }

    /// Polls every task that has been woken and reports how many ran.
    pub fn run_ready(&self) -> usize {
        // Take a snapshot so a task that spawns another one cannot deadlock the
        // lock that protects the list.
        let tasks: Vec<Arc<Task>> = self.tasks.lock().clone();
        let mut ran = 0;
        for task in tasks {
            if task.woken.swap(false, Ordering::AcqRel) && task.poll() {
                ran += 1;
            }
        }
        ran
    }

    /// Runs tasks forever. A timer tick marks every task woken, because time may
    /// have passed a `sleep` deadline; when nothing is ready the CPU sleeps
    /// until the next interrupt instead of spinning.
    pub fn run(&self) -> ! {
        let mut last_ticks = TICKS.load(Ordering::Relaxed);
        loop {
            let ticks = TICKS.load(Ordering::Relaxed);
            if ticks != last_ticks {
                last_ticks = ticks;
                self.wake_all();
            }
            self.run_ready();
            // Nothing can make progress until time moves: sleep until the timer
            // (or any other interrupt) arrives.
            if TICKS.load(Ordering::Relaxed) == ticks {
                hlt();
            }
        }
    }
}

/// A future that returns `Pending` once, waking itself, so that the executor
/// gets to poll the other tasks first.
pub struct YieldNow {
    yielded: bool,
}

impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Gives the other tasks a chance to run.
pub fn yield_now() -> YieldNow {
    YieldNow { yielded: false }
}

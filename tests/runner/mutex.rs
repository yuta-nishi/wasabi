use wasabi::mutex::Mutex;

static COUNTER: Mutex<u64> = Mutex::new(0);

fn assert_sync<T: Sync>() {}

pub fn mutex_guards_can_be_taken_again_after_dropping() {
    *COUNTER.lock() = 0;
    {
        let mut guard = COUNTER.lock();
        *guard += 1;
    }
    // The previous guard was dropped, so this lock must not spin forever.
    *COUNTER.lock() += 1;
    assert_eq!(*COUNTER.lock(), 2);
}

pub fn mutex_is_shareable() {
    assert_sync::<Mutex<u64>>();
}

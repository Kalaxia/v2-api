use std::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
    marker::{Send, Sync},
};

const UNINIT : usize = 0;
const INITING : usize = 1;
const INIT : usize = 3;

pub struct SyncOnceCell<T> {
    state: AtomicUsize,
    inner: UnsafeCell<MaybeUninit<T>>,
}

impl<T> SyncOnceCell<T> {
    pub const fn new() -> Self {
        Self { state: AtomicUsize::new(UNINIT), inner: UnsafeCell::new(MaybeUninit::uninit()) }
    }

    pub fn set(&self, value: T) {
        match self.state.fetch_or(INITING, Ordering::SeqCst) {
            UNINIT => {
                unsafe { self.inner.get().write_volatile(MaybeUninit::new(value)) };
                self.state.store(INIT, Ordering::Release);
            },
            _ => { },
        }
    }

    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == INIT {
            Some(unsafe { &*(self.inner.get() as *const T)})
        } else {
            None
        }
    }
}

unsafe impl<T:Send> Send for SyncOnceCell<T> {}
unsafe impl<T:Sync> Sync for SyncOnceCell<T> {}

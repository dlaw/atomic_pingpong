//! Lightweight ping-pong buffer intended for no-std targets.
//!
//! A ping-pong buffer is a two-element buffer which functions as a single-producer,
//! single-consumer queue. One element is reserved for writing by the producer, and
//! the other element is reserved for reading by the consumer. When writing and
//! reading are finished, the roles of the two elements are swapped (i.e. the one
//! which was written will be next to be read, and the one which was read will be
//! next to be written). This approach avoids the need for memory copies, which is
//! important when the element size is large.
//!
//! The ping-pong buffer is specifically designed to allow simultaneous reading and
//! writing.  However, the roles of the two elements can only be safely swapped when
//! neither reading or writing is in progress.  It is the user's responsibility to
//! ensure that the timing of reads and writes allows for this to happen.  If reads
//! and writes are interleaved such that one or the other is always in progress,
//! then the roles of the buffer elements will never be able to swap, and the reader
//! will continue to read an old value rather than the new values which are being
//! written.
//!
//! A reference for reading is acquired by calling `Buffer<T>::read()`, and a mutable
//! reference for writing is acquired by calling `Buffer<T>::write()`.  The types
//! returned are smart pointers (`Ref<T>` and `RefMut<T>`, respectively), which
//! automatically update the state of the ping-pong buffer when they are dropped.
//! Thus, it is important to ensure that these references are dropped as soon as
//! reading or writing is finished.  Attempting to acquire a second reference for
//! reading or a second reference for writing will result in a panic if the first
//! reference of that type has not been dropped.
//!
//! The memory footprint of a `Buffer<T>` is precisely two of `T` plus one additional
//! byte (an `AtomicU8`) which is used to synchronize access by the producer and
//! consumer. The performance overhead from this implementation is minimal -- no more
//! than a few instructions (as long as compiler optimization is enabled). However,
//! this crate cannot be used on targets which don't include atomic compare/swap in
//! their instruction sets (e.g. thumbv6m).

pub struct Buffer<T: Default> {
    ping: core::cell::UnsafeCell<T>,
    pong: core::cell::UnsafeCell<T>,
    flags: core::sync::atomic::AtomicU8,
}

pub struct Ref<'a, T> {
    ptr: &'a T,
    flags: &'a core::sync::atomic::AtomicU8,
}

pub struct RefMut<'a, T> {
    ptr: &'a mut T,
    flags: &'a core::sync::atomic::AtomicU8,
}

const LOCK_READ: u8 = 0b0001;
const LOCK_WRITE: u8 = 0b0010;
const MODE_READ_PING_WRITE_PONG: u8 = 0b0100; // when NOT set, we read pong and write ping
const WANT_MODE_CHANGE: u8 = 0b1000;

impl<T: Default> Buffer<T> {
    /// Returns a new ping-pong buffer with the elements initialized to their default
    /// values.
    pub fn new() -> Buffer<T> {
        Buffer {
            ping: core::cell::UnsafeCell::default(),
            pong: core::cell::UnsafeCell::default(),
            flags: core::sync::atomic::AtomicU8::new(0),
        }
    }
    /// Returns a `Ref<T>` smart pointer providing read-only access to the ping-pong
    /// buffer.  This function panics if the previous `Ref<T>` has not been dropped yet.
    /// The `T` element pointed to by the reference will be a value that was previously
    /// written through a call to `write`, or else the default value of `T` if `write`
    /// has not been called yet.
    pub fn read(&self) -> Ref<T> {
        let flags = self
            .flags
            .fetch_update(
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
                |flags| match flags & LOCK_READ {
                    0 => Some(flags | LOCK_READ),
                    _ => None,
                },
            )
            .expect("ping pong buffer should only have one reader at a time");
        let buffer = match flags & MODE_READ_PING_WRITE_PONG {
            0 => &self.pong,
            _ => &self.ping,
        };
        Ref {
            ptr: unsafe { &*buffer.get() },
            flags: &self.flags,
        }
    }
    /// Returns a `RefMut<T>` smart pointer providing mutable access to the ping-pong
    /// buffer. This function panics if the previous `RefMut<T>` has not been dropped yet.
    /// The `T` element pointed to by the reference is valid, but its initial value will
    /// be left over from a previous call to `write`.
    pub fn write(&self) -> RefMut<T> {
        let flags = self
            .flags
            .fetch_update(
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
                |flags| match flags & LOCK_WRITE {
                    0 => Some(flags | LOCK_WRITE),
                    _ => None,
                },
            )
            .expect("ping pong buffer should only have one writer at a time");
        let buffer = match flags & MODE_READ_PING_WRITE_PONG {
            0 => &self.ping,
            _ => &self.pong,
        };
        RefMut {
            ptr: unsafe { &mut *buffer.get() },
            flags: &self.flags,
        }
    }
}

unsafe impl<T: Default + Send> Send for Buffer<T> {}
unsafe impl<T: Default + Sync> Sync for Buffer<T> {}

impl<'a, T> core::ops::Deref for Ref<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.ptr
    }
}

impl<'a, T> Drop for Ref<'a, T> {
    fn drop(&mut self) {
        self.flags
            .fetch_update(
                core::sync::atomic::Ordering::Release,
                core::sync::atomic::Ordering::Relaxed,
                |mut flags| {
                    flags &= !LOCK_READ;
                    if (flags & LOCK_WRITE == 0) && (flags & WANT_MODE_CHANGE != 0) {
                        flags &= !WANT_MODE_CHANGE;
                        flags ^= MODE_READ_PING_WRITE_PONG;
                    }
                    Some(flags)
                },
            )
            .unwrap();
    }
}

impl<'a, T> core::ops::Deref for RefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.ptr
    }
}

impl<'a, T> core::ops::DerefMut for RefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.ptr
    }
}

impl<'a, T> Drop for RefMut<'a, T> {
    fn drop(&mut self) {
        self.flags
            .fetch_update(
                core::sync::atomic::Ordering::Release,
                core::sync::atomic::Ordering::Relaxed,
                |mut flags| {
                    flags &= !LOCK_WRITE;
                    if flags & LOCK_READ == 0 {
                        flags &= !WANT_MODE_CHANGE;
                        flags ^= MODE_READ_PING_WRITE_PONG;
                    } else {
                        flags |= WANT_MODE_CHANGE;
                    }
                    Some(flags)
                },
            )
            .unwrap();
    }
}

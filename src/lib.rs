//! Lightweight ping-pong buffer intended for no_std targets.
//!
//! A ping-pong buffer is a two-element buffer which allows simultaneous access
//! by a single producer and a single consumer.  One element is reserved for
//! writing by the producer, and the other element is reserved for reading by
//! the consumer. When writing and reading are finished, the roles of the two
//! elements are swapped (i.e. the one which was written will be next to be
//! read, and the one which was read will be next to be written). This approach
//! avoids the need for memory copies, which improves performance when the
//! element size is large.
//!
//! The ping-pong buffer is specifically designed to allow simultaneous reading
//! and writing.  However, the roles of the two elements can only be safely
//! swapped when neither reading or writing is in progress.  It is the user's
//! responsibility to ensure that the timing of reads and writes allows for this
//! to happen.  If reads and writes are interleaved such that one or the other
//! is always in progress, then the roles of the buffer elements will never be
//! able to swap, and the reader will continue to read an old value rather than
//! the new values which are being written.
//!
//! A reference for reading is acquired by calling `Buffer<T>::read()`, and a
//! mutable reference for writing is acquired by calling `Buffer<T>::write()`.
//! The types returned are smart pointers (`Ref<T>` and `RefMut<T>`,
//! respectively), which automatically update the state of the ping-pong buffer
//! when they are dropped. Thus, it is important to ensure that these references
//! are dropped as soon as reading or writing is finished.  Attempting to
//! acquire a second reference for reading or a second reference for writing
//! will result in a panic if the first reference of that type has not been
//! dropped.
//!
//! Ordinarily, calls to `read()` and `write()` are as permissive as possible:
//! `read()` always succeeds unless reading is already in progress, and
//! `write()` always succeeds unless writing is already in progress. Thus,
//! depending on the timing of `read()` and `write()` calls, it is possible that
//! some data which is written may never be read, and other data which is
//! written may be read multiple times.  (This is an important distinction
//! between a ping-pong buffer and a two-element ring buffer.)  Alternatively,
//! the `read_once()` function only returns a `Ref<T>` if it points to data
//! which has not yet been read, and the `write_no_discard()` function only
//! returns a `RefMut<T>` if the buffer does not currently contain unread data.
//!
//! The memory footprint of a `Buffer<T>` is precisely two of `T` plus one
//! additional byte (an `AtomicU8`) which is used to synchronize access by the
//! producer and consumer. The performance overhead from this implementation is
//! minimal -- no more than a few instructions (as long as compiler optimization
//! is enabled). However, this crate cannot be used on targets which don't
//! include atomic compare/swap in their instruction sets (e.g. thumbv6m).

#![no_std]

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic;

struct BufferState(atomic::AtomicU8);

pub struct Buffer<T> {
    ping: UnsafeCell<T>,
    pong: UnsafeCell<T>,
    state: BufferState,
}

// Smart pointer for reading from the buffer. Updates state when dropped.
pub struct Ref<'a, T> {
    ptr: &'a T,
    state: &'a BufferState,
}

// Smart pointer for writing to the buffer. Updates state when dropped.
pub struct RefMut<'a, T> {
    ptr: &'a mut T,
    state: &'a BufferState,
}

// We use a bitmask to track buffer state, rather than booleans or enum types,
// in order to permit the use of an AtomicU8 representation.  This allows atomic
// updates to multiple flags at once.

// Basically all of the tricky implementation logic for the ping-pong buffer
// lives in BufferState:
impl BufferState {
    const LOCK_READ: u8 = 0b0000_0001;
    const LOCK_WRITE: u8 = 0b0000_0010;
    const MODE_READ_PING_WRITE_PONG: u8 = 0b0000_0100; // when NOT set, read pong & write ping
    const WANT_MODE_CHANGE: u8 = 0b0000_1000;
    const NEW_DATA_READY_TO_READ: u8 = 0b0001_0000;
    const fn new() -> BufferState {
        BufferState(atomic::AtomicU8::new(0))
    }
    /// If `condition()` is true, update the state byte with `action()` (using
    /// "Acquire" ordering) and return its previous value.  If `condition()`
    /// is false, return None without changing the state byte.
    #[inline(always)]
    fn lock(&self, condition: fn(u8) -> bool, action: fn(u8) -> u8) -> Option<u8> {
        self.0
            .fetch_update(
                atomic::Ordering::Acquire,
                atomic::Ordering::Relaxed,
                |flags| {
                    if condition(flags) {
                        Some(action(flags))
                    } else {
                        None
                    }
                },
            )
            .ok()
    }
    fn lock_for_read(&self) -> Option<u8> {
        self.lock(
            |flags| flags & BufferState::LOCK_READ == 0,
            |flags| (flags | BufferState::LOCK_READ) & !BufferState::NEW_DATA_READY_TO_READ,
        )
    }
    fn lock_for_read_once(&self) -> Option<u8> {
        self.lock(
            |flags| {
                (flags & BufferState::LOCK_READ == 0)
                    && (flags & BufferState::NEW_DATA_READY_TO_READ != 0)
            },
            |flags| (flags | BufferState::LOCK_READ) & !BufferState::NEW_DATA_READY_TO_READ,
        )
    }
    fn lock_for_write(&self) -> Option<u8> {
        self.lock(
            |flags| flags & BufferState::LOCK_WRITE == 0,
            |flags| flags | BufferState::LOCK_WRITE,
        )
    }
    fn lock_for_write_no_discard(&self) -> Option<u8> {
        self.lock(
            |flags| {
                (flags & BufferState::LOCK_WRITE == 0)
                    && (flags & BufferState::NEW_DATA_READY_TO_READ == 0)
            },
            |flags| flags | BufferState::LOCK_WRITE,
        )
    }
    /// Update the state byte with `action()` (using "Release" ordering).
    #[inline(always)]
    fn release(&self, action: fn(u8) -> u8) {
        self.0
            .fetch_update(
                atomic::Ordering::Release,
                atomic::Ordering::Relaxed,
                |flags| Some(action(flags)),
            )
            .unwrap();
    }
    fn release_read(&self) {
        self.release(|mut flags| {
            flags &= !BufferState::LOCK_READ;
            if (flags & BufferState::LOCK_WRITE == 0)
                && (flags & BufferState::WANT_MODE_CHANGE != 0)
            {
                flags &= !BufferState::WANT_MODE_CHANGE;
                flags ^= BufferState::MODE_READ_PING_WRITE_PONG;
                flags |= BufferState::NEW_DATA_READY_TO_READ;
            }
            flags
        })
    }
    fn release_write(&self) {
        self.release(|mut flags| {
            flags &= !BufferState::LOCK_WRITE;
            if flags & BufferState::LOCK_READ == 0 {
                flags &= !BufferState::WANT_MODE_CHANGE;
                flags ^= BufferState::MODE_READ_PING_WRITE_PONG;
                flags |= BufferState::NEW_DATA_READY_TO_READ;
            } else {
                flags |= BufferState::WANT_MODE_CHANGE;
            }
            flags
        })
    }
}

impl<T: Copy> Buffer<T> {
    /// Returns a new ping-pong buffer with the elements initialized to the
    /// specified values.
    pub const fn new(default: T) -> Buffer<T> {
        Buffer {
            ping: UnsafeCell::new(default),
            pong: UnsafeCell::new(default),
            state: BufferState::new(),
        }
    }
}

impl<T: Default> Buffer<T> {
    /// Returns a new ping-pong buffer with the elements initialized to their
    /// default values.
    pub fn default() -> Buffer<T> {
        Buffer {
            ping: UnsafeCell::default(),
            pong: UnsafeCell::default(),
            state: BufferState::new(),
        }
    }
}

impl<T> Buffer<MaybeUninit<T>> {
    /// Returns a new ping-pong buffer with uninitialized elements.
    pub const fn uninit() -> Buffer<MaybeUninit<T>> {
        Buffer {
            ping: UnsafeCell::new(MaybeUninit::uninit()),
            pong: UnsafeCell::new(MaybeUninit::uninit()),
            state: BufferState::new(),
        }
    }
}

impl<T> Buffer<T> {
    /// Returns a `Ref<T>` smart pointer providing read-only access to the
    /// ping-pong buffer, or `None` if the `Ref<T>` from a previous call has
    /// not been dropped yet. If a call to `write` previously finished and
    /// the ping-pong buffer was able to swap, the `T` element pointed to by
    /// the reference will be a value that was previously written.
    /// Otherwise, the `T` element will have its specified initial
    /// value based on the function which was used to construct the ping-pong
    /// buffer.
    pub fn read(&self) -> Option<Ref<T>> {
        let flags = self.state.lock_for_read()?;
        let buffer = match flags & BufferState::MODE_READ_PING_WRITE_PONG {
            0 => &self.pong,
            _ => &self.ping,
        };
        Some(Ref {
            ptr: unsafe { &*buffer.get() },
            state: &self.state,
        })
    }
    /// Returns a `RefMut<T>` smart pointer providing mutable access to the
    /// ping-pong buffer, or `None` if the `RefMut<T>` from a previous call
    /// has not been dropped yet. The `T` element pointed to by the
    /// reference might contain a value that was previously written, or else
    /// it may still contain the initial value based on the function which
    /// was used to construct the ping-pong buffer.
    pub fn write(&self) -> Option<RefMut<T>> {
        let flags = self.state.lock_for_write()?;
        let buffer = match flags & BufferState::MODE_READ_PING_WRITE_PONG {
            0 => &self.ping,
            _ => &self.pong,
        };
        Some(RefMut {
            ptr: unsafe { &mut *buffer.get() },
            state: &self.state,
        })
    }
    /// Ordinarily, the `read()` function allows the same data to be read
    /// multiple times, and it allows the initial value to be read prior to
    /// any calls to `write()`. In contrast, `read_once()` only returns a
    /// `Ref<T>` if it points to new data which has been written into the
    /// buffer and not yet read. Returns `None` if new data is not available
    /// to read or if a previous `Ref<T>` has not yet been dropped.
    pub fn read_once(&self) -> Option<Ref<T>> {
        let flags = self.state.lock_for_read_once()?;
        let buffer = match flags & BufferState::MODE_READ_PING_WRITE_PONG {
            0 => &self.pong,
            _ => &self.ping,
        };
        Some(Ref {
            ptr: unsafe { &*buffer.get() },
            state: &self.state,
        })
    }
    /// Ordinarily, the `write()` function allows an arbitrary number of
    /// sequential writes, even if data which was previously written (and
    /// will now be overwritten) has never been read.  In contrast,
    /// `write_no_discard()` only returns a `RefMut<T>` if no unread
    /// data will be overwritten by this write. Returns `None` if the buffer
    /// already contains unread data or if a previous `RefMut<T>` has not
    /// yet been dropped.
    pub fn write_no_discard(&self) -> Option<RefMut<T>> {
        let flags = self.state.lock_for_write_no_discard()?;
        let buffer = match flags & BufferState::MODE_READ_PING_WRITE_PONG {
            0 => &self.ping,
            _ => &self.pong,
        };
        Some(RefMut {
            ptr: unsafe { &mut *buffer.get() },
            state: &self.state,
        })
    }
}

// Buffer<T> is Send and Sync (as long as its elements are Send and Sync)
// because of the atomic locking system implemented by BufferState. In
// particular:
//  1. Only one Ref<T> associated with this buffer can exist at any time.
//  2. Only one RefMut<T> associated with this buffer can exist at any time.
//  3. The Ref<T> and the RefMut<T> will always point to different elements.
unsafe impl<T: Send> Send for Buffer<T> {}
unsafe impl<T: Sync> Sync for Buffer<T> {}

impl<'a, T> core::ops::Deref for Ref<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.ptr
    }
}

impl<'a, T> Drop for Ref<'a, T> {
    fn drop(&mut self) {
        self.state.release_read();
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
        self.state.release_write();
    }
}

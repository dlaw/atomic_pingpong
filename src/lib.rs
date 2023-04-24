//! Lightweight ping-pong buffer intended for no_std targets.
//!
//! A ping-pong buffer is a two-element buffer which allows simultaneous access
//! by a single producer and a single consumer.  One element is reserved for
//! writing by the producer, and the other element is reserved for reading by
//! the consumer. When writing and reading are finished, the roles of the two
//! elements are swapped (i.e. the one which was written will be next to be
//! read, and the one which was read will be next to be overwritten). This
//! approach avoids the need for memory copies, which improves performance when
//! the element size is large.
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
//! acquire a second reference for reading or writing will fail if the first
//! reference of that type has not been dropped.
//!
//! Ordinarily, calls to `read()` and `write()` are as permissive as possible:
//! `read()` succeeds unless reading is already in progress, and `write()`
//! succeeds unless writing is already in progress. Thus, depending on the
//! timing of `read()` and `write()` calls, certain data which is written may
//! never be read, and other data which is written may be read multiple times.
//! (This is an important distinction between a ping-pong buffer and a FIFO
//! ring buffer.) Alternative behavior is possible using the `read_once()`
//! function, which only returns a `Ref<T>` if it points to data which has not
//! yet been read, and the `write_no_discard()` function, which only returns a
//! `RefMut<T>` if the buffer does not currently contain unread data.
//!
//! The memory footprint of a `Buffer<T>` is two of `T` plus one additional byte
//! (an `AtomicU8`) which is used to synchronize access by the producer and
//! consumer. The runtime overhead from this implementation is less than about
//! twenty instructions to acquire or release a reference to the ping-pong
//! buffer (assuming function inlining is enabled).  However, this crate can
//! only be used on targets which include atomic compare/swap in their
//! instruction sets.

#![no_std]

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic;

// Basically all of the tricky synchronization logic for the ping-pong buffer
// lives in the BufferState implementation.  The buffer state is a bitmask
// stored in an AtomicU8, rather than booleans or enums, in order to permit
// atomic updtes to multiple flags at once.  The custom BufferState type
// provides a convenient place for the associated functions and constants.
struct BufferState(atomic::AtomicU8);

/// A `Buffer<T>` consists of two copies of `T` plus one additional byte of
/// state.
pub struct Buffer<T> {
    ping: UnsafeCell<T>,
    pong: UnsafeCell<T>,
    state: BufferState,
}

/// Smart pointer for reading from a `Buffer<T>`.
/// Updates the buffer's state when dropped.
pub struct Ref<'a, T> {
    ptr: &'a T,
    state: &'a BufferState,
}

/// Smart pointer for writing to a `Buffer<T>`.
/// Updates the buffer's state when dropped.
pub struct RefMut<'a, T> {
    ptr: &'a mut T,
    state: &'a BufferState,
}

impl BufferState {
    // Bits of the bitmask:
    const LOCK_READ: u8 = 0b0000_0001;
    const LOCK_WRITE: u8 = 0b0000_0010;
    const MODE_IS_FLIPPED: u8 = 0b0000_0100;
    const WANT_MODE_CHANGE: u8 = 0b0000_1000;
    const NEW_DATA_READY: u8 = 0b0001_0000;

    const fn new() -> Self {
        Self(atomic::AtomicU8::new(0))
    }
    /// If `condition()` is true, atomically update the state byte with
    /// `action()` (using "Acquire" ordering) and return the previous mode.
    /// If `condition()` is false, return None without changing the state byte.
    fn lock(&self, condition: fn(u8) -> bool, action: fn(u8) -> u8) -> Option<bool> {
        match self.0.fetch_update(
            atomic::Ordering::Acquire,
            atomic::Ordering::Relaxed,
            |flags| {
                if condition(flags) {
                    Some(action(flags))
                } else {
                    None
                }
            },
        ) {
            Ok(flags) => Some(flags & Self::MODE_IS_FLIPPED != 0),
            Err(_) => None,
        }
    }
    fn lock_read(&self, allow_repeated: bool) -> Option<bool> {
        self.lock(
            if allow_repeated {
                // allow reading the same data multiple times
                |flags| flags & Self::LOCK_READ == 0
            } else {
                // only lock for reading if there is new unread data
                |flags| flags & (Self::LOCK_READ | Self::NEW_DATA_READY) == Self::NEW_DATA_READY
            },
            |flags| (flags | Self::LOCK_READ) & !Self::NEW_DATA_READY,
        )
    }
    fn lock_write(&self, allow_repeated: bool) -> Option<bool> {
        self.lock(
            if allow_repeated {
                // allow overwriting data which has not yet been read
                |flags| flags & Self::LOCK_WRITE == 0
            } else {
                // only lock for writing if there is not any unread data
                |flags| flags & (Self::LOCK_WRITE | Self::NEW_DATA_READY) == 0
            },
            |flags| flags | Self::LOCK_WRITE,
        )
    }
    /// Atomically update the state byte with `action()`
    /// (using "Release" ordering).
    fn release(&self, action: fn(u8) -> u8) {
        let _ = self.0.fetch_update(
            atomic::Ordering::Release,
            atomic::Ordering::Relaxed,
            |flags| Some(action(flags)),
        ); // always Ok because the closure always returns Some
    }
    fn release_read(&self) {
        self.release(|mut flags| {
            flags &= !Self::LOCK_READ;
            if flags & (Self::LOCK_WRITE | Self::WANT_MODE_CHANGE) == Self::WANT_MODE_CHANGE {
                flags &= !Self::WANT_MODE_CHANGE;
                flags ^= Self::MODE_IS_FLIPPED;
            }
            flags
        })
    }
    fn release_write(&self) {
        self.release(|mut flags| {
            flags &= !Self::LOCK_WRITE;
            flags |= Self::NEW_DATA_READY;
            if flags & Self::LOCK_READ == 0 {
                flags &= !Self::WANT_MODE_CHANGE;
                flags ^= Self::MODE_IS_FLIPPED;
            } else {
                flags |= Self::WANT_MODE_CHANGE;
            }
            flags
        })
    }
}

impl<'a, T> Ref<'a, T> {
    fn new(buf: &'a Buffer<T>, allow_repeated: bool) -> Option<Self> {
        let mode = buf.state.lock_read(allow_repeated)?;
        // If we get here, lock_read() succeeded, so it's safe to access the UnsafeCell
        // which is currently designated for reading.
        let cell = if mode { &buf.ping } else { &buf.pong };
        Some(Ref {
            ptr: unsafe { &*cell.get() },
            state: &buf.state,
        })
    }
}

impl<'a, T> RefMut<'a, T> {
    fn new(buf: &'a Buffer<T>, allow_repeated: bool) -> Option<Self> {
        let mode = buf.state.lock_write(allow_repeated)?;
        // If we get here, lock_write() succeeded, so it's safe to access the UnsafeCell
        // which is currently designated for writing.
        let cell = if mode { &buf.pong } else { &buf.ping };
        Some(RefMut {
            ptr: unsafe { &mut *cell.get() },
            state: &buf.state,
        })
    }
}

impl<'a, T> Drop for Ref<'a, T> {
    /// When a `Ref<'a, T>` is dropped, the state of the corresponding
    /// `Buffer<T>` is automatically updated.
    fn drop(&mut self) {
        self.state.release_read();
    }
}

impl<'a, T> Drop for RefMut<'a, T> {
    /// When a `RefMut<'a, T>` is dropped, the state of the corresponding
    /// `Buffer<T>` is automatically updated.
    fn drop(&mut self) {
        self.state.release_write();
    }
}

impl<'a, T> core::ops::Deref for Ref<'a, T> {
    /// `Ref<'a, T>` dereferences to a `T` element of the `Buffer<T>`.
    type Target = T;
    fn deref(&self) -> &T {
        self.ptr
    }
}

impl<'a, T> core::ops::Deref for RefMut<'a, T> {
    /// `RefMut<'a, T>` dereferences to a `T` element of the `Buffer<T>`.
    type Target = T;
    /// Dereferences the value.
    /// (Required in order to support `deref_mut`;
    /// not likely to be useful on its own.)
    fn deref(&self) -> &T {
        self.ptr
    }
}

impl<'a, T> core::ops::DerefMut for RefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.ptr
    }
}

impl<T: Copy> Buffer<T> {
    /// Returns a new ping-pong buffer with the elements initialized to the
    /// specified value.
    pub const fn new(value: T) -> Self {
        Buffer {
            ping: UnsafeCell::new(value),
            pong: UnsafeCell::new(value),
            state: BufferState::new(),
        }
    }
}

impl<T: Default> Buffer<T> {
    /// Returns a new ping-pong buffer with the elements initialized to their
    /// default value.
    pub fn default() -> Self {
        Buffer {
            ping: UnsafeCell::default(),
            pong: UnsafeCell::default(),
            state: BufferState::new(),
        }
    }
}

impl<T> Buffer<MaybeUninit<T>> {
    /// Returns a new ping-pong buffer with uninitialized elements.
    pub const fn uninit() -> Self {
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
    /// Otherwise, the `T` element will have its specified initial value based
    /// on the function which was used to construct the ping-pong buffer.
    pub fn read(&self) -> Option<Ref<T>> {
        Ref::new(&self, true)
    }
    /// Ordinarily, the `read()` function allows the same data to be read
    /// multiple times, and it allows the initial value to be read prior to
    /// any calls to `write()`. In contrast, `read_once()` only returns a
    /// `Ref<T>` if it points to new data which has been written into the
    /// buffer and not yet read. Returns `None` if new data is not available
    /// to read or if a previous `Ref<T>` has not yet been dropped.
    pub fn read_once(&self) -> Option<Ref<T>> {
        Ref::new(&self, false)
    }
    /// Returns a `RefMut<T>` smart pointer providing mutable access to the
    /// ping-pong buffer, or `None` if the `RefMut<T>` from a previous call
    /// has not been dropped yet. Due to the nature of the ping-pong buffer,
    /// the `T` element pointed to by the reference may have an arbitrary
    /// starting value prior to being overwritten by the caller.
    pub fn write(&self) -> Option<RefMut<T>> {
        RefMut::new(&self, true)
    }
    /// Ordinarily, the `write()` function allows an arbitrary number of
    /// sequential writes, even if data which was previously written (and
    /// will now be overwritten) has never been read.  In contrast,
    /// `write_no_discard()` only returns a `RefMut<T>` if no unread
    /// data will be overwritten by this write. Returns `None` if the buffer
    /// already contains unread data or if a previous `RefMut<T>` has not
    /// yet been dropped.
    pub fn write_no_discard(&self) -> Option<RefMut<T>> {
        RefMut::new(&self, false)
    }
}

unsafe impl<T: Send> Send for Buffer<T> {}
/// `Buffer<T>` safely inherits Send and Sync from `T`
/// because of the following guarantees which it enforces:
///  1. Only one `Ref` associated with this buffer can exist at any time.
///  2. Only one `RefMut` associated with this buffer can exist at any time.
///  3. The `Ref` and the `RefMut` will point to different elements of the
///     buffer.
///  4. Whenever a `Ref` or `RefMut` is created or dropped,
///     the buffer state is updated in a single atomic operation.
unsafe impl<T: Sync> Sync for Buffer<T> {}

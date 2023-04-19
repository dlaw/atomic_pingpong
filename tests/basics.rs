use atomic_pingpong::Buffer;

#[test]
fn swap_after_read() {
    let b: Buffer<i32> = Buffer::default();
    *b.write().unwrap() = 1;
    let r = b.read().unwrap();
    *b.write().unwrap() = 2;
    assert_eq!(*r, 1);
    drop(r);
    assert_eq!(*b.read().unwrap(), 2);
    assert_eq!(*b.read().unwrap(), 2);
}

#[test]
fn swap_after_write() {
    let b: Buffer<i32> = Buffer::default();
    *b.write().unwrap() = 1;
    let mut w = b.write().unwrap();
    *w = 2;
    assert_eq!(*b.read().unwrap(), 1);
    drop(w);
    assert_eq!(*b.read().unwrap(), 2);
    assert_eq!(*b.read().unwrap(), 2);
}

#[test]
fn no_swap_after_interleaved() {
    let b: Buffer<i32> = Buffer::default();
    let mut w = b.write().unwrap();
    *w = 1;
    let r = b.read().unwrap();
    drop(w);
    assert_eq!(*r, 0);
    drop(r);
    assert_eq!(*b.read().unwrap(), 1);
}

#[test]
fn no_double_read() {
    let b: Buffer<i32> = Buffer::default();
    let r = b.read().unwrap();
    assert!(b.read().is_none());
    drop(r);
}

#[test]
fn no_double_write() {
    let b: Buffer<i32> = Buffer::default();
    let w = b.write().unwrap();
    assert!(b.write().is_none());
    drop(w);
}

#[test]
fn read_once() {
    let b: Buffer<i32> = Buffer::default();
    assert!(b.read_once().is_none());
    *b.write().unwrap() = 1;
    assert_eq!(*b.read_once().unwrap(), 1);
    assert!(b.read_once().is_none());
}

#[test]
fn read_once_after_read() {
    let b: Buffer<i32> = Buffer::default();
    let r = b.read().unwrap();
    *b.write().unwrap() = 1;
    assert!(b.read_once().is_none());
    drop(r);
    assert_eq!(*b.read_once().unwrap(), 1);
}

#[test]
fn write_no_discard() {
    let b: Buffer<i32> = Buffer::default();
    *b.write_no_discard().unwrap() = 1;
    assert!(b.write_no_discard().is_none());
    b.read();
    *b.write_no_discard().unwrap() = 2;
}

#[test]
fn write_no_discard_while_reading() {
    let b: Buffer<i32> = Buffer::default();
    let r = b.read().unwrap();
    *b.write_no_discard().unwrap() = 1;
    assert!(b.write_no_discard().is_none());
    drop(r);
}

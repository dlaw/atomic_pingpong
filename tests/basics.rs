use atomic_pingpong::Buffer;

#[test]
fn swap_after_read() {
    let b: Buffer<i32> = Buffer::new();
    *b.write() = 1;
    let r = b.read();
    *b.write() = 2;
    assert_eq!(*r, 1);
    drop(r);
    assert_eq!(*b.read(), 2);
    assert_eq!(*b.read(), 2);
}

#[test]
fn swap_after_write() {
    let b: Buffer<i32> = Buffer::new();
    *b.write() = 1;
    let mut w = b.write();
    *w = 2;
    assert_eq!(*b.read(), 1);
    drop(w);
    assert_eq!(*b.read(), 2);
    assert_eq!(*b.read(), 2);
}

#[test]
fn no_swap_after_interleaved() {
    let b: Buffer<i32> = Buffer::new();
    let mut w = b.write();
    *w = 1;
    let r = b.read();
    drop(w);
    assert_eq!(*r, 0);
    drop(r);
    assert_eq!(*b.read(), 1);
}

#[test]
#[should_panic="ping pong buffer should only have one reader at a time"]
fn no_double_read() {
    let b: Buffer<i32> = Buffer::new();
    let r0 = b.read();
    let r1 = b.read();
    (r0, r1);
}

#[test]
#[should_panic="ping pong buffer should only have one writer at a time"]
fn no_double_write() {
    let b: Buffer<i32> = Buffer::new();
    let w0 = b.write();
    let w1 = b.write();
    (w0, w1);
}
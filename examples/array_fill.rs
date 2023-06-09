//! This example demonstrates how the ping-pong buffer permits unsynchronized
//! reads and writes while never allowing any half-written data to be read.

use atomic_pingpong::Buffer;

fn producer(b: &Buffer<[i32; 4]>) {
    for num in 1..8 {
        {
            println!("                write started");
            let mut w = b.write().unwrap();
            for (i, val) in w.iter_mut().enumerate() {
                *val = num;
                println!("                write buf[{}] = {}", i, *val);
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            println!("                write finished");
        } // we need `w` to go out of scope so the buffer can swap
        std::thread::sleep(std::time::Duration::from_millis(250))
    }
}

fn consumer(b: &Buffer<[i32; 4]>) {
    for _ in 1..8 {
        {
            println!("read started");
            let r = b.read().unwrap();
            for (i, val) in r.iter().enumerate() {
                println!("read buf[{}] = {}", i, val);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            println!("read finished");
        } // we need `r` to go out of scope so the buffer can swap
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn main() {
    let b = Buffer::<[i32; 4]>::default();
    std::thread::scope(|s| {
        s.spawn(|| {
            consumer(&b);
        });
        producer(&b);
    });
}

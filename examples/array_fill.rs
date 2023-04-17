//! This example demonstrates how the ping-pong buffer permits unsynchronized
//! reads and writes while never allowing any half-written data to be read.

fn main() {
    let b = atomic_pingpong::Buffer::<[i32; 4]>::new([0; 4]);
    std::thread::scope(|s| {
        s.spawn(|| {
            // consumer
            for _ in 1..10 {
                {
                    let r = b.read().unwrap();
                    for (i, val) in r.iter().enumerate() {
                        println!("read  buf[{}] = {}", i, val);
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    println!("read finished");
                } // we need `r` to go out of scope so the buffer can swap
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        });
        // producer
        for num in 1..8 {
            {
                let mut w = b.write().unwrap();
                for (i, val) in w.iter_mut().enumerate() {
                    *val = num;
                    println!("write buf[{}] = {}", i, *val);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                println!("write finished");
            } // we need `w` to go out of scope so the buffer can swap
            std::thread::sleep(std::time::Duration::from_millis(250))
        }
    });
}

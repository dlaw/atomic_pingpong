Lightweight ping-pong buffer intended for no-std targets.

A ping-pong buffer is a two-element buffer which functions as a single-producer,
single-consumer queue. One element is reserved for writing by the producer, and
the other element is reserved for reading by the consumer. When writing and
reading are finished, the roles of the two elements are swapped (i.e. the one
which was written will be next to be read, and the one which was read will be
next to be written). This approach avoids the need for memory copies, which is
important when the element size is large.

This implementation uses an AtomicU8 for synchronization, resulting in thread-safety and interrupt-safety with minimum overhead on platforms which support atomic compare and swap.

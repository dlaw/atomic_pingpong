A ping-pong buffer is a two-element buffer which alows for
simultaneous access by a single producer and a single consumer. One
element is reserved for writing by the producer, and the other element
is reserved for reading by the consumer. When writing and reading are
finished, the roles of the two elements are swapped (i.e. the one
which was written will be next to be read, and the one which was read
will be next to be overwritten). This approach avoids the need for
memory copies, which improves performance when the element size is
large.

This ping-pong buffer implementation uses an AtomicU8 for
synchronization between the producer and consumer, resulting in
thread-safety and interrupt-safety with minimum overhead.  This
implementation supports no_std environments, but requires a target
which supports atomic compare and swap.

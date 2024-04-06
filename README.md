# How-not-to-async-rs
Develop an async runtime like thing in Rust for educational purpose.

# What this is not
- This is not going to be a blog post explaining how things are working. Just dive into the code and hope comments are enough :)
- There are many bugs and incorrect design decisions. Some happened accidently and some are created knowingly. Not a lot of effort and time was given for this. A lot of code is taken and modified from the futures crate (incl futures-task, futures-executor etc).
- Have not implemented block-on. But it is easy enough if you got as far as understanding this example.

# Prerequisite
- Need to have a basic undestanding of Pinning, Future, Waker etc.
- This excercise just aims at combining all these concepts together to give a general idea on how an async runtime can be implemented.

For understanding about these concepts, I highly recommend Jon Gjengset youtube videos:
- [Future](youtube.com/watch?v=9_3krAQtD2k)
- [Pinning](youtube.com/watch?v=ThjvMReOXYM)

# How to approach
The source files have concepts implemented in a self contained manner.
1. future.rs - To know about Futures and other helper data structures and traits.
2. task.rs - For understanding RawWakers, Wakers, AtomicWakers and ArcWake.
3. spawn.rs + thread_pool.rs - Scheduler implementation

This completes basic scheduler. The next part is for understanding how to link a IoSource reactor pattern to drive a future.
1. net/io_service.rs - Combines an event loop and scheduler together.
2. net/socket.rs - Example only for accepting a socket connection.
3. main.rs - Has examples. 
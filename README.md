tinycoro
========

Tiny coroutines library, written in Rust.

This crate does _not_ require nightly Rust, and will build today on stable
Rust. Instead, it relies on coroutine-related platform features,
particularly [`<ucontext.h>`](https://en.wikipedia.org/wiki/Setcontext).

Platform support
----------------

* Mac OS X (via `ucontext`)
* Linux (via `ucontext`)
* Other UNIXy systems (via `ucontext`)

Windows has could support an identical API. Pull requests welcome.

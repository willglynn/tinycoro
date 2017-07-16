tinycoro
========

Tiny coroutines library, written in Rust.

This crate does _not_ require nightly Rust, and will build today on stable
Rust. Instead, it relies on coroutine-related platform features,
particularly [`<ucontext.h>`](https://en.wikipedia.org/wiki/Setcontext).

Example
------

```rust
let mut handle = Coroutine::new(|coro: &mut Coroutine| {
    println!("2: in coroutine");
    coro.yield_back();
    println!("4: in coroutine");
});

// handle.is_terminated() == false

println!("1: in caller");
handle.yield_in()?;    // == true
println!("3: in caller");
handle.yield_in()?;    // == false
println!("5: terminated");

// handle.is_terminated() == true
```

Features
--------

This example covers essentially the entire API.

You can share control of a single thread between one or more coroutine
execution contexts. Each coroutine yields back to the normal thread stack.

Non-features
------------

This crate does not have provisions for:

* Passing values into or out of the coroutine
* Coroutines that yield to other coroutines

Platform support
----------------

* Mac OS X (via `ucontext`)
* Linux (via `ucontext`)
* Other UNIXy systems (via `ucontext`)

Windows has could also support this same API. Pull requests welcome.

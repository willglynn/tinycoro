tinycoro
========

Tiny coroutines library, written in Rust.

This crate uses FFI bindings to coroutine-related platform features instead
of reimplementing coroutines in native Rust. This means that crate does
_not_ require nightly; you can use it today on stable Rust.

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
That's it.

Non-features
------------

This crate does not have provisions for:

* Passing values into or out of the coroutine
* Coroutines that yield to other coroutines

Platform support
----------------

Binding to `ucontext.h` lets `tinycoro` target:

* Linux (tested by Travis)
* Mac OS X (tested by Travis)
* Other UNIXy systems

`ucontext.h` specifies a POSIX standard C interface, but not standard
concrete types. This crate therefore uses `bindgen` as part of its build
process to generate Rust definitions matching the target system's C
definitions. This implies a requirement for a recent `clang`.

Windows has could also support this same user-facing API, but requires a
separate implementation. Pull requests welcome.

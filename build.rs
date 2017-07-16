extern crate bindgen;

use std::env;
use std::path::PathBuf;

fn main() {
    let bindings = bindgen::Builder::default()
        .header("src/ucontext.h")
        .whitelisted_type("ucontext_t")
        .whitelisted_function("setcontext")
        .whitelisted_function("getcontext")
        .whitelisted_function("makecontext")
        .whitelisted_function("swapcontext")
        .whitelisted_function("valloc")
        .whitelisted_function("free")
        .generate_inline_functions(true)
        .generate()
        .expect("generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("ucontext.rs"))
        .expect("write bindings");

    println!("wrote {:?}", out_path);
}

mod ucontext;

pub use ucontext::{Handle,Coroutine};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example() {
        let mut handle = Coroutine::new(|coro: &mut Coroutine| {
            println!("2: in coroutine");
            coro.yield_back();
            println!("4: in coroutine");
        });

        println!("1: in caller");
        handle.yield_in();
        println!("3: in caller");
        handle.yield_in();
        println!("5: terminated!");
        assert!(handle.is_terminated());
    }
}

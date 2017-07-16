mod ucontext;

pub use ucontext::{Handle,Coroutine};

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Result<(),()> {
        let mut handle = Coroutine::new(|coro: &mut Coroutine| {
            println!("2: in coroutine");
            coro.yield_back();
            println!("4: in coroutine");

        });
        assert!(!handle.is_terminated());

        println!("1: in caller");
        handle.yield_in()?;
        println!("3: in caller");
        handle.yield_in()?;
        println!("5: terminated!");

        assert!(handle.is_terminated());
        Ok(())
    }

    #[test]
    fn test_example() {
        example().expect("example");
    }
}

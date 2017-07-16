use std::mem;
use std::marker::PhantomData;
use std::cell::{Cell,RefCell};
use std::rc::Rc;
use std::os::raw::c_void;

// Include bindgen-created bindings, but pull in only the bits we need
mod sys;
use self::sys::{ucontext_t, getcontext, setcontext, makecontext, swapcontext};
use self::sys::{valloc, free};

struct Stack {
    size: usize,
    ptr: *mut c_void,
}

impl Stack {
    fn new(size: usize) -> Stack {
        let ptr = unsafe { valloc(size) };

        if ptr as usize == 0 {
            panic!("valloc() failed");
        }

        Stack{ size: size, ptr: ptr }
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        unsafe {
            free(self.ptr);
        }
    }
}

struct Ucontext {
    ctx: ucontext_t,
    stack: Option<Stack>,
    link: Option<Rc<Ucontext>>
}

trait UcontextFn {
    fn run_ucontext(&mut self);
}

unsafe extern "C" fn makecontext_target(ucontext_fn: usize) {
    let mut ucontext_fn: Box<Box<UcontextFn>> = Box::from_raw(ucontext_fn as _);
    ucontext_fn.run_ucontext();
}

impl Ucontext {
    // return a `Ucontext` representing the outside world
    fn new() -> Ucontext {
        let mut ctx: ucontext_t;

        unsafe {
            ctx = mem::zeroed();
        }

        Ucontext{
            ctx: ctx,
            stack: None,
            link: None,
        }
    }

    // return a `Ucontext` that will call `f` when switched to
    fn make<F>(f: F, stack_size: usize, link: Option<Rc<Ucontext>>) -> Ucontext
        where F: UcontextFn
    {
        let f: Box<UcontextFn> = Box::new(f);
        let stack = Stack::new(stack_size);

        let mut ctx: ucontext_t = unsafe { mem::zeroed() };

        unsafe {
            getcontext(&mut ctx as *mut ucontext_t);

            ctx.uc_stack.ss_sp = stack.ptr;
            ctx.uc_stack.ss_size = stack.size as _;
            ctx.uc_stack.ss_flags = 0;
            match &link {
                &Some(ref link) => {
                    ctx.uc_link = &link.ctx as *const ucontext_t as *mut ucontext_t;
                }
                &None => {
                    ctx.uc_link = 0 as _;
                }
            }

            let makecontext_target: unsafe extern "C" fn(usize) = makecontext_target;
            let makecontext_target: unsafe extern "C" fn() = mem::transmute(makecontext_target);
            makecontext(
                &mut ctx as *mut ucontext_t,
                Some(makecontext_target),
                1,
                Box::into_raw(Box::new(f)),
            );
        }

        Ucontext{
            ctx,
            stack: Some(stack),
            link: link,
        }
    }

    pub fn switch_to(&mut self, previous: &Ucontext) {
        unsafe {
            // ensure we return to previous if we don't switch elsewhere
            if swapcontext(&previous.ctx as *const ucontext_t as *mut ucontext_t, &self.ctx as *const ucontext_t) != 0 {
                // failed
                panic!("swapcontext failed");
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::atomic::{Ordering,AtomicUsize};

    #[test]
    fn test_ucontext() {
        let mut main = Rc::new(Ucontext::new());

        let seq = AtomicUsize::new(0);

        struct F1<'a>{ seq: &'a AtomicUsize };
        impl<'a> UcontextFn for F1<'a> {
            fn run_ucontext(&mut self) {
                // in coroutine (1 => 2)
                assert_eq!(self.seq.load(Ordering::Acquire), 1);
                self.seq.store(2, Ordering::Release);
            }
        }

        let mut coro = Ucontext::make(F1{seq: &seq}, 512*1024, Some(main.clone()));

        // sequence of events:

        // nothing (0 => 1)
        assert_eq!(seq.load(Ordering::Acquire), 0);
        seq.store(1, Ordering::Release);

        coro.switch_to(&main);

        // done (2!)
        assert_eq!(seq.load(Ordering::Acquire), 2);
    }
}
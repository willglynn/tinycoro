use std::mem;
use std::ptr;
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

// outside
struct Handle {
    ctx: ucontext_t,
    stack: Stack,
    link: Rc<Cell<Link>>,
}

// inside
struct Coroutine<'f> {
    f: Cell<Option<Box<UcontextFn + 'f>>>,
    link: Rc<Cell<Link>>,
}

#[derive(Copy,Clone)]
struct Link {
    left: *mut ucontext_t,
    right: *const ucontext_t,
}

impl Default for Link {
    fn default() -> Self {
        Link {
            left: 0 as _,
            right: 0 as _,
        }
    }
}

impl<'f> Coroutine<'f> {
    // return a `Ucontext` that will call `f` when switched to
    fn new<F>(f: F, stack_size: usize) -> Handle
        where F: UcontextFn + 'f
    {
        let stack = Stack::new(stack_size);

        let mut ctx: ucontext_t = unsafe { mem::zeroed() };

        // prepare a link, which we'll share with the ExecutionContext
        let link: Rc<Cell<Link>> = Rc::new(Cell::new(Link::default()));

        // prepare an Coroutine which we'll move into the context
        let coro = Coroutine {
            f: Cell::new(Some(Box::new(f))),
            link: link.clone(),
        };

        unsafe {
            getcontext(&mut ctx as *mut ucontext_t);

            ctx.uc_stack.ss_sp = stack.ptr;
            ctx.uc_stack.ss_size = stack.size as _;
            ctx.uc_stack.ss_flags = 0;
            ctx.uc_link = 0 as _;

            let executioncontext_entrypoint: unsafe extern "C" fn(*mut Coroutine) = executioncontext_entrypoint;
            let executioncontext_entrypoint: unsafe extern "C" fn() = mem::transmute(executioncontext_entrypoint);
            makecontext(
                &mut ctx as *mut ucontext_t,
                Some(executioncontext_entrypoint),
                1,
                Box::into_raw(Box::new(coro)),
            );
        }

        // assemble all the outer bits into the handle
        let mut handle = Handle {
            ctx,
            stack,
            link,
        };

        // at this point, we've leaked the Coroutine onto the heap
        // switch in so that executioncontext_entrypoint moves it to its internal stack
        handle.yield_in();

        handle
    }

    fn run(mut self) -> Link {
        let mut f = self.f.take().expect("f");
        f.run_ucontext(&mut self);

        return self.link.get();
    }

    fn yield_back(&mut self) {
        let link = self.link.get();
        unsafe {
            if swapcontext(link.left, link.right) != 0 {
                // failed
                panic!("swapcontext failed");
            }
        }
    }
}

trait UcontextFn {
    fn run_ucontext(&mut self, execution_context: &mut Coroutine);
}

unsafe extern "C" fn executioncontext_entrypoint(
    execution_context: *mut Coroutine
) {
    let link: Link;

    {
        // move in the incoming ExecutionContext ptr
        let mut execution_context: Coroutine = ptr::read(execution_context);

        // yield back, letting the constructor return
        execution_context.yield_back();

        // run the execution context until it returns our final link
        link = execution_context.run();

        // drop the execution context
    }

    println!("setcontext()ing back");
    setcontext(link.right);
    panic!("setcontext() failed");
}

impl Handle {
    pub fn yield_in(&mut self) {
        unsafe {
            // set the link to come back here
            let here: ucontext_t = mem::uninitialized();
            self.link.set(Link{
                left: &self.ctx as *const ucontext_t as _,
                right: &here as *const ucontext_t as _,
            });

            // swap in
            if swapcontext(&here as *const ucontext_t as *mut ucontext_t, &self.ctx as *const ucontext_t) != 0 {
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
        let seq = AtomicUsize::new(0);

        struct F1<'a>{ seq: &'a AtomicUsize };
        impl<'a> UcontextFn for F1<'a> {
            fn run_ucontext(&mut self, execution_context: &mut Coroutine) {
                // in coroutine (1 => 2)
                assert_eq!(self.seq.load(Ordering::Acquire), 1);
                self.seq.store(2, Ordering::Release);

                execution_context.yield_back();

                // back in coroutine (3 => 4)
                assert_eq!(self.seq.load(Ordering::Acquire), 3);
                self.seq.store(4, Ordering::Release);
            }
        }

        let mut coro = Coroutine::new(F1{seq: &seq}, 512*1024);

        // sequence of events:

        // nothing (0 => 1)
        assert_eq!(seq.load(Ordering::Acquire), 0);
        seq.store(1, Ordering::Release);

        coro.yield_in();

        // back from coroutine (2 => 3)
        assert_eq!(seq.load(Ordering::Acquire), 2);
        seq.store(3, Ordering::Release);

        coro.yield_in();

        // done (4!)
        assert_eq!(seq.load(Ordering::Acquire), 4);
    }
}
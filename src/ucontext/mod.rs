use std::mem;
use std::ptr;
use std::marker::PhantomData;
use std::cell::Cell;
use std::rc::Rc;
use std::os::raw::c_void;

// Include bindgen-created bindings, but pull in only the bits we need
mod sys;
use self::sys::{ucontext_t, getcontext, setcontext, makecontext, swapcontext};
use self::sys::{valloc, free};

// `Stack` is a page-aligned region of memory suitable for use as a coroutine's stack.
//
// It's allocated using C `valloc()` and dropped using C `free()`.
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

/// A `Handle` is created for the outside of an asymmetric coroutine. It contains the suspended
/// coroutine's thread state and the coroutine's stack.
pub struct Handle<'f> {
    ctx: ucontext_t,

    #[allow(dead_code)]
    stack: Stack, // Rust doesn't see the stack get used, but it is referenced by `ctx`.

    link: Rc<Cell<Link>>,
    pd: PhantomData<&'f ()>
}

/// A `Coroutine` is created for the inside of an asymmetric coroutine's execution.
pub struct Coroutine<'f> {
    f: Cell<Option<Box<FnMut(&mut Coroutine) + 'f>>>,
    link: Rc<Cell<Link>>,
}

#[derive(Copy,Clone,PartialEq,Eq)]
enum Link {
    Ready,
    Called {
        left: *mut ucontext_t,
        right: *const ucontext_t,
    },
    Terminated
}

impl<'f> Coroutine<'f> {
    pub fn new<F>(f: F) -> Handle<'f>
        where F: FnMut(&mut Coroutine) + 'f
    {
        Self::new_with_stack_size(f, 512*1024)
    }


    pub fn new_with_stack_size<F>(f: F, stack_size: usize) -> Handle<'f>
        where F: FnMut(&mut Coroutine) + 'f
    {
        let stack = Stack::new(stack_size);

        let mut ctx: ucontext_t = unsafe { mem::zeroed() };

        // prepare a link, which we'll share with the ExecutionContext
        let link: Rc<Cell<Link>> = Rc::new(Cell::new(Link::Ready));

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
            pd: PhantomData,
        };

        // at this point, we've leaked the Coroutine onto the heap
        // switch in so that executioncontext_entrypoint moves it to its internal stack
        handle.yield_in();

        handle
    }

    fn run(mut self) -> Rc<Cell<Link>> {
        let mut f = self.f.take().expect("callback function");
        f(&mut self);

        return self.link;
    }

    pub fn yield_back(&mut self) {
        let link = Cell::new(Link::Ready);
        self.link.swap(&link);
        match link.into_inner() {
            Link::Called { left, right } => {
                unsafe {
                    if swapcontext(left, right) != 0 {
                        // failed
                        panic!("swapcontext failed");
                    }
                }
            }
            _ => {
                panic!("don't know where to yield back to");
            }
        }
    }
}

unsafe extern "C" fn executioncontext_entrypoint(
    execution_context: *mut Coroutine
) {
    let link_cell: Rc<Cell<Link>>;

    {
        // move in the incoming ExecutionContext ptr
        let mut execution_context: Coroutine = ptr::read(execution_context);

        // yield back, letting the constructor return
        execution_context.yield_back();

        // run the execution context until it returns our final link
        link_cell = execution_context.run();

        // drop the execution context
    }

    let link = Cell::new(Link::Terminated);
    link_cell.swap(&link);
    match link.into_inner() {
        Link::Called { left: _, right } => {
            setcontext(right);
            panic!("setcontext() failed");
        }
        _ => {
            panic!("coroutine is complete but cannot return to caller");
        }
    }
}

impl<'f> Handle<'f> {
    pub fn is_terminated(&self) -> bool {
        match self.link.get() {
            Link::Terminated => true,
            _ => false,
        }
    }

    pub fn yield_in(&mut self) {
        match self.link.get() {
            Link::Terminated => {
                return;
            }
            _ => ()
        }

        unsafe {
            // set the link to come back here
            let here: ucontext_t = mem::uninitialized();
            self.link.set(Link::Called{
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

        let mut coro = Coroutine::new(|coro: &mut Coroutine| {
            // in coroutine (1 => 2)
            assert_eq!(seq.load(Ordering::Acquire), 1);
            seq.store(2, Ordering::Release);

            coro.yield_back();

            // back in coroutine (3 => 4)
            assert_eq!(seq.load(Ordering::Acquire), 3);
            seq.store(4, Ordering::Release);
        });

        // sequence of events:

        // nothing (0 => 1)
        assert_eq!(seq.load(Ordering::Acquire), 0);
        assert_eq!(coro.is_terminated(), false);
        seq.store(1, Ordering::Release);

        coro.yield_in();

        // back from coroutine (2 => 3)
        assert_eq!(seq.load(Ordering::Acquire), 2);
        assert_eq!(coro.is_terminated(), false);
        seq.store(3, Ordering::Release);

        coro.yield_in();

        // done (4!)
        assert_eq!(seq.load(Ordering::Acquire), 4);
        assert_eq!(coro.is_terminated(), true);
    }
}
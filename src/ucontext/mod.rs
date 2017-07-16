use std::mem;
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

/// A `Handle` is created for the outside of a coroutine. It contains the coroutine's thread state
/// and the coroutine's stack.
///
/// # Safety
///
/// It's probably not a good idea to drop the `Handle` while the coroutine is running.
pub struct Handle<'f> {
    ctx: ucontext_t,

    #[allow(dead_code)]
    stack: Stack, // Rust doesn't see the stack get used, but it is referenced by `ctx`.

    link: Rc<Cell<Link>>,
    pd: PhantomData<&'f ()>
}

/// A `Coroutine` is created for the inside of a coroutine. It allows the coroutine to
/// `yield_back()` control to its caller.
pub struct Coroutine<'f> {
    link: Rc<Cell<Link>>,
    pd: PhantomData<&'f ()>
}

/// `Link` encapsulates the communication of shared state between `Coroutine` and `Handle`.
#[derive(Copy,Clone,PartialEq,Eq)]
enum Link {
    /// Indicates the coroutine is ready to be called
    Ready,

    /// Indicates the coroutine _was_ called, and provides ucontext_t's to which it should return.
    Called {
        left: *mut ucontext_t,
        right: *const ucontext_t,
    },

    /// Indicates the coroutine is terminated and must not be called again.
    Terminated
}

impl<'f> Coroutine<'f> {
    /// Create a new `Coroutine`+`Handle` with a default stack size.
    ///
    /// The coroutine will call `f(&mut Coroutine)` when it starts.
    pub fn new<F>(f: F) -> Handle<'f>
        where F: FnOnce(&mut Coroutine) + 'f
    {
        Self::new_with_stack_size(f, 512*1024)
    }

    /// Create a new `Coroutine`+`Handle` with a specific stack size.
    ///
    /// The coroutine will call `f(&mut Coroutine)` when it starts.
    pub fn new_with_stack_size<F>(f: F, stack_size: usize) -> Handle<'f>
        where F: FnOnce(&mut Coroutine) + 'f
    {
        let stack = Stack::new(stack_size);

        let mut ctx: ucontext_t = unsafe { mem::zeroed() };

        // prepare a link, which we'll share between the Handle and the Coroutine
        let link: Rc<Cell<Link>> = Rc::new(Cell::new(Link::Ready));

        // prepare a Coroutine
        let coro = Coroutine {
            link: link.clone(),
            pd: PhantomData
        };

        // wrap `f` and `coro` into Option<_>s
        let mut coro: Option<Coroutine> = Some(coro);
        let mut callback: Option<F> = Some(f);

        // define a polymorphic C entrypoint suitable for this <F>
        //
        // the entrypoint takes pointers to the locals above, which obviously has lifetime issues
        // therefore, the strategy is to:
        //   - put coroutine data into locals on the constructor's stack
        //   - yield into the coroutine
        //   - have the entrypoint move data into the coroutine stack
        //   - yield back
        //   - return from the constructor
        unsafe extern "C" fn entrypoint<F>(coro: *mut Option<Coroutine>, f: *mut Option<F>)
            where F: FnOnce(&mut Coroutine)
        {
            // take the constructor's Coroutine
            let mut coro: Coroutine =
                mem::transmute::<*mut Option<Coroutine>, &mut Option<Coroutine>>(coro)
                .take()
                .unwrap();

            {
                // take the constructor's function
                let f: F =
                    mem::transmute::<*mut Option<F>, &mut Option<F>>(f)
                        .take()
                        .unwrap();

                // yield back, letting the constructor return
                coro.yield_back();

                // run the function
                f(&mut coro);

                // drop the function
            }

            // terminate the coroutine
            coro.terminate();
        }

        // express this plan in terms of <ucontext.h> API
        unsafe {
            // initialize the context from right here, right now
            getcontext(&mut ctx as *mut ucontext_t);

            // point it to the new stack
            ctx.uc_stack.ss_sp = stack.ptr;
            ctx.uc_stack.ss_size = stack.size as _;
            ctx.uc_stack.ss_flags = 0;
            ctx.uc_link = 0 as _;

            // makecontext() takes a C function() and separately asks for args
            // this means we need to transmute into a no-arg function
            let entrypoint: unsafe extern "C" fn(*mut Option<Coroutine>, *mut Option<F>) = entrypoint;
            let entrypoint: unsafe extern "C" fn() = mem::transmute(entrypoint);

            // have the context run entrypoint(&mut coro, &mut f) when it starts
            makecontext(
                &mut ctx as *mut ucontext_t,
                Some(entrypoint),
                2,
                &mut coro as *mut Option<Coroutine>,
                &mut callback as *mut Option<F>,
            );
        }

        // assemble all the outer bits into the handle
        let mut handle = Handle {
            ctx,
            stack,
            link,
            pd: PhantomData,
        };

        // at this point, the Handle's `ctx` is ready to invoke entrypoint() with pointers to our
        // local `coro` and `f` Option<_>s.
        //
        // yield into the coroutine, let it take the values out of `coro` and `f`, and yield back.
        handle.yield_in().unwrap();

        // the coroutine is now ready to invoke the user's function, and it shares no state except
        // `link`.
        //
        // return to caller
        handle
    }

    /// Yield control back to the caller. `Coroutine::yield_back()` will block until the caller
    /// calls `Handle::yield_in()`.
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

    /// Terminate the coroutine.
    ///
    /// # Safety
    ///
    /// Never returns.
    unsafe fn terminate(&mut self) {
        let link = Cell::new(Link::Terminated);
        self.link.swap(&link);
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
}

impl<'f> Handle<'f> {
    /// Indicates whether or not the coroutine has terminated.
    pub fn is_terminated(&self) -> bool {
        match self.link.get() {
            Link::Terminated => true,
            _ => false,
        }
    }

    /// Yield control into the coroutine. This function blocks until either the coroutine calls
    /// `Coroutine::yield_back()` or returns.
    ///
    /// Returns `Ok(still_running: bool)` on success, or `Err(())` if the coroutine could not be
    /// called because it has already terminated.
    pub fn yield_in(&mut self) -> Result<bool, ()> {
        if self.is_terminated() {
            return Err(());
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

        if self.is_terminated() {
            // terminated
            // TODO: free stack early?
            Ok(false)

        } else {
            // still running
            Ok(true)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::atomic::{Ordering,AtomicUsize};

    #[test]
    fn test_create_destroy() {
        let seq = AtomicUsize::new(0);

        {
            // create a coroutine
            let mut coro = Coroutine::new(|_: &mut Coroutine| {
                seq.store(1, Ordering::Release);
            });

            // don't ever actually call it
            if false {
                coro.yield_in().unwrap();
            }

            // dropping without calling should not be an error
        }

        // ...and the value should remain unchanged
        assert_eq!(seq.load(Ordering::Acquire), 0);
    }

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

        coro.yield_in().unwrap();

        // back from coroutine (2 => 3)
        assert_eq!(seq.load(Ordering::Acquire), 2);
        assert_eq!(coro.is_terminated(), false);
        seq.store(3, Ordering::Release);

        coro.yield_in().unwrap();

        // done (4!)
        assert_eq!(seq.load(Ordering::Acquire), 4);
        assert_eq!(coro.is_terminated(), true);
    }
}
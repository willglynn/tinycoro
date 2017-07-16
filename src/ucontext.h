#ifdef __APPLE__
#define _XOPEN_SOURCE
#endif

// we need:
//   ucontext_t, setcontext(), getcontext(), swapcontext(), makecontext()
#include <ucontext.h>

// we need:
//   valloc(), free()
#include <stdlib.h>
// Authoring-only support for a single meshoptimizer call on x86-64 Linux.
// The Python-owned aligned arena is discarded in one piece after the call.
using size_t = __SIZE_TYPE__;
static unsigned char* next = nullptr;
static size_t left = 0;

extern "C" void test_boat_arena(void* memory, size_t size)
{
    size_t skip = (0 - reinterpret_cast<__UINTPTR_TYPE__>(memory)) & 15;
    next = static_cast<unsigned char*>(memory) + skip;
    left = size - skip;
}

void* operator new(size_t n)
{
    n = (n + 15) & ~size_t(15);
    if (!next || n > left)
        __builtin_trap();
    void* result = next;
    next += n;
    left -= n;
    return result;
}

void operator delete(void*) noexcept {}

extern "C" void* memcpy(void* dst, const void* src, size_t n) noexcept
{
    auto* d = static_cast<unsigned char*>(dst);
    const auto* s = static_cast<const unsigned char*>(src);
    for (size_t i = 0; i < n; ++i)
        d[i] = s[i];
    return dst;
}

extern "C" void* memset(void* dst, int c, size_t n) noexcept
{
    auto* d = static_cast<unsigned char*>(dst);
    for (size_t i = 0; i < n; ++i)
        d[i] = static_cast<unsigned char>(c);
    return dst;
}

extern "C" float sqrtf(float x) noexcept
{
    float y;
    asm("sqrtss %1, %0" : "=x"(y) : "x"(x));
    return y;
}

extern "C" float fabsf(float x) noexcept
{
    return __builtin_fabsf(x);
}

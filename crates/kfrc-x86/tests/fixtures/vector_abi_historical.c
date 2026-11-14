#include <stdint.h>
#include <stdio.h>
#include <emmintrin.h>
#define PARAMS __m128i v0,uint64_t a0,__m128i v1,uint64_t a1,__m128i v2,uint64_t a2,__m128i v3,uint64_t a3,__m128i v4,uint64_t a4,__m128i v5,uint64_t a5,__m128i v6,uint64_t a6,__m128i v7,__m128i v8,uint64_t a7,__m128i v9
#define ARGS v0,a0,v1,a1,v2,a2,v3,a3,v4,a4,v5,a5,v6,a6,v7,v8,a7,v9
typedef __m128i (*callback)(PARAMS);
typedef struct { uint64_t x,y,z; } Triple;
extern __m128i vector_forward(PARAMS), vector_indirect(callback,PARAMS), vector_pick_stack(PARAMS);
extern uint64_t vector_scalar(PARAMS);
extern Triple vector_sret(PARAMS);
extern __m128i vector_extended_spill(__m128i);
static uint64_t rotate(uint64_t x,unsigned bits) { return (x<<bits)|(x>>(64-bits)); }
__attribute__((noinline)) __m128i c_vector_mix(PARAMS) {
    __m128i vectors[] = {v0,v1,v2,v3,v4,v5,v6,v7,v8,v9};
    uint64_t integers[] = {a0,a1,a2,a3,a4,a5,a6,a7};
    uint64_t low = 0xabcdef1234567890ULL, high = 0x123456789abcdef0ULL;
    for (unsigned i=0;i<10;i++) {
        uint64_t lanes[2]; _mm_storeu_si128((__m128i *)lanes,vectors[i]);
        low += lanes[0]*(i+3); high ^= rotate(lanes[1],i+1);
    }
    for (unsigned i=0;i<8;i++) { low ^= integers[i]*(i+11); high += rotate(integers[i],i+3); }
    return _mm_set_epi64x((long long)high,(long long)low);
}
__attribute__((noinline)) uint64_t c_vector_scalar(PARAMS) {
    uint64_t lanes[2]; _mm_storeu_si128((__m128i *)lanes,c_vector_mix(ARGS));
    return lanes[0]^lanes[1];
}
__attribute__((noinline)) Triple c_vector_sret(PARAMS) {
    uint64_t lanes[2]; _mm_storeu_si128((__m128i *)lanes,c_vector_mix(ARGS));
    return (Triple){lanes[0],lanes[1],a6+7*a7};
}
static int equal(__m128i a,__m128i b) { return _mm_movemask_epi8(_mm_cmpeq_epi8(a,b))==65535; }
int main(void) {
    for (uint64_t seed=0;seed<256;seed++) {
        __m128i v0=_mm_set_epi64x(~seed,seed),v1=_mm_set_epi64x(seed*3+1,seed*5+2),
            v2=_mm_set_epi64x(seed*7+3,seed*11+4),v3=_mm_set_epi64x(seed*13+5,seed*17+6),
            v4=_mm_set_epi64x(seed*19+7,seed*23+8),v5=_mm_set_epi64x(seed*29+9,seed*31+10),
            v6=_mm_set_epi64x(seed*37+11,seed*41+12),v7=_mm_set_epi64x(seed*43+13,seed*47+14),
            v8=_mm_set_epi64x(seed*53+15,seed*59+16),v9=_mm_set_epi64x(seed*61+17,seed*67+18);
        uint64_t a0=seed+1,a1=seed+3,a2=seed+5,a3=seed+7,a4=seed+11,a5=seed+13,a6=seed+17,a7=seed+19;
        __m128i first=c_vector_mix(ARGS);
        __m128i expected=c_vector_mix(first,a0,v1,a1,v2,a2,v3,a3,v4,a4,v5,a5,v6,a6,v7,v8,a7,v9);
        if (!equal(vector_forward(ARGS),expected)) return 1;
        if (!equal(vector_indirect(c_vector_mix,ARGS),expected)) return 2;
        if (!equal(vector_pick_stack(ARGS),v9)) return 3;
        if (vector_scalar(ARGS)!=c_vector_scalar(ARGS)) return 4;
        Triple actual=vector_sret(ARGS), wanted=c_vector_sret(ARGS);
        if (actual.x!=wanted.x || actual.y!=wanted.y || actual.z!=wanted.z) return 5;
        if (!equal(vector_extended_spill(v0),v0)) return 6;
    }
    puts("SysV XMM/mixed/spilled/callback/sret ABI PASS");
    return 0;
}

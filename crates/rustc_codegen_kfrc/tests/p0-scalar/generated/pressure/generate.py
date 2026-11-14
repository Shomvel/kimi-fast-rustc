#!/usr/bin/env python3
"""Deterministic defined-input stress for allocation, Phi cycles and fixed regs."""
from pathlib import Path
import hashlib,json
import sys
out=Path(sys.argv[1]);out.mkdir(parents=True,exist_ok=True)
s=['#![no_std]','extern "C" { fn clobber(p: *mut u64, salt: u64) -> u64; fn eight(a:u64,b:u64,c:u64,d:u64,e:u64,f:u64,g:u64,h:u64)->u64; }']
s+=['#[no_mangle] pub unsafe extern "C" fn pressure(p:*mut u64, seed:u64) -> u64 {']
for i in range(24):s+=[f' let x{i}=*p.add({i});']
s+=[' let r=clobber(p,seed);',' let mut z=r;']
for i in range(24):s+=[f' z=(z ^ (x{i} << {i%17})).wrapping_add(x{i} >> {(i*7)%19});']
s+=[' z','}']
s+=['#[no_mangle] pub unsafe extern "C" fn phi_cycles(p:*mut u64, seed:u64, count:u64) -> u64 {']
for i in range(16):s+=[f' let mut x{i}=*p.add({i});']
s+=[' let mut z=seed; let mut i=0u64; while i<count {',' z=z.wrapping_add(clobber(p,z));']
s+=[' if i & 1 == 0 {',' let t=x0;']+[f' x{i}=x{i+1};' for i in range(15)]+[' x15=t;',' } else {',' let t=x15;']+[f' x{i}=x{i-1};' for i in range(15,0,-1)]+[' x0=t;',' }',' x0=x0.wrapping_add(z); x7=x7 ^ (z >> (i & 63)); i+=1;',' }']
for i in range(16):s+=[f' z=(z ^ x{i}).wrapping_mul({2*i+3});']
s+=[' z','}']
s+=['#[no_mangle] pub unsafe extern "C" fn fixed_pressure(p:*mut u64, seed:u64) -> u64 {']
for i in range(16):s+=[f' let x{i}=*p.add({i});']
s+=[' let shift=(seed & 63) as u32; let den=(seed & 255)+1; let r=clobber(p,seed);',' let mut z=r;']
for i in range(16):s+=[f' z=z.wrapping_add((x{i} / den) ^ (x{i} % den) ^ (x{i} << shift) ^ (x{i} >> shift));']
s+=[' z','}']
s+=['#[no_mangle] pub unsafe extern "C" fn arg_cycle(a:u64,b:u64,c:u64,d:u64,e:u64,f:u64,g:u64,h:u64,seed:u64)->u64 {',' let x=eight(h,g,f,e,d,c,b,a);',' let y=eight(b,c,d,e,f,g,h,a);',' (x ^ y).wrapping_add(a ^ b ^ c ^ d ^ e ^ f ^ g ^ h ^ seed)','}']
(out/'lib.rs').write_text('\n'.join(s)+'\n')
(out/'driver.c').write_text(r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
extern uint64_t pressure(uint64_t*,uint64_t);
extern uint64_t phi_cycles(uint64_t*,uint64_t,uint64_t);
extern uint64_t fixed_pressure(uint64_t*,uint64_t);
extern uint64_t arg_cycle(uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t,uint64_t);
__attribute__((noinline)) uint64_t clobber(uint64_t *p,uint64_t salt) {
 for(unsigned i=0;i<24;i++) p[i]=salt ^ (UINT64_C(0xaabbccdd00112233)+i);
 __asm__ volatile("xor %%eax,%%eax; xor %%ecx,%%ecx; xor %%edx,%%edx; xor %%esi,%%esi; xor %%edi,%%edi; xor %%r8d,%%r8d; xor %%r9d,%%r9d; xor %%r10d,%%r10d; xor %%r11d,%%r11d" ::: "rax","rcx","rdx","rsi","rdi","r8","r9","r10","r11","memory","cc");
 return salt * 17 + 31;
}
__attribute__((noinline)) uint64_t eight(uint64_t a,uint64_t b,uint64_t c,uint64_t d,uint64_t e,uint64_t f,uint64_t g,uint64_t h) {
 return a*3+b*5+c*7+d*11+e*13+f*17+g*19+h*23;
}
static void init(uint64_t *p,uint64_t seed) {
 for(unsigned i=0;i<24;i++) p[i]=(seed*UINT64_C(0x123456789abcdef)+i*UINT64_C(0xfedcba987654321)) ^ (UINT64_C(1) << ((seed+i)&63));
}
static uint64_t reference_pressure(uint64_t *x,uint64_t seed) {
 uint64_t z=seed*17+31;
 for(unsigned i=0;i<24;i++)z=(z^(x[i]<<(i%17)))+(x[i]>>((i*7)%19));return z;
}
static uint64_t reference_phi(uint64_t *x,uint64_t seed,uint64_t n) {
 uint64_t z=seed;
 for(unsigned i=0;i<n;i++) {
  z+=z*17+31;
  if((i&1)==0){uint64_t t=x[0];for(unsigned j=0;j<15;j++)x[j]=x[j+1];x[15]=t;}
  else{uint64_t t=x[15];for(unsigned j=15;j>0;j--)x[j]=x[j-1];x[0]=t;}
  x[0]+=z;x[7]^=z>>(i&63);
 }
 for(unsigned i=0;i<16;i++)z=(z^x[i])*(2*i+3);return z;
}
static uint64_t reference_fixed(uint64_t *x,uint64_t seed) {
 uint64_t z=seed*17+31,den=(seed&255)+1;unsigned shift=seed&63;
 for(unsigned i=0;i<16;i++)z+=(x[i]/den)^(x[i]%den)^(x[i]<<shift)^(x[i]>>shift);return z;
}
int pressure_main(void) {
 for(uint64_t seed=0;seed<512;seed++){
  uint64_t a[24],b[24];init(a,seed);init(b,seed);
  uint64_t want=reference_pressure(b,seed),got=pressure(a,seed);if(want!=got)return fprintf(stderr,"pressure %lu\n",seed),1;
  for(unsigned i=0;i<24;i++)if(a[i]!=(seed^(UINT64_C(0xaabbccdd00112233)+i)))return 2;
  printf("p %lu %lu\n",seed,got);
  init(a,seed);init(b,seed);want=reference_phi(b,seed,seed%19);got=phi_cycles(a,seed,seed%19);if(want!=got)return fprintf(stderr,"phi %lu\n",seed),3;
  printf("c %lu %lu\n",seed,got);
  init(a,seed);init(b,seed);want=reference_fixed(b,seed);got=fixed_pressure(a,seed);if(want!=got)return fprintf(stderr,"fixed %lu\n",seed),4;
  printf("f %lu %lu\n",seed,got);
  want=eight(b[7],b[6],b[5],b[4],b[3],b[2],b[1],b[0])^eight(b[1],b[2],b[3],b[4],b[5],b[6],b[7],b[0]);
  want+=b[0]^b[1]^b[2]^b[3]^b[4]^b[5]^b[6]^b[7]^seed;
  got=arg_cycle(b[0],b[1],b[2],b[3],b[4],b[5],b[6],b[7],seed);if(want!=got)return fprintf(stderr,"args %lu\n",seed),5;
  printf("a %lu %lu\n",seed,got);
 }
 return 0;
}
''')
(out/'driver.rs').write_text('extern crate pressure;\nextern \"C\" { fn pressure_main() -> i32; }\nfn main() { let status=unsafe { pressure_main() }; assert_eq!(status,0); }\n')
(out/'inputs.json').write_text(json.dumps({'seed_range':[0,511],'vectors':2048,'functions':['pressure','phi_cycles','fixed_pressure','arg_cycle'],'definedness':'24-element valid buffers; shifts masked 0..63; unsigned wrapping; denominator1..256; loops0..18; extern clobber writes within buffer','hashes':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [out/'lib.rs',out/'driver.c',out/'driver.rs']}},indent=2)+'\n')

use super::*;
use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
use std::process::Command;

fn width(f: &mut Fixture, bytes: u64) -> TyId {
    f.graph.oracle.intern(
        TypeShape::Int {
            width: match bytes {
                1 => IntegerWidth::W8,
                2 => IntegerWidth::W16,
                4 => IntegerWidth::W32,
                _ => IntegerWidth::W64,
            },
            signed: false,
        },
        Some(SizeAlign {
            size: bytes,
            align: bytes,
        }),
    )
}

fn load_or_store(bytes: u64, store: bool, immediate: bool) -> MachineFunction {
    let mut f = Fixture::new(
        &format!(
            "sib_{}{}",
            if !store {
                "load"
            } else if immediate {
                "imm"
            } else {
                "store"
            },
            bytes
        ),
        3,
    );
    let ty = width(&mut f, bytes);
    let index = f.scaled(bytes as i64, bytes == 4);
    let address = f.address(index, -17);
    if store {
        let value = if immediate {
            let bits = u128::MAX >> (128 - bytes * 8);
            f.graph.add_constant(((-7i128) as u128) & bits, ty)
        } else {
            f.cast(f.args[2], ty)
        };
        f.store(address, value);
    }
    let value = f.load(address, ty);
    let value = f.cast(value, f.word);
    let function = f.finish(value);
    assert_eq!(
        indexed_loads(&function),
        [(
            match bytes {
                1 => Scale::One,
                2 => Scale::Two,
                4 => Scale::Four,
                _ => Scale::Eight,
            },
            -17
        )]
    );
    if store {
        assert!(function.instructions.iter().any(|i| if immediate {
            matches!(i, MachineInstr::StoreAddressImmediate { .. })
        } else {
            matches!(i, MachineInstr::StoreAddress { .. })
        }));
    }
    function
}

fn multiuse(pointer: bool) -> MachineFunction {
    let mut f = Fixture::new(
        if pointer {
            "sib_pointer_data"
        } else {
            "sib_index_data"
        },
        2,
    );
    let index = f.scaled(4, false);
    let address = f.address(index, 16);
    f.store(f.args[0], if pointer { address } else { index });
    let value = f.load(address, f.word);
    f.finish(value)
}

fn pressure() -> MachineFunction {
    let mut f = Fixture::new("sib_pressure", 3);
    let index = f.scaled(8, false);
    let address = f.address(index, 0);
    let mut values = Vec::new();
    for offset in 0..24 {
        let address = if offset == 0 {
            address
        } else {
            let displacement = f.constant(offset * 8);
            f.graph.add_node(
                Opcode::PtrOffset,
                NodeId::NONE,
                &[address, displacement],
                0,
                ValueType::Rust(f.pointer),
            )
        };
        values.push(f.load(address, f.word));
    }
    let target = f.graph.add_callee(CallTarget {
        symbol: "sib_clobber".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Ignore,
            c_variadic: false,
        },
    });
    let call = f.graph.add_node(
        Opcode::Call,
        f.control,
        &[f.effect, f.args[0]],
        target,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    f.control = projection(&mut f.graph, call, 0, ValueType::Control);
    f.effect = projection(&mut f.graph, call, 1, ValueType::Effect);
    f.store(address, f.args[2]);
    let mut result = f.load(address, f.word);
    for value in values {
        // The anchor deliberately keeps all loaded values live over the call.
        result = f.graph.add_node(
            Opcode::Add,
            f.control,
            &[result, value],
            0,
            ValueType::Rust(f.word),
        );
    }
    let function = f.finish(result);
    assert!(function.stack_slots > 0, "must exercise spills");
    assert_eq!(indexed_loads(&function).len(), 25);
    function
}

#[test]
fn encoded_indexed_loads_stores_wrapping_multiuse_and_call_pressure_match_c() {
    let mut functions = vec![narrow_index(), multiuse(false), multiuse(true), pressure()];
    for bytes in [1, 2, 4, 8] {
        functions.extend([
            load_or_store(bytes, false, false),
            load_or_store(bytes, true, false),
            load_or_store(bytes, true, true),
        ]);
    }
    let object = write_object(
        &functions
            .into_iter()
            .map(|function| FunctionCode {
                function,
                binding: SymbolBinding::Global { hidden: false },
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "kfrc-sib-native-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("sib.o"), object).unwrap();
    std::fs::write(dir.join("driver.c"), r#"
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#define DECL(N) \
extern uint64_t sib_load##N(uint8_t*,uint64_t,uint64_t); \
extern uint64_t sib_store##N(uint8_t*,uint64_t,uint64_t); \
extern uint64_t sib_imm##N(uint8_t*,uint64_t,uint64_t);
DECL(1) DECL(2) DECL(4) DECL(8)
extern uint64_t sib_narrow(uint8_t*,uint64_t);
extern uint64_t sib_index_data(uint8_t*,uint64_t);
extern uint64_t sib_pointer_data(uint8_t*,uint64_t);
extern uint64_t sib_pressure(uint8_t*,uint64_t,uint64_t);
static volatile unsigned calls;
__attribute__((noinline)) void sib_clobber(uint8_t *p) {
  calls += p[0] + 1;
  __asm__ volatile("xor %%rax,%%rax; xor %%rcx,%%rcx; xor %%rdx,%%rdx; xor %%rsi,%%rsi; xor %%rdi,%%rdi; xor %%r8,%%r8; xor %%r9,%%r9; xor %%r10,%%r10; xor %%r11,%%r11"
    ::: "rax","rcx","rdx","rsi","rdi","r8","r9","r10","r11","cc");
}
static void fill(uint8_t *p, unsigned seed) {
  for (unsigned j=0;j<4096;j++) p[j]=(uint8_t)(j*73+seed*19);
}
static uint64_t read_word(const uint8_t *p, unsigned bytes) {
  uint64_t value=0; memcpy(&value,p,bytes); return value;
}
int main(void) {
  uint8_t actual[4096], expected[4096];
  typedef uint64_t (*fn)(uint8_t*,uint64_t,uint64_t);
  fn loads[]={sib_load1,sib_load2,sib_load4,sib_load8};
  fn stores[]={sib_store1,sib_store2,sib_store4,sib_store8};
  fn immediates[]={sib_imm1,sib_imm2,sib_imm4,sib_imm8};
  for (unsigned seed=0;seed<64;seed++) for(unsigned lane=0;lane<4;lane++) {
    unsigned bytes=1u<<lane;
    for(unsigned k=0;k<8;k++) {
      uint64_t index=k;
      if (lane && (seed&1)) index += UINT64_C(1)<<(64-lane);
      int64_t offset=(int64_t)(index*bytes-17);
      fill(actual,seed); memcpy(expected,actual,sizeof(actual));
      uint64_t want=read_word(expected+1024+offset,bytes);
      if (loads[lane](actual+1024,index,0)!=want || memcmp(actual,expected,sizeof(actual))) return 1;
      uint64_t value=UINT64_C(0x9abcdeff12345678)^seed;
      memcpy(expected+1024+offset,&value,bytes);
      want=read_word(expected+1024+offset,bytes);
      if(stores[lane](actual+1024,index,value)!=want || memcmp(actual,expected,sizeof(actual))) return 2;
      value=(uint64_t)-7; memcpy(expected+1024+offset,&value,bytes);
      want=read_word(expected+1024+offset,bytes);
      if(immediates[lane](actual+1024,index,0)!=want || memcmp(actual,expected,sizeof(actual))) return 3;
    }
  }
  uint64_t narrow[]={0,3,UINT64_C(0x40000003),UINT64_C(0x80000005),UINT64_C(0x100000007)};
  for(unsigned j=0;j<sizeof(narrow)/sizeof(*narrow);j++) {
    fill(actual,j); memcpy(expected,actual,sizeof(actual));
    uint64_t offset=(uint32_t)((uint32_t)narrow[j]*4u);
    if(sib_narrow(actual+1024,narrow[j])!=read_word(expected+1024+offset,8) || memcmp(actual,expected,sizeof(actual))) return 4;
  }
  for(unsigned i=0;i<32;i++) for(unsigned pointer=0;pointer<2;pointer++) {
    fill(actual,i); memcpy(expected,actual,sizeof(actual));
    uint64_t stored=pointer?(uintptr_t)(actual+1024+i*4+16):i*4;
    memcpy(expected+1024,&stored,8);
    uint64_t want=read_word(expected+1024+i*4+16,8);
    uint64_t got=pointer?sib_pointer_data(actual+1024,i):sib_index_data(actual+1024,i);
    if(got!=want || memcmp(actual,expected,sizeof(actual))) return 5;
  }
  for(unsigned i=0;i<32;i++) {
    fill(actual,i); memcpy(expected,actual,sizeof(actual));
    uint64_t value=UINT64_C(0x876543210fedcba9)+i, want=value;
    for(unsigned j=0;j<24;j++) want+=read_word(expected+1024+(i+j)*8,8);
    memcpy(expected+1024+i*8,&value,8);
    if(sib_pressure(actual+1024,i,value)!=want || memcmp(actual,expected,sizeof(actual))) return 6;
  }
  if(!calls) return 7;
  puts("indexed loads/stores/wrapping/multiuse/call-pressure PASS");
  return 0;
}
"#).unwrap();
    let compiler = Command::new("cc")
        .current_dir(&dir)
        .args(["-O2", "driver.c", "sib.o", "-o", "driver"])
        .output()
        .unwrap();
    assert!(
        compiler.status.success(),
        "{}",
        String::from_utf8_lossy(&compiler.stderr)
    );
    let run = Command::new(dir.join("driver")).output().unwrap();
    assert!(
        run.status.success(),
        "driver {:?}, stdout={}, stderr={}; evidence={}",
        run.status,
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
        dir.display()
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("PASS"));
    std::fs::remove_dir_all(dir).unwrap();
}

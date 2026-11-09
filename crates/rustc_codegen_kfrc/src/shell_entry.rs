//! The platform entry point. This follows rustc's `main -> lang_start`
//! contract, including runtime initialization and `Termination::report`.

use kfrc_x86::{FunctionCode, MachineFunction, MachineInstr, Reg, SymbolBinding};
use rustc_hir::LangItem;
use rustc_middle::ty::{self, Instance, TyCtxt};
use rustc_session::config::EntryFnType;
use rustc_span::DUMMY_SP;

use kfrc_body::{BodyBuilder, ConstantKind, DataObject, FunctionAbi, PassMode, RecordedTypes};
use rustc_middle::mir::{self, interpret::AllocId};
use rustc_middle::ty::{Ty, TypingEnv};
use rustc_span::def_id::DefId;

/// The PoC implements the compiler-generated unit-main adapter in the shell.
/// The process runtime itself remains the real `std::rt::lang_start_internal`.
/// Vtable bytes and method symbols are rustc's recorded answers, rather than
/// a second, hard-coded trait-object layout.
pub(crate) struct UnitEntryPlan<'tcx> {
    start: Instance<'tcx>,
    internal: Instance<'tcx>,
    closure: DefId,
    vtable: AllocId,
}

impl<'tcx> UnitEntryPlan<'tcx> {
    pub(crate) fn new(tcx: TyCtxt<'tcx>) -> Option<Self> {
        let (main_def_id, _) = tcx.entry_fn(())?;
        let main_ret_ty = tcx.fn_sig(main_def_id).no_bound_vars().unwrap().output();
        let main_ret_ty = tcx.normalize_erasing_regions(
            TypingEnv::fully_monomorphized(),
            main_ret_ty.no_bound_vars().unwrap(),
        );
        if !main_ret_ty.is_unit() {
            tcx.dcx().fatal("kfrc: the PoC executable adapter currently supports `fn main() -> ()`; other Termination return types are not supported yet");
        }
        let start = Instance::expect_resolve(
            tcx,
            TypingEnv::fully_monomorphized(),
            tcx.require_lang_item(LangItem::Start, DUMMY_SP),
            tcx.mk_args(&[main_ret_ty.into()]),
            DUMMY_SP,
        );
        let body = tcx.instance_mir(start.def);
        let mut closure_ty = None;
        let mut principal = None;
        let mut internal = None;
        for block in body.basic_blocks.iter() {
            for statement in &block.statements {
                if let mir::StatementKind::Assign(assignment) = &statement.kind {
                    match &assignment.1 {
                        mir::Rvalue::Aggregate(kind, _) => {
                            if let mir::AggregateKind::Closure(def_id, args) = &**kind {
                                let ty = Ty::new_closure(tcx, *def_id, args);
                                closure_ty = Some(monomorphize(tcx, start, ty));
                            }
                        }
                        mir::Rvalue::Cast(_, _, target) => {
                            let target = monomorphize(tcx, start, *target);
                            if let ty::Dynamic(predicates, _) = target.peel_refs().kind() {
                                principal = predicates.principal().map(|principal| {
                                    tcx.instantiate_bound_regions_with_erased(principal)
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let mir::TerminatorKind::Call { func, .. } = &block.terminator().kind {
                let ty = monomorphize(tcx, start, func.ty(&body.local_decls, tcx));
                if let ty::FnDef(def_id, args) = *ty.kind()
                    && tcx.item_name(def_id).as_str() == "lang_start_internal"
                {
                    internal = Some(Instance::expect_resolve(
                        tcx,
                        TypingEnv::fully_monomorphized(),
                        def_id,
                        args,
                        DUMMY_SP,
                    ));
                }
            }
        }
        let (Some(closure_ty), Some(principal), Some(internal)) = (closure_ty, principal, internal)
        else {
            tcx.dcx().fatal("kfrc: rustc's unit `lang_start` MIR does not match the supported runtime adapter contract");
        };
        let ty::Closure(closure, _) = *closure_ty.kind() else {
            tcx.dcx()
                .fatal("kfrc: `lang_start` capture is not a closure");
        };
        let layout = tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(closure_ty))
            .unwrap_or_else(|error| {
                tcx.dcx()
                    .fatal(format!("kfrc: unit entry capture layout: {error:?}"))
            });
        if layout.size.bytes() != 8 || layout.align.abi.bytes() > 8 {
            tcx.dcx()
                .fatal("kfrc: unit `lang_start` capture must contain exactly one function pointer");
        }
        let abi = crate::abi::instance_abi(tcx, internal)
            .unwrap_or_else(|error| tcx.dcx().fatal(format!("kfrc: unit runtime ABI: {error}")));
        if abi.args
            != [
                PassMode::Pair,
                PassMode::Direct,
                PassMode::Direct,
                PassMode::Direct,
            ]
            || abi.ret != PassMode::Direct
        {
            tcx.dcx().fatal(format!(
                "kfrc: unsupported lang_start_internal ABI: {abi:?}"
            ));
        }
        let vtable = tcx.vtable_allocation((closure_ty, Some(principal)));
        Some(Self {
            start,
            internal,
            closure,
            vtable,
        })
    }

    pub(crate) fn compile(
        &self,
        tcx: TyCtxt<'tcx>,
        instance: Instance<'tcx>,
    ) -> Option<(MachineFunction, Vec<DataObject>)> {
        if instance == self.start {
            return Some(self.compile_start(tcx));
        }
        let is_closure = instance.def_id() == self.closure
            || instance.args.iter().any(|arg| {
                arg.as_type().is_some_and(|ty| ty.walk().any(|arg| {
                    matches!(arg.as_type().map(|ty| *ty.kind()), Some(ty::Closure(def_id, _)) if def_id == self.closure)
                }))
            });
        if !is_closure {
            return None;
        }
        let symbol = tcx.symbol_name(instance).name.to_owned();
        let abi = tcx
            .fn_abi_of_instance(
                TypingEnv::fully_monomorphized().as_query_input((instance, ty::List::empty())),
            )
            .unwrap_or_else(|error| {
                tcx.dcx()
                    .fatal(format!("kfrc: unit entry closure ABI: {error:?}"))
            });
        let mut function = MachineFunction::new(symbol);
        if abi.ret.layout.ty.is_unit() {
            // The capture is a function pointer, so its drop glue is empty.
            if !matches!(instance.def, ty::InstanceKind::DropGlue(..)) {
                tcx.dcx().fatal(format!(
                    "kfrc: unsupported unit entry closure helper `{instance}`"
                ));
            }
            function.instructions.push(MachineInstr::Ret);
        } else {
            if abi.ret.layout.ty != tcx.types.i32
                || abi.args.is_empty()
                || !matches!(
                    abi.args[0].mode,
                    rustc_target::callconv::PassMode::Direct(_)
                )
                || abi
                    .args
                    .iter()
                    .skip(1)
                    .any(|arg| !matches!(arg.mode, rustc_target::callconv::PassMode::Ignore))
            {
                tcx.dcx().fatal(format!(
                    "kfrc: unsupported unit entry closure helper ABI `{instance}`"
                ));
            }
            match abi.args[0].layout.ty.kind() {
                ty::Ref(..) | ty::RawPtr(..) => {
                    function.instructions.push(MachineInstr::LoadIndirect {
                        dst: Reg::Rax,
                        address: Reg::Rdi,
                        offset: 0,
                        bytes: 8,
                    })
                }
                ty::Closure(def_id, _) if *def_id == self.closure => {
                    function.instructions.push(MachineInstr::Mov {
                        dst: Reg::Rax,
                        src: Reg::Rdi,
                    })
                }
                _ => tcx.dcx().fatal(format!(
                    "kfrc: unsupported receiver of unit entry helper `{instance}`"
                )),
            }
            function.instructions.extend([
                MachineInstr::CallIndirect { target: Reg::Rax },
                MachineInstr::MovImm {
                    dst: Reg::Rax,
                    value: 0,
                },
                MachineInstr::Ret,
            ]);
        }
        Some((function, Vec::new()))
    }

    fn compile_start(&self, tcx: TyCtxt<'tcx>) -> (MachineFunction, Vec<DataObject>) {
        let symbol = tcx.symbol_name(self.start).name.to_owned();
        let mut builder = BodyBuilder::new(
            &symbol,
            FunctionAbi {
                args: Vec::new(),
                ret: PassMode::Ignore,
                c_variadic: false,
            },
            RecordedTypes::new(),
        );
        let mut constants = crate::constants::ConstantCx::new(tcx, &symbol);
        let address = constants
            .address(self.vtable, 0, &mut builder)
            .unwrap_or_else(|error| tcx.dcx().fatal(format!("kfrc: unit entry vtable: {error}")));
        let ConstantKind::Address {
            symbol: vtable_symbol,
            addend,
        } = address
        else {
            tcx.dcx()
                .fatal("kfrc: unit entry vtable did not produce a symbol address");
        };
        let mut function = MachineFunction::new(symbol);
        function.stack_slots = 1;
        function.symbols = vec![
            vtable_symbol,
            tcx.symbol_name(self.internal).name.to_owned(),
        ];
        function.instructions = vec![
            MachineInstr::Store {
                slot: kfrc_x86::StackSlot(0),
                src: Reg::Rdi,
            },
            MachineInstr::Mov {
                dst: Reg::R8,
                src: Reg::Rcx,
            },
            MachineInstr::Mov {
                dst: Reg::Rcx,
                src: Reg::Rdx,
            },
            MachineInstr::Mov {
                dst: Reg::Rdx,
                src: Reg::Rsi,
            },
            MachineInstr::SymbolAddress {
                dst: Reg::Rsi,
                symbol: 0,
                addend,
            },
            MachineInstr::StackAddress {
                dst: Reg::Rdi,
                offset: -8,
            },
            MachineInstr::Call { target: 1 },
            MachineInstr::Ret,
        ];
        (function, builder.finish().data_objects)
    }
}

fn monomorphize<'tcx, T>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>, value: T) -> T
where
    T: ty::TypeFoldable<TyCtxt<'tcx>>,
{
    instance.instantiate_mir_and_normalize_erasing_regions(
        tcx,
        TypingEnv::fully_monomorphized(),
        ty::EarlyBinder::bind(value),
    )
}

/// Emit the wrapper beside the Rust entry function, once per executable.
pub(crate) fn entry_wrapper<'tcx>(
    tcx: TyCtxt<'tcx>,
    is_primary_cgu: bool,
    functions: &[FunctionCode],
) -> Option<FunctionCode> {
    let (main_def_id, EntryFnType::Main { sigpipe }) = tcx.entry_fn(())?;
    let main_instance = Instance::mono(tcx, main_def_id);
    let main_symbol = tcx.symbol_name(main_instance).name.to_owned();
    if main_def_id.is_local() {
        if !functions
            .iter()
            .any(|code| code.function.name == main_symbol)
        {
            return None;
        }
    } else if !is_primary_cgu {
        return None;
    }

    let main_ret_ty = tcx.fn_sig(main_def_id).no_bound_vars().unwrap().output();
    let main_ret_ty = tcx.normalize_erasing_regions(
        ty::TypingEnv::fully_monomorphized(),
        main_ret_ty.no_bound_vars().unwrap(),
    );
    let start_def_id = tcx.require_lang_item(LangItem::Start, DUMMY_SP);
    let start_instance = Instance::expect_resolve(
        tcx,
        ty::TypingEnv::fully_monomorphized(),
        start_def_id,
        tcx.mk_args(&[main_ret_ty.into()]),
        DUMMY_SP,
    );
    let start_symbol = tcx.symbol_name(start_instance).name.to_owned();
    let entry_name = tcx.sess.target.options.entry_name.to_string();
    if functions
        .iter()
        .any(|code| code.function.name == entry_name)
    {
        tcx.dcx().fatal(format!(
            "kfrc: entry symbol `{entry_name}` is already defined"
        ));
    }

    // SysV x86-64: C main(argc, argv) arrives in rdi/rsi. Move both
    // arguments before loading the function address into rdi.
    let mut function = MachineFunction::new(entry_name);
    function.symbols = vec![main_symbol, start_symbol];
    function.instructions = vec![
        MachineInstr::Mov {
            dst: Reg::Rdx,
            src: Reg::Rsi,
        },
        MachineInstr::Mov {
            dst: Reg::Rsi,
            src: Reg::Rdi,
        },
        MachineInstr::SymbolAddress {
            dst: Reg::Rdi,
            symbol: 0,
            addend: 0,
        },
        MachineInstr::MovImm {
            dst: Reg::Rcx,
            value: i64::from(sigpipe),
        },
        MachineInstr::Call { target: 1 },
        MachineInstr::Ret,
    ];
    Some(FunctionCode {
        function,
        binding: SymbolBinding::Global { hidden: false },
    })
}

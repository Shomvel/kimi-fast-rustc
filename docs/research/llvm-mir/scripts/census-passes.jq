# 输入：前期 E2B 诊断的 census-*.json（mirx/measure/llvm_pass_census.py 的输出：
#   .cpu[stage][crate] = 进程 CPU 秒；.passes[stage][group][name] = [计时器秒, 指令数]）。
# 参数：$top（每张表的行数）。输出 TSV 文本。
# 口径：opt/llc 默认运行 IR verifier，rustc 默认不跑，这里剔除（VerifierPass、Module Verifier）。
# 同一阶段内 new PM 的 pass 计时与分析计时互斥（嵌套时外层暂停），可以相加；
# llc 的 "Instruction Selection and Scheduling" 与 "Register Allocation" 两组是子分解，不再相加。

def is_verifier: test("^VerifierPass$|^Module Verifier");

def opt_family:
  if test("^(SROAPass|PromotePass)$") then "SROA/mem2reg"
  elif test("^(InstCombinePass|AggressiveInstCombinePass|InstSimplifyPass)$") then "InstCombine 类"
  elif . == "SimplifyCFGPass" then "SimplifyCFG"
  elif test("^(InlinerPass|AlwaysInlinerPass)$") then "内联器"
  elif test("^(GVNPass|EarlyCSEPass)$") then "GVN/EarlyCSE"
  elif test("^(DSEPass|MemCpyOptPass|MergedLoadStoreMotionPass|MoveAutoInitPass)$") then "DSE/MemCpyOpt"
  elif test("Vectoriz|VectorCombine|InjectTLIMappings") then "向量化"
  elif test("Loop|LICM|IndVar|LCSSA") then "循环"
  elif test("^(JumpThreadingPass|CorrelatedValuePropagationPass|ConstraintEliminationPass|SCCPPass)$") then "JT/CVP/约束/SCCP"
  elif test("^(IPSCCPPass|GlobalOptPass|GlobalDCEPass|EliminateAvailableExternallyPass|MergeFunctionsPass|ConstantMergePass|CGProfilePass|RecomputeGlobalsAAPass|DeadArgumentEliminationPass|CalledValuePropagationPass|ArgumentPromotionPass|PostOrderFunctionAttrsPass|ReversePostOrderFunctionAttrsPass|InferFunctionAttrsPass|PGOIndirectCallPromotion|RelLookupTableConverterPass)$") then "过程间（IPSCCP/GlobalOpt/属性/参数）"
  elif test("^(BDCEPass|ADCEPass|ReassociatePass|TailCallElimPass|DivRemPairsPass|Float2IntPass|SpeculativeExecutionPass|CallSiteSplittingPass|LibCallsShrinkWrapPass|ControlHeightReductionPass|AlignmentFromAssumptionsPass|InferAlignmentPass|LowerConstantIntrinsicsPass|LowerExpectIntrinsicPass)$") then "其他标量"
  else "其他（PM 胶水、Coro/OpenMP 空转等）" end;

def analysis_family:
  if test("MemorySSA") then "分析：MemorySSA"
  elif test("AA|Alias") then "分析：别名"
  elif test("BranchProbability|BlockFrequency") then "分析：BPI/BFI"
  elif test("DominatorTree|PostDominatorTree|LoopAnalysis") then "分析：支配树/循环"
  else "分析：其他" end;

def cg_family:
  if test("Assembly Printer|Prologue/Epilogue|Stack Frame Layout|StackMap|Insert CFI|Branch relaxation") then "发射/帧"
  elif test("Instruction Selection|DAG->DAG|FastISel|IRTranslator|Legalizer|RegBankSelect|InstructionSelect") then "指令选择"
  elif test("DEBUG_VALUE|Debug Variable|Debug Value|LiveDebug") then "调试信息"
  elif test("Register Allocator|Virtual Register Rewriter|Register Coalescer|Live Interval|Live Variable Analysis|Live Range Shrink|Two-Address|Eliminate PHI|Slot index|Live Stack|Spill|Rename Disconnected|Greedy|Fast Register") then "寄存器分配与活跃性"
  elif test("[Ss]cheduler") then "指令调度"
  elif test("Loop Strength Reduction|CodeGen Prepare|Canonicalize Freeze|Constant Hoisting|Induction Variable Users|Expand|Lower|Interleaved|Partially inline|Merge contiguous|Scalarize|Merge internal globals|Exception handling") then "IR 级后端准备（LSR/CGP…）"
  elif test("Analysis|Dominator|Natural Loop|Scalar Evolution|Block Frequency|Branch Probability") then "后端分析"
  else "机器级优化（CSE/LICM/sink/窥孔/布局…）" end;

def pct($x; $t): (if $t > 0 then ($x * 1000 / $t | round) / 10 else 0 end);
def r2: (. * 100 | round) / 100;

def entries($s):
  if $s == "codegen" then
    (.passes.codegen["Pass execution timing report"] // {}) | to_entries
    | map(select(.key | is_verifier | not) | {name: .key, t: .value[0], fam: (.key | cg_family), stage: $s})
  else
    ((.passes[$s]["Pass execution timing report"] // {}) | to_entries
      | map(select(.key | is_verifier | not) | {name: .key, t: .value[0], fam: (.key | opt_family), stage: $s}))
    + ((.passes[$s]["Analysis execution timing report"] // {}) | to_entries
      | map({name: .key, t: .value[0], fam: (.key | analysis_family), stage: $s}))
  end;

. as $root
| ["prelink", "postlink", "codegen"] as $stages
| ($stages | map(. as $s | $root | entries($s)) | add) as $all
| ($all | map(.t) | add) as $total
| ($stages | map(. as $s | {key: $s, value: ($all | map(select(.stage == $s) | .t) | add)}) | from_entries) as $st
| "# input crates: \(.crates)  units: \(.units)  skip_policy: \(.skip_policy | if . == "" then "none" else . end)",
  "",
  "## stage\tprocess_cpu_s\ttimer_s(no verifier)\tshare_of_timer_total_%\tverifier_excluded_s",
  ($stages[] as $s
   | "\($s)\t\($root.cpu[$s] | add | r2)\t\($st[$s] | r2)\t\(pct($st[$s]; $total))\t\(
       ($root.passes[$s] | to_entries | map(.value | to_entries[] | select(.key | is_verifier) | .value[0]) | add // 0) | r2)"),
  "total\t\($root.cpu | map(add) | add | r2)\t\($total | r2)\t100\t",
  "",
  "## family\tprelink_s\tpostlink_s\tcodegen_s\tsum_s\tshare_of_LLVM_%",
  ($all | group_by(.fam) | map({fam: .[0].fam,
      pre: (map(select(.stage == "prelink") | .t) | add // 0),
      post: (map(select(.stage == "postlink") | .t) | add // 0),
      cg: (map(select(.stage == "codegen") | .t) | add // 0)})
   | map(. + {sum: (.pre + .post + .cg)}) | sort_by(-.sum)[]
   | "\(.fam)\t\(.pre | r2)\t\(.post | r2)\t\(.cg | r2)\t\(.sum | r2)\t\(pct(.sum; $total))"),
  "",
  "## top passes (pre+post merged by name; codegen separate)\tprelink_s\tpostlink_s\tcodegen_s\tshare_of_LLVM_%",
  ($all | group_by(.name) | map({name: .[0].name,
      pre: (map(select(.stage == "prelink") | .t) | add // 0),
      post: (map(select(.stage == "postlink") | .t) | add // 0),
      cg: (map(select(.stage == "codegen") | .t) | add // 0)})
   | map(. + {sum: (.pre + .post + .cg)}) | sort_by(-.sum) | .[0:$top][]
   | "\(.name)\t\(.pre | r2)\t\(.post | r2)\t\(.cg | r2)\t\(pct(.sum; $total))"),
  "",
  "## codegen family\tcodegen_s\tshare_of_codegen_%",
  ($all | map(select(.stage == "codegen")) | group_by(.fam)
   | map({fam: .[0].fam, t: (map(.t) | add)}) | sort_by(-.t)[]
   | "\(.fam)\t\(.t | r2)\t\(pct(.t; $st.codegen))")

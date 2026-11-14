# Residual inline cost

Fable A2/C3 identifies an architectural mismatch: publishing one final callee
body can increase semantic node count while reducing its generated code. A
64-node profitability limit can consequently reject an eight-instruction helper.
The R3/R4 Callgrind differential demonstrates this for base64 `read_u64`, reverse
character iteration, and slice-reference equality in humantime unit parsing.

`InlineCostPolicy::Residual` compares the prepared call-site source with the ABI
sequence it replaces. The source is the same immutable body used for emission;
literal scalar and selected aggregate-field arguments first go through the
existing bounded specialization and simplification machinery. A source whose
estimated cost is no larger than the removed call is free. Other calls use one
threshold on their estimated positive growth. A sole local reference and strong
inline intent retain their existing profitability exceptions.

Free ordinary splices do not consume costed-call or depth allowances. They still
pay every cloned and newly allocated node, and retain ABI, recursion, symbol,
data identity and type checks. Disposable specialization retains its separate
finite depth and work accounts. Positive-growth calls retain ordinary call,
depth and allocation limits. Unknown source or call costs never become free.

`ResidualCostModel` is a versioned 64-bit relative operation model. Semantic
packaging, Phi/projection nodes, constants, requirements, entry and return
boundaries have no independent cost; operations and materialization consumers
carry the cost. Ordinary scalar operations and branches cost one, memory
snapshots scale with their recorded size, and nested calls include the call
instruction plus their physical ABI lanes and indirect argument snapshots.
Unmodeled operations/layouts stay unknown. Call and copy runtime work remains
unknown rather than assigning a fabricated fixed execution time.

This is an estimate before instruction selection. It does not predict encoded
bytes, execution frequency, register pressure or every addressing-mode fold.
Byte packing, aggregate traffic, and Phi moves can still make an estimate
optimistic; location forwarding can make it pessimistic. Correctness and work
limits are independent of the estimate. Runtime instruction counts, code size,
compile instructions and monotonicity must calibrate its threshold.

The rustc backend defaults to `callsite` residual pricing. Ordinary, cold and
unknown sites use a positive-growth threshold of 32; cyclic S control sites use
256. An immutable CFG snapshot classifies cyclic SCCs and propagates coldness
backward from Unreachable only when every successor is cold. Nested calls keep
source/copy correspondence: a cold source path stays cold inside a hot caller;
only classified non-cold paths may inherit the hot allowance. This is static
profitability, not a trip-count or semantic fact. Classification uses a bounded
weighted work account, cached by source/specialization, and falls back to 32 on
exhaustion. Hot emission sites precede ordinary sites within strong inline
intent. Scratch constant-argument preparation keeps the context-independent 32
threshold and the same shared cache/work accounts.

The current candidate also gives each emission invocation a shared account of
512 extra growth units. Its effective hot threshold is the smaller of 256 and
32 plus the remaining account. After a successful non-exempt hot splice, the
account pays only the source estimate minus the removed call and the ordinary
32-unit allowance. It never refunds growth after cleanup. Nested splices use
the same account; they cannot reset it by inheriting hotness. Rejected plans,
ordinary-sized/free splices, sole local references and strong inline intent
do not consume this account. The latter two retain their existing profitability
exceptions and still pay all hard graph/depth/work limits. Exhaustion restores
ordinary pricing; it does not stop the inliner. This is an estimated growth
account, not a measured code-size or compiler-instruction limit.

Explicit `KFRC_INLINE_COST_MODEL=residual` retains uniform 32. Every explicit
`KFRC_INLINE_RESIDUAL_UNITS` still selects exactly that uniform threshold,
preserving historical calibration and ablation commands.
The uniform 32 policy retains the measured 0/8/16/32 calibration arm. The
call-site candidate changes no graph allocation, recursion, depth, costed-call
or preparation-work limits and needs its own runtime/compile/size acceptance.
`KFRC_INLINE_COST_MODEL=legacy|observe|size` retains the established alternatives;
`observe` follows legacy decisions while recording the seed estimates. The
standalone Son API retains its explicitly selected policy.
The cost diagnostic records the residual model, source estimate, call-specific
free threshold, effective site threshold, static temperature and exact rejection
reason, together with the extra hot growth remaining before the decision and
actually charged on success. Interpreter
differentials cover source requirements, free wrapper chains, literal pruning,
poison observations and positive-growth/work limits. Calibration and final
default verification are recorded separately from these mechanism tests.

`KFRC_CGSCC_STATS=1`, `KFRC_SPECIALIZATION_STATS=1` or `KFRC_DUMP_GRAPH=1`
enables cost, preparation and seam diagnostics. `observe` always emits them.
Ordinary compilation does not format or print these records; reporting does
not change the estimates, decisions or work accounts.

CGSCC's library entry reads process configuration, then passes an explicit
policy and limits to its implementation. Tests bind their intended policy
directly: legacy shape/depth fixtures do not inherit a calibration arm's process
environment. A separate residual threshold differential checks a 32-operation
body rejected at growth 0/16 and accepted at 40, the shared source used for
emission, and defined/poison behavior. This keeps profitability-specific
expectations distinct from semantic and publication contracts.

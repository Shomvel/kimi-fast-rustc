# Interleave inlining and scalar simplification

The crate-wide provider exposes one prepared callee body to its callers. After
each ordinary splice, the inliner now simplifies the changed scalar seam before
examining the next call. A returned expression can consequently become a
literal argument to the next call's existing specialization machinery. Previously
ordinary splices accumulated until the outer pipeline's batch cleanup.

`transforms/scalar/seam::Worklist` owns a reverse operand index and the existing
field expression index for one inliner invocation. It indexes the initial caller
once, then indexes appended nodes and repairs existing users named by each
splice. Scalar replacements enqueue their users and newly created nodes. The
ordinary batch seam API uses the same worklist implementation with a shorter
lifetime.

The reverse index is conservative: obsolete edges may enqueue extra candidates.
Every rewrite reads current, resolved operands and revalidates field expression
representatives. The index cannot establish exact use counts, uniqueness, or
absence of uses. It stores operand edges; fixed control/effect behavior remains
owned by the graph and the control cleanup.

When a changed condition becomes a literal or a known enum variant, the existing
control simplifier removes unreachable continuations and repairs Region/Phi
predecessors before another call is examined. A divergent splice follows the
same path. This mandatory pruning creates no graph records; optional diamond
construction remains in the outer pipeline. Arbitrary CFG edits invalidate the worklist, so the next splice builds
a fresh index. Node IDs remain stable throughout this process. Compaction still
belongs to pipeline finalization.

Disposable call-site preparation retains its existing fact reapplication,
constant propagation and control cleanup, with its existing work accounts. This
change does not widen the node, helper, depth or data import budgets and does not
change the default profitability policy. It is one part of Fable A2/A4: memory
SSA, shared S/L optimization, and pricing by residual machine work remain separate
work.

Scalar rule allocations consume the same nonrefundable inliner growth account
as cloned nodes. Each local rule may create at most two records (a constant and
its consumer), checked as a rule contract. With less space remaining, scalar
rewriting waits for the outer pipeline; branch pruning can still run. This
preserves hard growth limits even when folding temporarily creates dead nodes.

With `KFRC_CGSCC_STATS=1` (or the existing specialization/graph diagnostics),
`kfrc-inline-seams` reports indexed nodes, scalar visits, control cleanups and
allocation bailouts.
These indexing and visit costs also contribute to the CGSCC work account.
Interpreter differentials cover constants crossing consecutive calls, repeated
splices sharing one index, defined execution and poison observations. The full
semantic inlining suite covers effects, lifetimes, divergent paths and CFG joins.
The real workload gate must still establish profitability independently.

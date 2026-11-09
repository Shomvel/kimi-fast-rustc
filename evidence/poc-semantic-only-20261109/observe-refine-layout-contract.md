# S→L proof and representation boundary proposal

This is a static design memo, not a new IR implementation. It supersedes the
next-snapshot work as the active layout task. No compilation was run for it.

## Why the current path manufactures executable work

1. lower_layout::run visits ObserveValue before ordinary semantic projections.
   lower_observation calls expand(input), so a whole enum observation demands
   decode_boundary/decode_fields or phi_tree before knowing which data is used.
2. observe_parts emits per-variant comparisons, If, Region, effect Phi, then
   fixed scalar ObserveValue nodes. Nested enums recursively multiply this work.
   encoded_demands also treats an ObserveValue use as a reason to reject tag-only
   representation, so a domain requirement changes the storage representation.
3. refine_value first expands its source. Even after candidate09 narrows a known
   outer variant, pin_tree traverses the whole active payload tree, including
   dynamically nested enums. ControlOnly Refine becomes an anchored runtime
   identity, and x86 emits a Mov for every such identity.
4. lower_transmute independently calls observe_parts before encoding and after
   MaybeUninit→T. Removing only ordinary ObserveValue lowering misses these paths.
5. Current x86 scheduling roots all Refine and ObserveValue nodes. Even an unused
   observer forces its input materialization before no-code emission is considered.

## Explicit semantic boundary

S is the authority for definedness. ObserveValue/Assumed fact failures remain UB
in S and are tested there. For every input/execution on which S is defined, L must
preserve results, effects, memory and required control behavior. L need not
reproduce S's diagnostic on a source-UB execution. This is the ordinary refinement
contract, not authorization to weaken S's validity predicate or move a requirement
past a source operation. Runtime checks/panics are separate effectful nodes and
are never erased as observations. Opaque is also not an observation synonym.

If the project instead requires L to diagnose every source-UB case, it must keep
an explicit non-production oracle sidecar with the S program; requesting that
and zero runtime observation at once cannot be achieved by scalarizing checks.
Do not retain full payload computation in production L merely for differential
negative tests. Tests must first classify S-definedness, compare on defined runs,
and separately assert the S error class for invalid runs.

## Two independent demands

Representation demand: values/bytes actually consumed by arithmetic, control,
loads/stores, transmute representation, calls, returns, and externally meaningful
operations. This is the only demand that expands a value into scalar leaves or
owned storage.

Proof demand: a fact about an exact immutable semantic value at an exact control
scope. It licenses a use, does not clone/refine that value's whole data tree, and
does not by itself request fields, payload bytes, or ABI storage. A required
runtime VariantSwitch still demands a tag; its witness does not demand payload.
An Assumed variant fact is a source-domain obligation, not an extra runtime tag
comparison introduced by L.

## Proposed VariantWitness and projection relationship

Conceptually, VariantWitness(value, proof_control; variant, origin) has a proof
result, not Rust(enum) result. VariantField(value, witness; variant, field) takes
the ORIGINAL value and a witness tied to that exact resolved value/variant.
The verifier proves witness control dominates this projection/use; a witness for
another enum, another value, sibling branch or a different incoming Phi is invalid.
Constructed-variant evidence can be checked without a runtime switch. Nested enum
fields need their own evidence and never inherit the outer variant number.

A pure speculative VariantField without witness may retain its existing poison
semantics. Do not turn it into an Assumed witness, because that would introduce
UB where unused poison was previously permitted. The exact arity/opcode split
belongs to the common graph contract owner.

S interpreter validates Proven evidence and checks Assumed facts at their original
anchor without observing inactive payloads. Graph edits/import/inlining must remap
proof value identity and control together. Facts cannot escape through a merge
unless a separately checked incoming witness construction exists.

## Minimal production lowering migration

A. On a verified S candidate, erase semantic-only ObserveValue BEFORE
encoded_demands/tag-only-load analysis or any expand call. Rewire projection 0 to
its input control, 1 to input effect, and 2 to its original value. Remove it and
compact before representation demand analysis. Calls, memory and runtime checks
remain roots. Erase the transmute-generated observation-only work as part of the
same seam rule; actual representation conversion and raw MaybeUninit copying stay.

B. Consume witnesses during lowering of actual VariantField uses. Expand the
original semantic value once, select the demanded field, and carry only that
use's placement constraint. Never run pin_tree because a witness exists. Start
with existing scalar ControlOnly anchors for actually demanded extracted leaves
if necessary; this is a bounded transition, not an aggregate Refine clone. A
non-material proof-only use emits no anchor, load, Phi, slot, or machine register.

C. Keep placement separate from data identity. Two field uses under different
witness scopes cannot globally replace the source scalar with one stronger value.
Attach the lower-bound control to each lowered use (or a zero-data alias scoped
to that use); preserve original fixed controls of loads/divisions/calls. Check
that fixed and potentially trapping operations do not become eager on bypass
edges. A scalar alias may later share its source machine register, but that is a
separate scheduler/liveness change, not permission to discard its control bound.

D. All representation-demanded aggregate reads remain owned snapshots at the
original Start/call/load control+effect. Removing proof demand does not make an
ABI pointer an immutable Rust value. Mutable source addresses must not escape as
Parts::Scalar for an aggregate, and reads must not be deferred across alias writes.
Do not change private outgoing argument materialization or return placement here.

E. Final L verifier rejects semantic observers/variant witnesses/aggregate Refine
and checks surviving placement constraints. L assembly must contain no branch,
load or move whose only root was an erased proof/domain requirement. Keep counts
for representation-generated tag guards separately from source runtime branches.

## Ownership for the next implementation agreement

- Contract owner: proof result type, witness identity/arity, verifier dominance,
  S interpreter domain checks, negative-test classification and import schema.
- Builder/optimization owners: producers of Proven/Assumed witness, CFG edits,
  dead proof handling, inlining and observation obligations before the seam.
- Layout owner: seam erasure before analysis, projection/witness demand lowering,
  removal of whole-tree pinning, transmute seam behavior, owned snapshot invariants.
- x86 owner (layout agent): zero runtime handling after constraints are resolved;
  preserve schedule dependency order and machine liveness. No blind global alias
  replacement for scoped values.
- Validation owner: defined-run S/L/native differential, S-only UB classification,
  aliasing snapshot tests, nested/sibling/Phi witness rejection, real semver/W01
  correctness and frozen stage/work counters before performance claims.

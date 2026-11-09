//! Stable semantic identities for nominal types and statically resolved items.
//! Display paths are diagnostics, never interner keys by themselves: nested
//! generic arguments can contain different definitions with identical names.
use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::stable_hasher::{HashStable, StableHasher};
use rustc_middle::ty::{Ty, TyCtxt};

/// Hash the complete Rust type (including nested arguments and DefPathHashes).
/// Regions are erased/anonymized exactly as rustc does for type identity; the
/// hashing context excludes source spans and process-local interner addresses.
/// A function item type carries both its DefId and complete GenericArgs, so the
/// same helper also keys R Resolve questions without a second identity scheme.
pub(crate) fn name<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> String {
    let canonical = tcx.erase_and_anonymize_regions(ty);
    let fingerprint: Fingerprint = tcx.with_stable_hashing_context(|mut context| {
        let mut hasher = StableHasher::new();
        context.while_hashing_spans(false, |context| canonical.hash_stable(context, &mut hasher));
        hasher.finish()
    });
    format!("{canonical}#{fingerprint}")
}

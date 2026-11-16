use super::*;

#[test]
fn full_and_seam_cleanup_reprove_observations_after_phi_expansion() {
    for seam in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
        f.fixed(Opcode::RequireDefined, f.a, false);
        f.fixed(Opcode::RequireDefined, f.next_a, false);
        f.verify();
        let before = f.graph.clone();
        let stats = if seam {
            crate::optimize_specialized_seams(&mut f.graph, &[f.next_iv], &[]).unwrap()
        } else {
            crate::optimize_specialized_graph(&mut f.graph).unwrap()
        };
        assert_eq!(stats.loops_unrolled, 1);
        assert!(stats.observations_removed >= 8);
        assert!(f.graph.ids().all(|node| !matches!(
            f.graph.op(node),
            Opcode::RequireDefined | Opcode::If | Opcode::Phi
        )));
        f.verify();
        equivalent(&before, &f.graph, u32::MAX.into());
    }
}

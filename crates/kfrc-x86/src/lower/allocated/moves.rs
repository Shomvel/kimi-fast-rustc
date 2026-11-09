//! Parallel location copies with an independently reserved register or stack temporary.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Location {
    Reg(Reg),
    Stack(StackSlot),
    Imm(i64),
}
impl Location {
    pub fn register(self) -> Option<Reg> {
        if let Self::Reg(r) = self {
            Some(r)
        } else {
            None
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Copy {
    pub src: Location,
    pub dst: Location,
}

pub(super) fn resolve(
    mut pending: Vec<Copy>,
    temporary: Location,
) -> Result<Vec<Copy>, LowerError> {
    for (index, copy) in pending.iter().enumerate() {
        if matches!(copy.dst, Location::Imm(_))
            || pending[..index].iter().any(|c| c.dst == copy.dst)
        {
            return Err(LowerError::Malformed(
                "parallel copies have duplicate or immediate destination".into(),
            ));
        }
    }
    if matches!(temporary, Location::Imm(_))
        || pending
            .iter()
            .any(|copy| copy.src == temporary || copy.dst == temporary)
    {
        return Err(LowerError::Malformed(
            "parallel copy temporary aliases an operand".into(),
        ));
    }
    pending.retain(|c| c.src != c.dst);
    let mut result = Vec::new();
    while !pending.is_empty() {
        if let Some(index) = pending
            .iter()
            .position(|copy| pending.iter().all(|other| other.src != copy.dst))
        {
            result.push(pending.remove(index));
        } else {
            let source = pending[0].src;
            let saved = temporary;
            result.push(Copy {
                src: source,
                dst: saved,
            });
            for copy in &mut pending {
                if copy.src == source {
                    copy.src = saved;
                }
            }
        }
    }
    Ok(result)
}

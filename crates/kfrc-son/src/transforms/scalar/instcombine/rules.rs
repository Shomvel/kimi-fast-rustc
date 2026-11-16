//! Stable switches and opt-in attribution for independently measured rules.
use std::sync::OnceLock;

use super::{FunctionGraph, NodeId};

#[derive(Clone, Copy)]
pub(super) enum Rule {
    MulPowerOfTwo,
    TruncBinop,
    OppositeShifts,
    MulShiftFactor,
    MemcmpZero,
}

impl Rule {
    fn id(self) -> &'static str {
        match self {
            Self::MulPowerOfTwo => "mul-pow2",
            Self::TruncBinop => "trunc-binop",
            Self::OppositeShifts => "opposite-shifts",
            Self::MulShiftFactor => "mul-shift-factor",
            Self::MemcmpZero => "memcmp-zero",
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

const ALL: [Rule; 5] = [
    Rule::MulPowerOfTwo,
    Rule::TruncBinop,
    Rule::OppositeShifts,
    Rule::MulShiftFactor,
    Rule::MemcmpZero,
];

struct Options {
    disabled: u8,
    stats: bool,
}

fn options() -> &'static Options {
    static OPTIONS: OnceLock<Options> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        let mut disabled = 0;
        for id in std::env::var("KFRC_INSTCOMBINE_DISABLE")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
        {
            if id == "all" {
                disabled = u8::MAX;
            } else if let Some(rule) = ALL.into_iter().find(|rule| rule.id() == id) {
                disabled |= rule.bit();
            }
        }
        Options {
            disabled,
            stats: std::env::var_os("KFRC_INSTCOMBINE_STATS").is_some_and(|value| value == "1"),
        }
    })
}

pub(super) fn enabled(rule: Rule) -> bool {
    options().disabled & rule.bit() == 0
}

pub(super) fn any_enabled() -> bool {
    ALL.into_iter().any(enabled)
}

pub(super) fn record(rule: Rule, graph: &FunctionGraph, node: NodeId) {
    if options().stats {
        eprintln!(
            "[kfrc-instcombine] rule={} level={:?} function={} node={}",
            rule.id(),
            graph.level,
            graph.symbol,
            node,
        );
    }
}

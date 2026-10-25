// `cfg_select!` is itself a host-selection macro; any invocation outside
// the boundary is denied regardless of its arms.
use std::cfg_select;

cfg_select! {
    unix => {
        fn f() -> u8 { 1 }
    },
    windows => {
        fn f() -> u8 { 2 }
    },
}

fn main() {}
